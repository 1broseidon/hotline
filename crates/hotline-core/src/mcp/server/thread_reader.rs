//! A teammate reads its own record, under the calling session's lease.
use crate::contract::{ThreadId, ThreadKind};
use crate::log::StreamId;
use crate::session::Room;
use crate::thread::{Participant, ThreadStore};
use serde_json::{Value, json};

pub(super) fn read(room: &Room, persona: &str, args: &Value) -> Result<Value, String> {
    let key = args["thread"]
        .as_str()
        .ok_or("read_thread needs a thread key.")?;
    let (kind, id) = key
        .split_once(':')
        .ok_or("Use a thread key such as call:<id>.")?;
    let kind = ThreadKind::parse(kind).ok_or("Unknown thread kind.")?;
    let thread = ThreadId::new(kind, id);
    let found = ThreadStore::new(room.log())
        .load(&thread)
        .ok_or("No such thread of yours.")?;
    // A handoff's opener participates, but does not own the recipient's work.
    let owns = if kind == ThreadKind::Pair {
        found
            .participants
            .contains(&Participant::Persona(persona.into()))
    } else if kind == ThreadKind::Dm {
        id == persona
    } else {
        found
            .parent
            .as_ref()
            .is_some_and(|parent| parent.thread == ThreadId::dm(persona))
    };
    if !owns {
        return Err("No such thread of yours.".into());
    }
    let stream = StreamId::of_thread(&thread).ok_or("That thread has no lines.")?;
    let events = room
        .log()
        .try_load(&stream)
        .map_err(|error| format!("The thread could not be read: {error}"))?;
    let start = match args.get("after") {
        None => 0,
        Some(Value::String(after)) => events
            .iter()
            .position(|event| event["id"] == *after)
            .map(|at| at + 1)
            .ok_or("That cursor is not in this thread.")?,
        _ => return Err("after must be an event id.".into()),
    };
    let limit = match args.get("limit") {
        None => 50,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .ok_or("limit must be between 1 and 100.")? as usize,
    };
    let end = (start + limit).min(events.len());
    let (user, agent) = if kind == ThreadKind::Pair {
        crate::paths::thread_participants(id).ok_or("Invalid pair thread.")?
    } else {
        ("person", persona)
    };
    let lines: Vec<Value> = events[start..end]
        .iter()
        .map(|event| {
            let mut line = event.clone();
            let speaker = match event["kind"].as_str() {
                Some("user") => user,
                Some("agent") => agent,
                Some("tool" | "thought") if kind != ThreadKind::Pair => persona,
                _ => "record",
            };
            line["speaker"] = json!(speaker);
            line
        })
        .collect();
    Ok(json!({
        "thread": key,
        "description": if kind == ThreadKind::Call { "This thread IS the call's transcript." } else { "The thread's full record in order." },
        "lines": lines,
        "next_after": if end < events.len() { events[end - 1].get("id") } else { None },
    }))
}
