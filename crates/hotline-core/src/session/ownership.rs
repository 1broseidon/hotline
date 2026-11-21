//! The owner of work is captured at launch, not guessed when it returns.

use super::{Room, lock};
use crate::contract::{InboxItem, ThreadId};

pub(crate) type Track = std::sync::Arc<dyn Fn(Option<WorkFor>) -> WorkFor + Send + Sync>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkFor {
    #[default]
    Person,
    Colleague {
        name: String,
        item: String,
    },
}

impl WorkFor {
    pub(super) fn item(&self) -> Option<&str> {
        match self {
            Self::Colleague { item, .. } => Some(item),
            Self::Person => None,
        }
    }

    /// Different owners have no single colleague to answer for the batch.
    pub(crate) fn merge(&mut self, other: &Self) -> bool {
        if self == other {
            return false;
        }
        *self = Self::Person;
        true
    }
}

impl Room {
    pub(super) fn colleague_work(&self, item: &InboxItem) -> WorkFor {
        if item.notice.is_some() || item.reply_to.is_some() {
            return WorkFor::Person;
        }
        WorkFor::Colleague {
            name: self.sender_of(item),
            item: item.id.clone(),
        }
    }

    pub(crate) fn work_for(&self, thread: &ThreadId) -> WorkFor {
        lock(&self.posting)
            .get(thread)
            .map_or_else(WorkFor::default, |turn| turn.work_for.clone())
    }

    pub(crate) fn set_work_for(&self, thread: &ThreadId, owner: WorkFor) {
        let private_work = owner.item().is_some();
        {
            let mut posting = lock(&self.posting);
            let Some(turn) = posting.get_mut(thread) else {
                return;
            };
            turn.work_for = owner;
            turn.answered_for = None;
        }
        if thread.kind == crate::contract::ThreadKind::Dm {
            let session = lock(&self.sessions).get(&thread.key).cloned();
            if let Some(session) = session {
                self.inbox_activity(&session, private_work);
            }
        }
    }

    /// Outcomes said just after an answer still belong to the work they describe.
    pub(super) fn result_item(&self, thread: &ThreadId) -> Option<String> {
        let turns = lock(&self.posting);
        let turn = turns.get(thread)?;
        turn.answered_for
            .as_ref()
            .unwrap_or(&turn.work_for)
            .item()
            .map(str::to_owned)
    }

    pub(super) fn finish_work_for(&self, thread: &ThreadId, ended: &str) {
        if let Some(turn) = lock(&self.posting).get_mut(thread)
            && matches!(&turn.work_for, WorkFor::Colleague { item, .. } if item == ended)
        {
            turn.answered_for = Some(std::mem::take(&mut turn.work_for));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::{DeskKeys, scratch};
    use serde_json::Value;
    use std::sync::Arc;

    pub(super) fn slice() -> Vec<Value> {
        include_str!("tests/receiver-slice.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn an_operator_line_does_not_change_the_owner_captured_by_a_helper() {
        let room = Room::new(scratch("receiver-owner"), Arc::new(DeskKeys));
        let dm = ThreadId::dm("ada");
        let rows = slice();
        let mut helpers = std::collections::HashMap::new();
        let mut captured = WorkFor::Person;
        let mut turn = room.open_posting(&dm, None);
        for row in &rows {
            match row["kind"].as_str().unwrap() {
                "inbox_sorted" if row["entries"][0]["state"] == "taken" => {
                    captured = WorkFor::Colleague {
                        item: row["entries"][0]["item"].as_str().unwrap().into(),
                        name: "Pork Chop".into(),
                    };
                    room.set_work_for(&dm, captured.clone());
                }
                "link" if row["state"] == "live" => {
                    helpers.insert(row["thread"].as_str().unwrap(), room.work_for(&dm));
                }
                "link" if row["state"] == "closed" => {
                    drop(turn);
                    turn = room.open_posting(&dm, None);
                    let launch = &helpers[row["thread"].as_str().unwrap()];
                    room.set_work_for(&dm, launch.clone());
                    assert_eq!(room.work_for(&dm), captured);
                }
                "user" => {
                    room.set_work_for(&dm, WorkFor::Person);
                    assert_eq!(room.work_for(&dm), WorkFor::Person);
                    assert!(helpers.values().all(|owner| owner == &captured));
                }
                _ => {}
            }
        }
        assert_eq!(helpers.len(), 3);
    }
}
