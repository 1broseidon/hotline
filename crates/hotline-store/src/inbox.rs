//! The items in a teammate's inbox, read off its own stream.
//!
//! Each is one `inbox_item` line on [`StreamId::Inbox`], written when it is
//! sent and again under the same id each time it changes, so the fold is the
//! whole state: what was sent, and where it stands. The room writes them;
//! this only reads.
//!
//! A build leaves alone what a newer build wrote: a field it does not know
//! rides on the item and is written back with it, and an item line it cannot
//! read at all is never an item here ([`newer_of`]).

use crate::log::{Log, StreamId};
use hotline_contract::{InboxItem, InboxNewerItem, InboxView};
use serde_json::Value;

/// The `kind` an item's line carries.
pub const KIND: &str = "inbox_item";

/// The most closed items, and the most sent ones, the window is sent of a
/// teammate's inbox: the newest. Closed items leave the inbox once they have
/// been kept as long as the desk keeps them, so this is only a ceiling.
pub const VIEW_MOST: usize = 200;

/// The fields of an item that hold a thread, whose own fields a newer build
/// may add to.
const THREADS: [&str; 3] = ["fromThread", "about", "work"];

/// The fields of a thread this build reads.
const THREAD_FIELDS: [&str; 3] = ["desk", "kind", "key"];

/// How far apart, at most, the items one `send` made were kept, for an
/// item kept before its send id was: a send keeps one for each teammate it
/// reaches in one go, well inside this.
const ONE_SEND_MS: i64 = 5_000;

/// The most items a chain of answers is followed through, so a chain that
/// names itself, which no build writes, still ends.
const CHAIN_MOST: usize = 1_000;

/// One item as its line on a stream holds it, or nothing for any other line
/// and for an item line this build cannot read ([`newer_of`]). What a newer
/// Hotline wrote on it that this build does not know is kept on the item
/// (`InboxItem::newer`), and [`line`] writes it back.
pub fn read(event: &Value) -> Option<InboxItem> {
    if event.get("kind").and_then(Value::as_str) != Some(KIND) {
        return None;
    }
    let mut item: InboxItem = serde_json::from_value(event.clone()).ok()?;
    // The stream's own slot, which `line` writes.
    item.newer.remove("kind");
    // A thread with a field this build does not read is kept as found.
    for field in THREADS {
        if let Some(thread) = event.get(field).and_then(Value::as_object)
            && thread
                .keys()
                .any(|key| !THREAD_FIELDS.contains(&key.as_str()))
        {
            item.nested
                .insert(field.into(), Value::Object(thread.clone()));
        }
    }
    Some(item)
}

/// An item as its line is written: with what a newer build wrote on it, and
/// each thread it names as it was found while it is still that thread.
pub fn line(item: &InboxItem) -> Value {
    let mut value = serde_json::to_value(item).unwrap_or(Value::Null);
    if let Some(fields) = value.as_object_mut() {
        fields.insert("kind".into(), Value::from(KIND));
        for (field, found) in &item.nested {
            let same = fields
                .get(field)
                .and_then(Value::as_object)
                .is_some_and(|now| {
                    THREAD_FIELDS
                        .iter()
                        .all(|key| now.get(*key) == found.get(*key))
                });
            if same {
                fields.insert(field.clone(), found.clone());
            }
        }
    }
    value
}

/// Every item sent to the teammate, held ones included, oldest first.
pub fn of(log: &Log, persona_id: &str) -> Vec<InboxItem> {
    log.load(&StreamId::Inbox(persona_id.to_string()))
        .iter()
        .filter_map(read)
        .collect()
}

/// The item lines on the teammate's stream that this build cannot read, as
/// a newer Hotline wrote them, oldest first: a state, an outcome or another
/// value this build does not know. They are none of [`of`]'s, so nothing
/// here moves, expires or sweeps them; they are only listed as they are.
pub fn newer_of(log: &Log, persona_id: &str) -> Vec<Value> {
    log.load(&StreamId::Inbox(persona_id.to_string()))
        .into_iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some(KIND))
        .filter(|event| read(event).is_none())
        .collect()
}

/// The items the teammate has: every one sent to it that is not held for the
/// person's first-contact card.
pub fn received(log: &Log, persona_id: &str) -> Vec<InboxItem> {
    of(log, persona_id)
        .into_iter()
        .filter(|item| !item.held)
        .collect()
}

/// How many open items the teammate has not read yet.
pub fn unread(log: &Log, persona_id: &str) -> usize {
    received(log, persona_id)
        .iter()
        .filter(|item| item.state.unread())
        .count()
}

