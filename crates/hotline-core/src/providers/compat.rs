//! The transport under custom OpenAI-compatible connections. Compatible
//! servers speak the Responses API more loosely than OpenAI: an output item
//! without an id, `created_at` as a float, `instructions` echoed back as input
//! items, an empty `error` object. Each of those makes Rig's strict decode drop
//! the whole turn, so this transport repairs Responses frames to the shape Rig
//! reads before Rig sees them. Chat Completions traffic passes through as is.

use bytes::Bytes;
use futures_util::StreamExt;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, StreamingResponse, sse::BoxedStream,
};
use rig::providers::openai::responses_api::streaming::StreamingCompletionChunk;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashSet};
use std::sync::{Mutex, OnceLock, PoisonError};

#[derive(Clone, Debug, Default)]
pub(crate) struct Http {
    inner: reqwest::Client,
}

impl Http {
    pub(crate) fn new(inner: reqwest::Client) -> Self {
        Self { inner }
    }
}

fn is_responses<T>(request: &http::Request<T>) -> bool {
    request.uri().path().ends_with("/responses")
}

fn host<T>(request: &http::Request<T>) -> String {
    request.uri().host().unwrap_or("custom server").to_string()
}

impl HttpClientExt for Http {
    fn send<T, U>(
        &self,
        request: http::Request<T>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let repair = is_responses(&request).then(|| host(&request));
        let sent = HttpClientExt::send::<T, Bytes>(&self.inner, request);
        async move {
            let response = sent.await?;
            let (parts, body) = response.into_parts();
            let body: LazyBody<U> = Box::pin(async move {
                let bytes = body.await?;
                Ok(U::from(match repair {
                    Some(host) => repaired_body(bytes, &host),
                    None => bytes,
                }))
            });
            Ok(http::Response::from_parts(parts, body))
        }
    }

    fn send_multipart<U>(
        &self,
        request: http::Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<http::Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        HttpClientExt::send_multipart(&self.inner, request)
    }

    async fn send_streaming<T>(
        &self,
        request: http::Request<T>,
    ) -> http_client::Result<StreamingResponse>
    where
        T: Into<Bytes> + Send,
    {
        let repair = is_responses(&request).then(|| host(&request));
        let response = HttpClientExt::send_streaming(&self.inner, request).await?;
        Ok(match repair {
            Some(host) => response.map(|stream| repaired_stream(stream, host)),
            None => response,
        })
    }
}

/// A unary Responses body: the response object itself.
fn repaired_body(bytes: Bytes, host: &str) -> Bytes {
    let Ok(Value::Object(mut response)) = serde_json::from_slice::<Value>(&bytes) else {
        return bytes;
    };
    let mut fixed = BTreeSet::new();
    repair_response(&mut response, &mut fixed);
    if fixed.is_empty() {
        return bytes;
    }
    note_repairs(host, &fixed);
    serde_json::to_vec(&response).map_or(bytes, Bytes::from)
}

struct Stream {
    inner: BoxedStream,
    pending: Vec<u8>,
    frames: Frames,
    ended: bool,
}

/// Re-chunk the stream on line boundaries so every `data:` line is whole
/// when it is repaired; a frame split across chunks is held until it ends.
fn repaired_stream(inner: BoxedStream, host: String) -> BoxedStream {
    let state = Stream {
        inner,
        pending: Vec::new(),
        frames: Frames { host, next: 0 },
        ended: false,
    };
    Box::pin(futures_util::stream::unfold(
        state,
        |mut state| async move {
            loop {
                if state.ended {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(bytes)) => {
                        state.pending.extend_from_slice(&bytes);
                        let Some(cut) = state.pending.iter().rposition(|byte| *byte == b'\n')
                        else {
                            continue;
                        };
                        let rest = state.pending.split_off(cut + 1);
                        let lines = std::mem::replace(&mut state.pending, rest);
                        let out = state.frames.lines(&lines);
                        return Some((Ok(Bytes::from(out)), state));
                    }
                    Some(Err(error)) => return Some((Err(error), state)),
                    None => {
                        state.ended = true;
                        if state.pending.is_empty() {
                            return None;
                        }
                        let lines = std::mem::take(&mut state.pending);
                        let out = state.frames.lines(&lines);
                        return Some((Ok(Bytes::from(out)), state));
                    }
                }
            }
        },
    ))
}

