//! The log: everything the room remembers, as events on a stream.
//!
//! A stream is an append-only JSONL file — one event per line — folded by
//! `id` when it is read. Some events are written more than once: a tool call
//! moves from pending to completed, a permission card is answered, a teammate
//! is renamed. A later line with the same `id` supersedes the earlier one and
//! keeps its place, so the fold is the whole state of a stream, and a
//! compaction is that fold written back over the file.
//!
//! Three streams, one rule for all of them:
//!
//! - [`StreamId::Room`] is the room itself — the roster, the settings, the
//!   schedules — in `room.jsonl`. One file, no epochs.
//! - [`StreamId::Tape`] is one teammate's conversation, in
//!   `transcripts/<id>/<epoch>.jsonl`, with the legacy flat file standing in
//!   for epoch 1. Byte-for-byte what the previous Toad writes, so importing
//!   its data directory copies tapes unchanged.
//! - [`StreamId::Thread`] is one pair of teammates' conversation, in
//!   `threads/<key>.jsonl` beside a sidecar naming the two sides.
//!
//! [`Log`] is the only door to all three, and the only writer. A reader that
//! wants history calls [`Log::load`]; a reader that wants to keep up calls
//! [`Log::subscribe`] and is handed every event appended after it asked.
//! There is no cursor and no "from" on the subscription: history and the live
//! feed are two questions, and putting the two answers in order belongs to
//! the wire, which is the part that knows what its client has already seen.

pub mod thread;

mod tape;

pub(crate) use tape::{open_epoch, segments_of};

use crate::paths::room_path;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::broadcast;

/// How far behind a subscriber may fall before it starts missing events.
///
/// A subscriber is one socket's pump, which does nothing but forward, so the
/// depth only has to cover a burst — a streaming turn writing faster than a
/// socket drains. One that falls further behind than this is told it lagged
/// and reloads the fold; that is cheaper for everybody than a queue the log
/// grows without bound.
const SUBSCRIPTION_DEPTH: usize = 256;

/// Which stream. The room's belongs to the room, a tape's to one teammate,
/// a thread's to one pair — the key from [`crate::paths::thread_key`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StreamId {
    Room,
    Tape(String),
    Thread(String),
}

/// One write, as replication sees it: which bytes landed where. The bytes are
/// the serialized line, newline included.
///
/// `append` hands this back rather than ringing a seam: the log must not know
/// about wires, and a return value is the version with one fewer moving part.
/// The caller with a mesh to feed pushes it; the caller with none drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    /// The segment the line landed in. Only a tape has more than one, so for
    /// the room and for a thread this is always 1.
    pub epoch: u64,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// The log over one data directory.
///
/// Cloning is cheap and shares the subscribers, so a clone is the same log:
/// the session that appends and the wire that listens hold their own copies
/// and still meet on every event.
#[derive(Clone)]
pub struct Log {
    root: PathBuf,
    subscribers: Arc<Mutex<HashMap<StreamId, broadcast::Sender<Value>>>>,
    /// The one writer. Held for the whole of an append and the whole of a
    /// compaction, because both are read-then-write: an append measures the
    /// file to say where its bytes landed and may relocate a legacy flat file
    /// on the way, and a compaction rewrites the file it just read. A tape has
    /// several writers above it — the line a person typed, the turn it
    /// started, the idle sweep, a colleague's peer session — and this is where
    /// they become one.
    writer: Arc<Mutex<()>>,
}