/// The items `persona_id` sent, out of the inboxes of `teammates`, oldest
/// first. Hotline's own notices, which name the teammate they are about as
/// their sender, are not among them: it sent none of them.
pub fn sent_by<'a>(
    log: &Log,
    persona_id: &str,
    teammates: impl IntoIterator<Item = &'a str>,
) -> Vec<InboxItem> {
    let mut sent: Vec<InboxItem> = teammates
        .into_iter()
        .filter(|teammate| *teammate != persona_id)
        .flat_map(|teammate| of(log, teammate))
        .filter(|item| item.from == persona_id && item.notice.is_none())
        .collect();
    sent.sort_by_key(|item| item.ts);
    sent
}

/// Every item kept in the inboxes of `teammates`, with the topic and send
/// id of each worked out where it was kept before they were recorded
/// ([`topics`]).
pub fn everything<'a>(log: &Log, teammates: impl IntoIterator<Item = &'a str>) -> Vec<InboxItem> {
    let mut items: Vec<InboxItem> = teammates
        .into_iter()
        .flat_map(|teammate| of(log, teammate))
        .collect();
    topics(&mut items);
    items
}

/// Gives each item kept before topics and send ids were recorded the ones
/// it would have had, out of `items`, which should hold every item it could
/// be linked to. The items one `send` made for several, which name each
/// other in `also_to`, share the send id of the first of them: the same
/// sender, title and teammates reached, kept within [`ONE_SEND_MS`]. An
/// item's topic is the stored topic of the nearest item up its chain of
/// `reply_to` that has one, else that chain's first item's send id, else
/// that first item's id: a chain whose first item is gone starts at the
/// oldest item still kept. Nothing here is written back.
pub fn topics(items: &mut [InboxItem]) {
    let mut sends: Vec<usize> = (0..items.len())
        .filter(|at| items[*at].send_id.is_none() && !items[*at].also_to.is_empty())
        .collect();
    sends.sort_by_key(|at| items[*at].ts);
    let reached = |item: &InboxItem| {
        let mut all: Vec<&str> = item.also_to.iter().map(String::as_str).collect();
        all.push(&item.to);
        all.sort_unstable();
        all.join(" ")
    };
    let mut firsts: Vec<usize> = Vec::new();
    for at in sends {
        let item = &items[at];
        let first = firsts.iter().rev().copied().find(|first| {
            let first = &items[*first];
            first.from == item.from
                && first.title == item.title
                && reached(first) == reached(item)
                && item.ts - first.ts <= ONE_SEND_MS
        });
        let first = first.unwrap_or_else(|| {
            firsts.push(at);
            at
        });
        items[at].send_id = Some(items[first].id.clone());
    }
    let index: std::collections::HashMap<String, usize> = items
        .iter()
        .enumerate()
        .map(|(at, item)| (item.id.clone(), at))
        .collect();
    let found: Vec<Option<String>> = (0..items.len())
        .map(|at| {
            if items[at].topic.is_some() {
                return None;
            }
            let mut root = at;
            for _ in 0..CHAIN_MOST {
                let up = items[root]
                    .reply_to
                    .as_ref()
                    .and_then(|named| index.get(named))
                    .copied();
                match up {
                    Some(up) if items[up].topic.is_some() => return items[up].topic.clone(),
                    Some(up) => root = up,
                    None => break,
                }
            }
            let root = &items[root];
            Some(root.send_id.clone().unwrap_or_else(|| root.id.clone()))
        })
        .collect();
    for (item, topic) in items.iter_mut().zip(found) {
        if topic.is_some() {
            item.topic = topic;
        }
    }
}

