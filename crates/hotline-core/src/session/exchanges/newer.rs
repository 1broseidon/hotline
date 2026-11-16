//! A pair record as the room stream keeps it, and what a newer Hotline wrote
//! into it that this build does not read (`docs/log.md`, The room stream).

use super::{Pair, Record, Request};
use serde::{Deserialize, Serialize};

/// What a newer Hotline wrote into a pair record that this build does not
/// read, written back exactly as it was found and never acted on, so going
/// back to that build finds its record as it left it: its fields on the
/// pair, and its requests whole, each with the place it had among the
/// pair's requests. A request is the newer build's when this build cannot
/// read it (a phase or intent it does not know) or it carries a field this
/// build does not know: either way this build cannot tell what moving it
/// would mean there, so it is not queued, run, failed, stopped or
/// delivered here.
#[derive(Clone, Debug, Default)]
pub(super) struct Newer {
    fields: serde_json::Map<String, serde_json::Value>,
    requests: Vec<(usize, serde_json::Value)>,
}
impl Newer {
    /// Whether a newer Hotline wrote anything here: a field on the pair, or
    /// a request of its own.
    pub(super) fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.requests.is_empty()
    }
}
/// Fields an older Hotline wrote on a request that this build no longer
/// reads: the asides crossing a link with it, from before the inbox. A
/// request with only these besides its own is an older build's, which this
/// one reads and moves, and they are not written back.
const OLDER: [&str; 1] = ["crossing"];
/// A pair as the room stream keeps it.
#[derive(Serialize, Deserialize)]
struct PairRecord {
    id: String,
    a: String,
    b: String,
    exchanges: i64,
    paused: bool,
    /// Left out while false, so a record from before the person's Stop
    /// exchange reads as not stopped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    stopped: bool,
    requests: Vec<serde_json::Value>,
    #[serde(flatten)]
    newer: serde_json::Map<String, serde_json::Value>,
}
impl Serialize for Pair {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut requests = self
            .requests
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::ser::Error::custom)?;
        // In the order they were read, each goes back where it stood: this
        // build only ever adds a request after the others.
        for (place, request) in &self.newer.requests {
            requests.insert((*place).min(requests.len()), request.clone());
        }
        PairRecord {
            id: self.id.clone(),
            a: self.a.clone(),
            b: self.b.clone(),
            exchanges: self.exchanges,
            paused: self.paused,
            stopped: self.stopped,
            requests,
            newer: self.newer.fields.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Pair {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut record = PairRecord::deserialize(deserializer)?;
        // The stream's own slot, which `save_pair` writes.
        record.newer.remove("kind");
        let mut requests = Vec::new();
        let mut newer = Vec::new();
        for (place, raw) in record.requests.into_iter().enumerate() {
            match Record::deserialize(&raw) {
                Ok(mut read) => {
                    read.newer
                        .retain(|field, _| !OLDER.contains(&field.as_str()));
                    if read.newer.is_empty() {
                        requests.push(Request::from(read));
                    } else {
                        newer.push((place, raw));
                    }
                }
                Err(_) => newer.push((place, raw)),
            }
        }
        Ok(Pair {
            id: record.id,
            a: record.a,
            b: record.b,
            exchanges: record.exchanges,
            paused: record.paused,
            stopped: record.stopped,
            requests,
            newer: Newer {
                fields: record.newer,
                requests: newer,
            },
        })
    }
}