impl Log {
    /// Opening touches nothing: a stream's file is made when something is
    /// appended to it, and an empty data directory is a room with no history
    /// rather than an error.
    pub fn open(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            writer: Arc::new(Mutex::new(())),
        }
    }

    /// The data directory this log is the log of. What lives beside the
    /// streams — the search index, a teammate's workspace — is named from
    /// here.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The whole stream, folded. A stream nobody has written to is empty.
    pub fn load(&self, stream: &StreamId) -> Vec<Value> {
        let mut events = Vec::new();
        for file in self.readable_files(stream) {
            if let Ok(text) = fs::read_to_string(&file) {
                events.extend(parse_lines(&text));
            }
        }
        fold(events.into_iter())
    }

    /// Adds one event to the end of the stream, then hands it to whoever is
    /// listening.
    pub fn append(&self, stream: &StreamId, event: &Value) -> io::Result<Appended> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let (file, epoch) = self.writable_file(stream)?;
        let offset = fs::metadata(&file).map_or(0, |file| file.len());
        let bytes = format!("{event}\n").into_bytes();
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?
            .write_all(&bytes)?;
        self.publish(stream, event);
        Ok(Appended {
            epoch,
            offset,
            bytes,
        })
    }

    /// Rewrites the file this stream is written to with its fold, and answers
    /// the epoch it rewrote. A tape's older segments are closed history and
    /// are left alone, duplicates and all.
    ///
    /// A fold that changes nothing skips the write and answers `None`,
    /// because announcing a rewrite costs every mirror its copy of the epoch,
    /// and a rewrite nobody made is not worth that.
    pub fn compact(&self, stream: &StreamId) -> io::Result<Option<u64>> {
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let (file, epoch) = self.writable_file(stream)?;
        if !file.exists() {
            return Ok(None);
        }
        let before = fs::read_to_string(&file)?;
        let events = fold(parse_lines(&before));
        if events.is_empty() {
            return Ok(None);
        }
        let after = events
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>()
            .join("\n")
            + "\n";
        if after == before {
            return Ok(None);
        }
        fs::write(&file, after)?;
        Ok(Some(epoch))
    }

    /// Every event appended after this call, live.
    ///
    /// Nothing already on disk comes back this way — a subscriber that wants
    /// history loads it. A receiver that falls [`SUBSCRIPTION_DEPTH`] events
    /// behind is told it lagged, which is its own problem to recover from.
    pub fn subscribe(&self, stream: &StreamId) -> broadcast::Receiver<Value> {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        subscribers
            .entry(stream.clone())
            .or_insert_with(|| broadcast::channel(SUBSCRIPTION_DEPTH).0)
            .subscribe()
    }

    /// After the bytes are on disk, never before: a subscriber that acted on
    /// an event the log then failed to write would be acting on a fact the
    /// next load does not have.
    fn publish(&self, stream: &StreamId, event: &Value) {
        let subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = subscribers.get(stream) {
            // No receivers left is not a failure; the last one hung up.
            let _ = sender.send(event.clone());
        }
    }

    /// The file an append goes to, and the epoch that file is, making the
    /// directories it needs on the way.
    fn writable_file(&self, stream: &StreamId) -> io::Result<(PathBuf, u64)> {
        match stream {
            StreamId::Room => {
                fs::create_dir_all(&self.root)?;
                Ok((room_path(&self.root), 1))
            }
            StreamId::Tape(persona_id) => tape::writable_segment(&self.root, persona_id),
            StreamId::Thread(key) => {
                let file = thread::file(&self.root, key)?;
                fs::create_dir_all(crate::paths::threads_dir(&self.root))?;
                Ok((file, 1))
            }
        }
    }

    /// Every file the stream is spread over, oldest first. Reading makes
    /// nothing and moves nothing: a tape's legacy flat file stays where it is
    /// until somebody writes.
    fn readable_files(&self, stream: &StreamId) -> Vec<PathBuf> {
        match stream {
            StreamId::Room => vec![room_path(&self.root)],
            StreamId::Tape(persona_id) => tape::segments_of(&self.root, persona_id)
                .into_iter()
                .map(|(_, path)| path)
                .collect(),
            StreamId::Thread(key) => thread::file(&self.root, key).into_iter().collect(),
        }
    }
}

/// One line is one event; a torn final line from an unclean exit is skipped.
pub(crate) fn parse_lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
}

