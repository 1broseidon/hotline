//! Derived state under the log's writer lock. Disk remains the restart record.
use super::StreamId;
use serde_json::Value;
use std::collections::HashMap;
use std::mem::size_of;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_STREAMS: usize = 64;

#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<StreamId, Fold>,
    clock: u64,
}

pub(super) struct Fold {
    pub events: Vec<Value>,
    positions: HashMap<String, usize>,
    chapter: Option<usize>,
    message: Option<usize>,
    bytes: usize,
    used: u64,
}

impl Fold {
    pub fn new(events: Vec<Value>) -> Self {
        let mut fold = Self {
            events: Vec::new(),
            positions: HashMap::new(),
            chapter: None,
            message: None,
            bytes: 0,
            used: 0,
        };
        for event in events {
            fold.append(event);
        }
        fold
    }

    pub fn contains(&self, id: &str) -> bool {
        self.positions.contains_key(id)
    }

    pub fn chapter_activity(&self) -> (Option<Value>, Option<i64>) {
        let Some(index) = self.chapter else {
            return (None, None);
        };
        let chapter = &self.events[index];
        if chapter.get("endedAt").is_some() {
            return (None, None);
        }
        let activity = self
            .message
            .filter(|message| *message > index)
            .and_then(|message| self.events[message].get("ts").and_then(Value::as_i64));
        (Some(chapter.clone()), activity)
    }

    fn append(&mut self, event: Value) {
        let Some(id) = event.get("id").and_then(Value::as_str) else {
            return;
        };
        let index = if let Some(&index) = self.positions.get(id) {
            self.bytes -= weight(&self.events[index]);
            index
        } else {
            let index = self.events.len();
            // Include the id index and spare collection capacity in the budget.
            self.bytes += id.len() + 128;
            self.positions.insert(id.to_owned(), index);
            self.events.push(Value::Null);
            index
        };
        self.bytes += weight(&event);
        self.events[index] = event;
        if self.events[index]["kind"] == "chapter" {
            self.chapter = Some(self.chapter.map_or(index, |old| old.max(index)));
        } else if self.chapter == Some(index) {
            self.chapter = self.events.iter().rposition(|e| e["kind"] == "chapter");
        }
        if crate::chapters::is_message(&self.events[index]) {
            self.message = Some(self.message.map_or(index, |old| old.max(index)));
        } else if self.message == Some(index) {
            self.message = self.events.iter().rposition(crate::chapters::is_message);
        }
    }
}

// A conservative heap estimate, rather than serialized bytes: small JSON
// fields and the fold's id index can cost more than their text on disk.
fn weight(value: &Value) -> usize {
    2 * size_of::<Value>()
        + match value {
            Value::String(text) => text.capacity(),
            Value::Array(values) => values.iter().map(weight).sum(),
            Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| key.capacity() + 128 + weight(value))
                .sum(),
            _ => 0,
        }
}

impl Cache {
    pub fn get(&mut self, stream: &StreamId) -> Option<&Fold> {
        self.clock += 1;
        let entry = self.entries.get_mut(stream)?;
        entry.used = self.clock;
        Some(entry)
    }

    pub fn insert(&mut self, stream: &StreamId, mut fold: Fold) {
        if *stream != StreamId::Room && fold.bytes > MAX_BYTES {
            self.entries.remove(stream);
            return;
        }
        self.clock += 1;
        fold.used = self.clock;
        self.entries.insert(stream.clone(), fold);
        self.trim();
    }

    pub fn append(&mut self, stream: &StreamId, event: &Value) {
        if let Some(fold) = self.entries.get_mut(stream) {
            fold.append(event.clone());
            if *stream != StreamId::Room && fold.bytes > MAX_BYTES {
                self.entries.remove(stream);
            }
        }
        self.trim();
    }

    pub fn remove(&mut self, stream: &StreamId) {
        self.entries.remove(stream);
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn trim(&mut self) {
        loop {
            let tapes: Vec<_> = self
                .entries
                .iter()
                .filter(|(id, _)| **id != StreamId::Room)
                .collect();
            if tapes.len() <= MAX_STREAMS
                && tapes.iter().map(|(_, fold)| fold.bytes).sum::<usize>() <= MAX_BYTES
            {
                break;
            }
            let oldest = tapes
                .iter()
                .min_by_key(|(_, fold)| fold.used)
                .map(|(id, _)| (*id).clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