struct Frames {
    host: String,
    /// The sequence number the next frame should carry, for servers that
    /// leave it out.
    next: u64,
}

impl Frames {
    fn lines(&mut self, text: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        for line in text.split_inclusive(|byte| *byte == b'\n') {
            out.extend_from_slice(&self.line(line).unwrap_or_else(|| line.to_vec()));
        }
        out
    }

    /// The repaired line, or `None` to pass the original through.
    fn line(&mut self, line: &[u8]) -> Option<Vec<u8>> {
        let body = line.strip_suffix(b"\n").unwrap_or(line);
        let body = body.strip_suffix(b"\r").unwrap_or(body);
        let ending = &line[body.len()..];
        let data = body.strip_prefix(b"data:")?.trim_ascii();
        let Ok(Value::Object(mut event)) = serde_json::from_slice::<Value>(data) else {
            return None;
        };
        let mut fixed = BTreeSet::new();
        repair_event(&mut event, &mut self.next, &mut fixed);
        let repaired = match fixed.is_empty() {
            true => None,
            false => Some(serde_json::to_vec(&event).ok()?),
        };
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let frame = repaired.as_deref().unwrap_or(data);
        if read_by_rig(kind)
            && let Err(error) = serde_json::from_slice::<StreamingCompletionChunk>(frame)
        {
            note_unreadable(&self.host, kind, &error, frame);
        }
        let repaired = repaired?;
        note_repairs(&self.host, &fixed);
        let mut out = b"data: ".to_vec();
        out.extend_from_slice(&repaired);
        out.extend_from_slice(ending);
        Some(out)
    }
}

/// The event types Rig decodes strictly; anything else it skips as unknown.
fn read_by_rig(kind: &str) -> bool {
    matches!(
        kind,
        "response.created"
            | "response.in_progress"
            | "response.completed"
            | "response.failed"
            | "response.incomplete"
            | "response.output_item.added"
            | "response.output_item.done"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.delta"
            | "response.output_text.done"
            | "response.refusal.delta"
            | "response.refusal.done"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done"
    )
}

pub(crate) fn repair_event(
    event: &mut Map<String, Value>,
    next: &mut u64,
    fixed: &mut BTreeSet<&'static str>,
) {
    if !event.get("type").is_some_and(Value::is_string) {
        return;
    }
    if let Some(number) = event.get("sequence_number").and_then(Value::as_u64) {
        *next = number + 1
    } else {
        event.insert("sequence_number".into(), (*next).into());
        *next += 1;
        fixed.insert("sequence_number");
    }
    let done = event
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.ends_with(".done"));
    let fallback = event
        .get("item_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            event
                .get("output_index")
                .and_then(Value::as_u64)
                .map(|index| format!("item_{index}"))
        });
    if let Some(Value::Object(response)) = event.get_mut("response") {
        repair_response(response, fixed);
    }
    if let Some(Value::Object(item)) = event.get_mut("item") {
        repair_item(item, fallback.as_deref().unwrap_or("item"), done, fixed);
    }
}