/// Later lines win by `id`, and each id keeps the place it first appeared.
pub(crate) fn fold(events: impl Iterator<Item = Value>) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for event in events {
        let Some(id) = event.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if !by_id.contains_key(&id) {
            order.push(id.clone());
        }
        by_id.insert(id, event);
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// The permission and human-action cards a restart orphaned, superseded as
/// expired.
///
/// A resolver only exists in the process that received the request, so a card
/// that still claims to be live after a restart is a button nobody is behind.
/// It is a free function over events because tapes and threads both carry
/// cards and there should be one copy of the rule: the caller appends what
/// this answers and then compacts, which is the whole startup fold.
///
/// A permission with a `decision` of `null` is left alone — a decision
/// somebody wrote is a decision. A human-action card that is no longer
/// `pending` is the same fact.
pub fn expire_orphaned_permissions(events: &[Value], ts: i64) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| match event.get("kind").and_then(Value::as_str) {
            Some("permission") if event.get("decision").is_none() => {
                let mut expired = event.as_object()?.clone();
                expired.insert("ts".into(), Value::from(ts));
                expired.insert("decision".into(), Value::from("expired"));
                Some(Value::Object(expired))
            }
            Some("human_action")
                if event.get("status").and_then(Value::as_str) == Some("pending") =>
            {
                let mut expired = event.as_object()?.clone();
                expired.insert("ts".into(), Value::from(ts));
                expired.insert("status".into(), Value::from("expired"));
                Some(Value::Object(expired))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::broadcast::error::TryRecvError;

    fn scratch(name: &str) -> Log {
        let root =
            std::env::temp_dir().join(format!("toad-core-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Log::open(root)
    }

    fn setting(key: &str, value: i64) -> Value {
        json!({"kind": "setting", "id": key, "value": value})
    }

    #[test]
    fn the_room_is_one_file_that_folds_like_every_other_stream() {
        let log = scratch("room-file");
        log.append(&StreamId::Room, &setting("chapterIdleHours", 8))
            .unwrap();
        let second = log
            .append(&StreamId::Room, &setting("chapterIdleHours", 2))
            .unwrap();

        assert_eq!(log.load(&StreamId::Room), [setting("chapterIdleHours", 2)]);
        assert_eq!(second.epoch, 1);
        assert_eq!(
            fs::read_to_string(room_path(log.root())).unwrap(),
            format!(
                "{}\n{}\n",
                setting("chapterIdleHours", 8),
                setting("chapterIdleHours", 2)
            )
        );

        assert_eq!(log.compact(&StreamId::Room).unwrap(), Some(1));
        assert_eq!(
            fs::read_to_string(room_path(log.root())).unwrap(),
            format!("{}\n", setting("chapterIdleHours", 2))
        );
    }

    #[test]
    fn two_subscribers_both_see_an_append_and_a_third_stream_hears_nothing() {
        let log = scratch("subscribers");
        let ada = StreamId::Tape("ada".into());
        let mut first = log.subscribe(&ada);
        let mut second = log.subscribe(&ada);
        let mut elsewhere = log.subscribe(&StreamId::Tape("bob".into()));

        let event = json!({"kind": "user", "id": "u1", "ts": 1, "text": "hi"});
        log.append(&ada, &event).unwrap();

        assert_eq!(first.try_recv().unwrap(), event);
        assert_eq!(second.try_recv().unwrap(), event);
        assert!(matches!(elsewhere.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn a_subscriber_hears_only_what_lands_after_it_asked() {
        let log = scratch("late-subscriber");
        let earlier = json!({"kind": "user", "id": "u1", "ts": 1, "text": "before"});
        let later = json!({"kind": "user", "id": "u2", "ts": 2, "text": "after"});
        log.append(&StreamId::Room, &earlier).unwrap();

        let mut listener = log.subscribe(&StreamId::Room);
        assert!(matches!(listener.try_recv(), Err(TryRecvError::Empty)));

        log.append(&StreamId::Room, &later).unwrap();
        assert_eq!(listener.try_recv().unwrap(), later);
        // History is loaded, not replayed: both lines are still in the fold.
        assert_eq!(log.load(&StreamId::Room), [earlier, later]);
    }

    /// A tape has several writers above it — the line a person typed, the turn
    /// it started, the idle sweep, a colleague's peer session — and the
    /// offsets the log answers with are what a mirror will ship the bytes by.
    /// Two of them measuring the same file before either has written would
    /// each be told the same place.
    #[test]
    fn appends_from_many_threads_land_at_the_offsets_they_were_told() {
        let log = scratch("one-writer");
        let landed: Vec<Appended> = std::thread::scope(|scope| {
            let writers: Vec<_> = (0..8)
                .map(|writer| {
                    let log = log.clone();
                    scope.spawn(move || {
                        (0..40)
                            .map(|n| {
                                log.append(&StreamId::Room, &setting(&format!("{writer}-{n}"), n))
                                    .unwrap()
                            })
                            .collect::<Vec<Appended>>()
                    })
                })
                .collect();
            writers
                .into_iter()
                .flat_map(|writer| writer.join().unwrap())
                .collect()
        });

        let mut by_offset = landed;
        by_offset.sort_by_key(|appended| appended.offset);
        let mut end = 0;
        for appended in &by_offset {
            assert_eq!(
                appended.offset, end,
                "an append was told an offset another one had already taken"
            );
            end += appended.bytes.len() as u64;
        }
        assert_eq!(fs::metadata(room_path(log.root())).unwrap().len(), end);
        assert_eq!(log.load(&StreamId::Room).len(), 8 * 40);
    }

    #[test]
    fn a_clone_of_the_log_is_the_same_log() {
        let log = scratch("clone");
        let mut listener = log.subscribe(&StreamId::Room);
        let event = json!({"kind": "setting", "id": "defaultBackendId", "value": "pi"});
        log.clone().append(&StreamId::Room, &event).unwrap();
        assert_eq!(listener.try_recv().unwrap(), event);
    }
}
