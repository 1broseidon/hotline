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
