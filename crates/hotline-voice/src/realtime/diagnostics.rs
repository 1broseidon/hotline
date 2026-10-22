//! Refusals retain request identity, never request bodies or credentials.
use serde_json::Value;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Requests(VecDeque<(String, String)>);

impl Requests {
    pub(super) fn stamp(&mut self, event: &mut Value) {
        let id = uuid::Uuid::new_v4().to_string();
        let kind = event["type"].as_str().unwrap_or("unknown").to_string();
        event["event_id"] = Value::String(id.clone());
        self.0.push_back((id, kind));
        while self.0.len() > 256 {
            self.0.pop_front();
        }
    }

    pub(super) fn refusal(&self, event: &Value, secrets: &[&str]) -> String {
        let error = &event["error"];
        let id = error["event_id"].as_str();
        let kind = self
            .0
            .iter()
            .find(|(known, _)| Some(known.as_str()) == id)
            .map_or(
                "unknown (provider supplied no matching event_id)",
                |(_, kind)| kind.as_str(),
            );
        let detail = format!(
            "event={kind} code={} param={} message={}",
            error["code"].as_str().unwrap_or("unknown"),
            error["param"].as_str().unwrap_or("none"),
            error["message"].as_str().unwrap_or("No provider message.")
        );
        sanitize(&detail, secrets)
    }
}

/// Only turn boundaries are traced: no audio, transcript, tool arguments,
/// provider identifiers or arbitrary event fields can enter these logs.
pub(super) fn turn_boundary(event: &Value) -> Option<String> {
    let kind = event["type"].as_str()?;
    match kind {
        "input_audio_buffer.speech_started"
        | "input_audio_buffer.speech_stopped"
        | "input_audio_buffer.commit"
        | "input_audio_buffer.committed"
        | "input_audio_buffer.cleared"
        | "conversation.item.input_audio_transcription.completed"
        | "conversation.item.input_audio_transcription.failed"
        | "conversation.item.create"
        | "response.function_call_arguments.done"
        | "conversation.item.truncate"
        | "conversation.item.truncated"
        | "response.cancel"
        | "response.create"
        | "response.created"
        | "response.done" => {}
        _ => return None,
    }
    let status = match event["response"]["status"].as_str() {
        Some("completed") => "completed",
        Some("cancelled") => "cancelled",
        Some("failed") => "failed",
        Some("incomplete") => "incomplete",
        _ => "none",
    };
    Some(format!("event={kind} status={status}"))
}

pub(super) fn sanitize(text: &str, secrets: &[&str]) -> String {
    let mut safe = text.to_string();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        safe = safe.replace(secret, "[redacted]");
    }
    let safe = hotline_room::secret::redact_text(&safe);
    let safe: String = safe
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    hotline_room::text::clip(&safe, 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn turn_traces_include_boundaries_but_never_provider_content() {
        let event = json!({"type":"response.done", "response":{
            "status":"cancelled", "output":"private words"},
            "item_id":"private-id", "audio":"private-audio"});
        assert_eq!(
            turn_boundary(&event).unwrap(),
            "event=response.done status=cancelled"
        );
        assert_eq!(
            turn_boundary(&json!({"type":"response.done",
            "response":{"status":"secret"}}))
            .unwrap(),
            "event=response.done status=none"
        );
        assert!(
            turn_boundary(&json!({"type":"response.output_audio.delta", "delta":"audio"}))
                .is_none()
        );
        assert!(turn_boundary(&json!({"type":"secret"})).is_none());
    }

    #[test]
    fn a_refusal_names_its_request_without_logging_credentials_or_bodies() {
        let mut requests = Requests::default();
        let mut sent = json!({"type":"response.create", "body":"private conversation"});
        requests.stamp(&mut sent);
        let event = json!({"error":{"event_id":sent["event_id"], "code":"invalid_request_error",
            "message":"Bad field; Bearer opaque-token sk-secret credential-value\nforged log"}});
        let log = requests.refusal(&event, &["credential-value"]);
        assert!(log.contains("event=response.create"));
        assert!(log.contains("Bad field"));
        for secret in [
            "opaque-token",
            "sk-secret",
            "credential-value",
            "private conversation",
            "\n",
        ] {
            assert!(!log.contains(secret), "{log}");
        }
    }
}
