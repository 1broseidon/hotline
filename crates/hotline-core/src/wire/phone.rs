//! How a snapshot is shaped for the seat that reads it: a phone's window and budget.

use super::RoomHandle;
use crate::contract::SessionConfigCategory;
use crate::log::{Log, StreamId};
use crate::thread::Link;
use serde_json::{Value, json};
use std::sync::Arc;

/// A stream's fold, then its events, and on a tape the deltas that are never
/// written beside them. A thread's link is sent as the marker its kind has
/// always had (see [`Link::wire`]), or as itself to a client that declared
/// `threads2` (see [`Link::threads2`]).
fn public_snapshot(log: &Log, stream: &StreamId, threads2: bool) -> Vec<Value> {
    let events = log.load(stream);
    if *stream == StreamId::Room {
        events
            .into_iter()
            .map(crate::mcp::public_room_event)
            .collect()
    } else {
        events
            .into_iter()
            .map(|event| link_as_sent(event, threads2))
            .collect()
    }
}

/// An event as this client is sent it: a thread's link as the one `link` shape
/// when it declared `threads2`, and as the marker its kind has always had when
/// it did not.
pub(super) fn link_as_sent(event: Value, threads2: bool) -> Value {
    if threads2 {
        Link::threads2(event)
    } else {
        Link::wire(event)
    }
}

/// A computer frame the phone can afford: no wider than this, a JPEG at
/// this quality, and only when that comes out under this many bytes.
/// Otherwise the frame is stripped as before and the desktop keeps the
/// full picture.
const PHONE_FRAME_WIDTH: u32 = 720;
const PHONE_FRAME_QUALITY: u8 = 60;
const PHONE_FRAME_BYTES: usize = 96 * 1024;

/// The phone-sized copy of a frame's data URL, when one fits the budget.
fn phone_frame(data_url: &str) -> Option<String> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    let (_, encoded) = data_url.split_once(";base64,")?;
    let decoded = image::load_from_memory(&BASE64_STANDARD.decode(encoded).ok()?).ok()?;
    let sized = if decoded.width() > PHONE_FRAME_WIDTH {
        decoded.resize(
            PHONE_FRAME_WIDTH,
            u32::MAX,
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let mut out = std::io::Cursor::new(Vec::new());
    sized
        .to_rgb8()
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            PHONE_FRAME_QUALITY,
        ))
        .ok()?;
    let jpeg = out.into_inner();
    (jpeg.len() <= PHONE_FRAME_BYTES)
        .then(|| format!("data:image/jpeg;base64,{}", BASE64_STANDARD.encode(jpeg)))
}

// The phone shows a bounded recent window; full history remains on desktop.
/// Whether this config is the one a phone may set: a model's effort, by the
/// category the session reports rather than by the name a harness gave it.
pub(super) fn effort_config(room: &Arc<dyn RoomHandle>, persona_id: &str, config_id: &str) -> bool {
    room.info(persona_id).configs.iter().any(|config| {
        config.id == config_id && config.category == Some(SessionConfigCategory::Effort)
    })
}

pub(super) fn phone_event(mut event: Value) -> Value {
    let mut truncated = false;
    if let Some(text) = event.get("text").and_then(Value::as_str)
        && text.len() > 8192
    {
        let short: String = text.chars().take(2000).collect();
        event["text"] = json!(format!("{short}\n[Continue reading on desktop]"));
        truncated = true;
    }
    if let Some(data_url) = event.get("dataUrl").and_then(Value::as_str) {
        if let Some(small) = phone_frame(data_url) {
            event["dataUrl"] = json!(small)
        } else {
            event.as_object_mut().unwrap().remove("dataUrl");
            truncated = true;
        }
    }
    if event.get("output").is_some() {
        event.as_object_mut().unwrap().remove("output");
        truncated = true;
    }
    if truncated {
        event["mobileTruncated"] = json!(true);
    }
    event
}
/// How many of a tape's lines the window's subscription opens with. A busy
/// teammate's tape runs to thousands of lines and megabytes; the window draws
/// the last of them and asks for older ones with `tape.page` as you scroll.
const DESK_TAPE_WINDOW: usize = 400;

/// Where a tape's window starts: `len` lines from the end, or further back
/// to the last thing said when those are all steps. A turn that has run for a
/// long time is tool calls and thoughts alone, and a window with no message
/// in it draws a blank chat until the turn ends.
fn window_start(events: &[Value], len: usize) -> usize {
    let start = events.len().saturating_sub(len);
    let said = |event: &Value| {
        matches!(
            event.get("kind").and_then(Value::as_str),
            Some("user" | "agent")
        )
    };
    if events[start..].iter().any(said) {
        return start;
    }
    events[..start].iter().rposition(said).unwrap_or(start)
}

/// How many lines a phone's window opens with, and the most it may ask for.
pub(super) const PHONE_WINDOW: usize = 200;
/// The most a phone's snapshot or page carries, in bytes of JSON.
const PHONE_BYTES: usize = 512 * 1024;

/// Events cut for a phone, newest kept first until the budget is spent:
/// what a page of older lines or a window comes to on a phone's link.
pub(super) fn phone_budget(events: Vec<Value>) -> Vec<Value> {
    let mut used = 0;
    let mut kept = Vec::new();
    for event in events.into_iter().rev() {
        let event = phone_event(event);
        used += event.to_string().len();
        if used > PHONE_BYTES && !kept.is_empty() {
            break;
        }
        kept.push(event);
    }
    kept.reverse();
    kept
}

/// A stream's opening lines as this socket is sent them. A phone gets a
/// window of `window` lines (or [`PHONE_WINDOW`]), cut and within budget.
pub(super) fn snapshot_shaped(
    log: &Log,
    stream: &StreamId,
    phone: bool,
    window: Option<usize>,
    threads2: bool,
) -> Vec<Value> {
    let events = public_snapshot(log, stream, threads2);
    if !phone {
        // The desk and its owner device open a tape on its last lines and
        // page back with `tape.page`; a whole tape is too much to draw.
        if matches!(stream, StreamId::Tape(_)) && events.len() > DESK_TAPE_WINDOW {
            let skip = window_start(&events, DESK_TAPE_WINDOW);
            return events.into_iter().skip(skip).collect();
        }
        return events;
    }
    let window = window.unwrap_or(PHONE_WINDOW).min(PHONE_WINDOW);
    let skip = if matches!(stream, StreamId::Tape(_)) {
        window_start(&events, window)
    } else {
        events.len().saturating_sub(window)
    };
    phone_budget(events.into_iter().skip(skip).collect())
}