fn repair_response(response: &mut Map<String, Value>, fixed: &mut BTreeSet<&'static str>) {
    if !response.contains_key("object") {
        response.insert("object".into(), "response".into());
        fixed.insert("object");
    }
    match response.get("created_at") {
        Some(Value::Number(number)) if number.is_u64() => {}
        Some(Value::Number(number)) => {
            let seconds = number.as_f64().unwrap_or_default().max(0.0) as u64;
            response.insert("created_at".into(), seconds.into());
            fixed.insert("created_at");
        }
        _ => {
            response.insert("created_at".into(), 0.into());
            fixed.insert("created_at");
        }
    }
    // Rig keeps only a string; an echo of the instructions as input items
    // is dropped, since Hotline sent them and does not read them back.
    if response
        .get("instructions")
        .is_some_and(|value| !value.is_string() && !value.is_null())
    {
        response.insert("instructions".into(), Value::Null);
        fixed.insert("instructions");
    }
    if let Some(Value::Object(error)) = response.get_mut("error") {
        if error.is_empty() {
            response.insert("error".into(), Value::Null);
            fixed.insert("error");
        } else {
            if !error.get("code").is_some_and(Value::is_string) {
                let code = match error.get("code") {
                    Some(Value::Null) | None => "error".to_string(),
                    Some(other) => other.to_string(),
                };
                error.insert("code".into(), code.into());
                fixed.insert("error");
            }
            if !error.get("message").is_some_and(Value::is_string) {
                error.insert("message".into(), "".into());
                fixed.insert("error");
            }
        }
    }
    if let Some(Value::Object(details)) = response.get_mut("incomplete_details")
        && !details.get("reason").is_some_and(Value::is_string)
    {
        if details.is_empty() {
            response.insert("incomplete_details".into(), Value::Null);
        } else {
            details.insert("reason".into(), "unknown".into());
        }
        fixed.insert("incomplete_details");
    }
    if let Some(Value::Array(tools)) = response.get_mut("tools") {
        for tool in tools.iter_mut().filter_map(Value::as_object_mut) {
            for key in ["name", "description"] {
                if tool.get(key).is_some_and(Value::is_null) {
                    tool.remove(key);
                    fixed.insert("tools");
                }
            }
        }
    }
    if let Some(Value::Array(output)) = response.get_mut("output") {
        for (index, item) in output.iter_mut().enumerate() {
            if let Value::Object(item) = item {
                repair_item(item, &format!("item_{index}"), true, fixed);
            }
        }
    }
}

fn repair_item(
    item: &mut Map<String, Value>,
    fallback: &str,
    done: bool,
    fixed: &mut BTreeSet<&'static str>,
) {
    // (needs an id, needs a status, the list it must carry)
    let (id, status, list) = match item.get("type").and_then(Value::as_str) {
        Some("message") => (true, true, Some(("content", "message content"))),
        Some("reasoning") => (true, false, Some(("summary", "reasoning summary"))),
        Some("function_call") => (false, true, None),
        _ => return,
    };
    if id && !item.get("id").is_some_and(Value::is_string) {
        item.insert("id".into(), fallback.into());
        fixed.insert("item id");
    }
    if status && !item.get("status").is_some_and(Value::is_string) {
        let status = if done { "completed" } else { "in_progress" };
        item.insert("status".into(), status.into());
        fixed.insert("item status");
    }
    if let Some((key, name)) = list
        && !item.get(key).is_some_and(Value::is_array)
    {
        item.insert(key.into(), Value::Array(Vec::new()));
        fixed.insert(name);
    }
    if !id && item.get("arguments").is_some_and(Value::is_null) {
        item.insert("arguments".into(), "".into());
        fixed.insert("function arguments");
    }
}

fn first_time(key: String) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SEEN.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key)
}

fn note_repairs(host: &str, fixed: &BTreeSet<&'static str>) {
    let fields = fixed.iter().copied().collect::<Vec<_>>().join(", ");
    if first_time(format!("repair {host} {fields}")) {
        eprintln!(
            "[custom] {host} sent Responses frames outside OpenAI's shape; repaired {fields}"
        );
    }
}