/// The teammate's inbox as the window draws it: open and closed newest
/// first, what it sent newest first, and its unread count, each item with
/// its topic and send id ([`topics`]).
pub fn view<'a>(
    log: &Log,
    persona_id: &str,
    teammates: impl IntoIterator<Item = &'a str>,
) -> InboxView {
    let teammates: Vec<&str> = teammates.into_iter().collect();
    let all = everything(log, teammates.iter().copied());
    let mut open = Vec::new();
    let mut closed = Vec::new();
    let held = all
        .iter()
        .filter(|item| item.to == persona_id && !item.held)
        .cloned()
        .collect::<Vec<_>>();
    for item in held.into_iter().rev() {
        if item.state == hotline_contract::InboxState::Closed {
            closed.push(item);
        } else {
            open.push(item);
        }
    }
    let mut sent: Vec<InboxItem> = all
        .into_iter()
        .filter(|item| item.from == persona_id && item.to != persona_id && item.notice.is_none())
        .collect();
    sent.sort_by_key(|item| item.ts);
    sent.reverse();
    closed.truncate(VIEW_MOST);
    sent.truncate(VIEW_MOST);
    let unread = open.iter().filter(|item| item.state.unread()).count();
    // What a newer build saved and this one cannot read: held for this
    // teammate (but not held back for the person's card), or sent by it.
    let mut newer: Vec<InboxNewerItem> = newer_of(log, persona_id)
        .into_iter()
        .filter(|line| line["held"] != true)
        .chain(
            teammates
                .iter()
                .filter(|teammate| **teammate != persona_id)
                .flat_map(|teammate| newer_of(log, teammate))
                .filter(|line| line["from"] == persona_id && line.get("notice").is_none()),
        )
        .map(|line| {
            let text = |key: &str| line[key].as_str().unwrap_or_default().to_string();
            InboxNewerItem {
                id: text("id"),
                ts: line["ts"].as_i64().unwrap_or_default(),
                from: text("from"),
                to: text("to"),
                title: text("title"),
                state: text("state"),
            }
        })
        .collect();
    newer.sort_by_key(|item| std::cmp::Reverse(item.ts));
    InboxView {
        persona_id: persona_id.to_string(),
        open,
        closed,
        sent,
        unread: u32::try_from(unread).unwrap_or(u32::MAX),
        newer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> Log {
        let root =
            std::env::temp_dir().join(format!("hotline-store-inbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Log::open(root)
    }

    /// An item line in Bob's inbox from `from`, in `state`, sent at `ts`.
    fn written(id: &str, from: &str, state: &str, ts: i64) -> Value {
        json!({"kind": KIND, "id": id, "ts": ts, "from": from,
            "fromThread": {"kind": "dm", "key": from}, "to": "bob", "title": format!("Title of {id}"),
            "body": "", "wants": "answer", "state": state, "receipts": [{"state": state, "at": ts}],
            "senderTold": 0})
    }

    /// A field a newer build wrote inside `fromThread`, `about` or `work` is
    /// written back while the item names the same thread, and not once this
    /// build names another there.
    #[test]
    fn a_field_inside_a_thread_an_item_names_is_kept_while_it_names_that_thread() {
        let mut line = written("i1", "ada", "taken", 1);
        line["fromThread"]["lane"] = json!("quiet");
        line["about"] = json!({"kind": "side", "key": "s1", "lane": "quiet"});
        line["work"] = json!({"kind": "side", "key": "w1", "seat": 2});
        let mut item = read(&line).unwrap();
        assert_eq!(super::line(&item), line);
        item.work = Some(hotline_contract::ThreadId {
            desk: None,
            kind: hotline_contract::ThreadKind::Side,
            key: "w2".into(),
        });
        let moved = super::line(&item);
        assert_eq!(moved["work"], json!({"kind": "side", "key": "w2"}));
        assert_eq!(moved["about"]["lane"], "quiet");
        assert_eq!(moved["fromThread"]["lane"], "quiet");
    }

    /// The window is sent the newest of what closed and what was sent, and
    /// the items a newer build saved that this one cannot read: held for
    /// the teammate, or sent by it, but not one held back for the person's
    /// card or Hotline's own notice.
    #[test]
    fn the_view_caps_what_is_closed_and_sent_and_lists_what_a_newer_build_saved() {
        let log = scratch("view");
        for at in 0..(VIEW_MOST as i64 + 5) {
            log.append(
                &StreamId::Inbox("bob".into()),
                &written(&format!("c{at}"), "ada", "closed", at),
            )
            .unwrap();
        }
        let bob = StreamId::Inbox("bob".into());
        log.append(&bob, &written("parked", "ada", "parked", 900))
            .unwrap();
        let mut held = written("held", "ada", "parked", 901);
        held["held"] = json!(true);
        log.append(&bob, &held).unwrap();
        let mut sent = written("sent-parked", "bob", "parked", 902);
        sent["to"] = json!("ada");
        log.append(&StreamId::Inbox("ada".into()), &sent).unwrap();
        let mut notice = written("notice", "bob", "parked", 903);
        notice["to"] = json!("ada");
        notice["notice"] = json!("forwarded");
        log.append(&StreamId::Inbox("ada".into()), &notice).unwrap();

        let shown = view(&log, "bob", ["ada", "bob"]);
        assert_eq!(shown.closed.len(), VIEW_MOST);
        assert_eq!(
            shown.closed[0].id,
            format!("c{}", VIEW_MOST + 4),
            "newest first"
        );
        let newer: Vec<(&str, &str)> = shown
            .newer
            .iter()
            .map(|item| (item.id.as_str(), item.state.as_str()))
            .collect();
        assert_eq!(newer, [("sent-parked", "parked"), ("parked", "parked")]);
        assert_eq!(shown.newer[0].to, "ada");
    }
}