/// A frame still unreadable after repair ends the turn in Rig. Log it once
/// per server and event type, so the next gap can be closed from the log.
fn note_unreadable(host: &str, kind: &str, error: &serde_json::Error, frame: &[u8]) {
    if first_time(format!("unreadable {host} {kind}")) {
        let frame = String::from_utf8_lossy(frame)
            .chars()
            .take(800)
            .collect::<String>();
        eprintln!("[custom] {host} sent a {kind} frame Hotline cannot read ({error}): {frame}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repaired(event: Value) -> Value {
        let Value::Object(mut event) = event else {
            unreachable!()
        };
        repair_event(&mut event, &mut 0, &mut BTreeSet::new());
        Value::Object(event)
    }

    fn readable(event: Value) -> bool {
        serde_json::from_value::<StreamingCompletionChunk>(event).is_ok()
    }

    fn response() -> Value {
        json!({"id":"resp_1","object":"response","created_at":1,"status":"in_progress","model":"m","output":[],"tools":[]})
    }

    #[test]
    fn loose_compatible_frames_become_readable() {
        let mut instructions = response();
        instructions["instructions"] = json!([{"type":"message","role":"developer","content":[]}]);
        let mut float_time = response();
        float_time["created_at"] = json!(1_791_871_040.5);
        let mut empty_error = response();
        empty_error["error"] = json!({});
        let mut empty_details = response();
        empty_details["incomplete_details"] = json!({});
        let mut null_description = response();
        null_description["tools"] =
            json!([{"type":"function","name":"read","description":null,"parameters":{}}]);
        let loose = [
            json!({"type":"response.created","sequence_number":0,"response":instructions}),
            json!({"type":"response.created","sequence_number":0,"response":float_time}),
            json!({"type":"response.created","sequence_number":0,"response":empty_error}),
            json!({"type":"response.created","sequence_number":0,"response":empty_details}),
            json!({"type":"response.created","sequence_number":0,"response":null_description}),
            json!({"type":"response.created","response":response()}),
            json!({"type":"response.output_item.added","output_index":0,"sequence_number":1,"item":{"type":"message","role":"assistant","content":[]}}),
            json!({"type":"response.output_item.added","output_index":0,"sequence_number":1,"item":{"type":"reasoning","summary":null}}),
            json!({"type":"response.output_item.done","output_index":0,"sequence_number":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read","arguments":"{}"}}),
            json!({"type":"response.completed","sequence_number":9,"response":{"id":"resp_1","object":"response","created_at":1.0,"status":"completed","model":"m","output":[{"type":"reasoning","summary":null},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[]}]}]}}),
        ];
        for event in loose {
            assert!(!readable(event.clone()), "already readable: {event}");
            let fixed = repaired(event.clone());
            assert!(
                readable(fixed.clone()),
                "still unreadable: {event} -> {fixed}"
            );
        }
    }

    #[test]
    fn missing_sequence_numbers_continue_from_the_last_one_seen() {
        let mut next = 0;
        let mut fixed = BTreeSet::new();
        let mut seen = Vec::new();
        for event in [
            json!({"type":"response.output_text.delta","sequence_number":4}),
            json!({"type":"response.output_text.delta"}),
            json!({"type":"response.output_text.delta"}),
        ] {
            let Value::Object(mut event) = event else {
                unreachable!()
            };
            repair_event(&mut event, &mut next, &mut fixed);
            seen.push(event["sequence_number"].clone());
        }
        assert_eq!(seen, [json!(4), json!(5), json!(6)]);
    }

    #[test]
    fn openai_shaped_lines_pass_through_byte_for_byte() {
        let mut frames = Frames {
            host: "test".into(),
            next: 0,
        };
        let text = concat!(
            "event: response.output_text.delta\r\n",
            "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m\",\"output_index\":0,\"content_index\":0,\"sequence_number\":3,\"delta\":\"hi\"}\r\n",
            "\r\n",
            ": keep-alive\n",
            "data: [DONE]\n",
        );
        assert_eq!(frames.lines(text.as_bytes()), text.as_bytes());
    }

    #[tokio::test]
    async fn frames_split_across_chunks_are_repaired_whole() {
        let frame = "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"sequence_number\":1,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n";
        let (head, tail) = frame.split_at(40);
        let inner: BoxedStream = Box::pin(futures_util::stream::iter([
            Ok(Bytes::from(head.to_string())),
            Ok(Bytes::from(tail.to_string())),
        ]));
        let out = repaired_stream(inner, "test".into())
            .map(|chunk| chunk.unwrap())
            .collect::<Vec<_>>()
            .await
            .concat();
        let line = String::from_utf8(out).unwrap();
        let data = line.lines().next().unwrap().strip_prefix("data: ").unwrap();
        let event = serde_json::from_str::<Value>(data).unwrap();
        assert_eq!(event["item"]["id"], "item_0");
        assert_eq!(event["item"]["status"], "in_progress");
        assert!(readable(event));
    }
}
