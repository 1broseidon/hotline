//! A failed wake does not end the item. Hotline lets its sender know without
//! making the receiver speak or counting another message between teammates.

use super::{Room, item, lock, moved, now_ms, title_of};
use crate::contract::{
    DeliveryFrom, InboxItem, InboxNotice, InboxState, InboxWakeFailure, InboxWakeFailureState,
    InboxWants, ThreadId, ThreadKind, TranscriptEvent,
};
use crate::log::StreamId;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// A short id prefix, like other inbox items, also needs to distinguish
/// notices. Do not use an id made of a common prefix plus the original id.
fn notice_id(original: &str, state: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("inbox-wake-failure:{state}:{original}"))
    )
}

impl Room {
    pub(in crate::session) fn notify_inbox_wake_failure(
        self: &Arc<Self>,
        original: &InboxItem,
        failure: &InboxWakeFailure,
    ) -> Result<(), String> {
        if original.notice.is_some()
            || original.held
            || !original.state.unread()
            || self.persona(&original.from).is_err()
        {
            return Ok(());
        }
        let receiver = self.persona(&original.to)?;
        let name = crate::fence::one_line(&self.name_of(&receiver.id));
        // The first word in each pair is a permanent part of the notice id.
        let (state, reason) = match failure.state {
            InboxWakeFailureState::OutOfQuota => ("outOfQuota", "the agent is out of quota"),
            InboxWakeFailureState::NotSignedIn => ("notSignedIn", "the agent is not signed in"),
            InboxWakeFailureState::Stopped => ("stopped", "the teammate was stopped"),
            InboxWakeFailureState::CouldNotStart => ("couldNotStart", "the agent could not start"),
            InboxWakeFailureState::Other => ("other", "the agent could not run the inbox turn"),
        };
        let reset = failure
            .reset_at
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map_or_else(String::new, |at| {
                format!(" The quota resets at {}.", at.format("%Y-%m-%d %H:%M UTC"))
            });
        let id = notice_id(&original.id, state);
        let inbox = StreamId::Inbox(original.from.clone());
        // The chat post has its own key and is made only once per item, even
        // if a later wake fails for a different reason. Neither write is a
        // receipt on the original item.
        let saved = {
            let _writing = lock(&self.inbox_lock);
            if original.for_person {
                let id = format!("inbox-wake-failure-post:{}", original.id);
                if !self
                    .log
                    .contains(&StreamId::Tape(original.from.clone()), &id)
                {
                    let text = format!(
                        "Hotline: I couldn't wake {name} for your request: {reason}. The inbox item is still waiting.{reset}"
                    );
                    let from = &original.from_thread;
                    // A normal agent post can attach the unrelated inbox item
                    // that woke the current turn. The desk has no such source.
                    self.write(
                        &original.from,
                        &TranscriptEvent::Agent {
                            id,
                            ts: now_ms(),
                            text,
                            attachments: None,
                            reactions: None,
                            handoff_thread: None,
                            ring: None,
                            receipt: None,
                            spoken: None,
                            silent: Some(true),
                            from: (from.kind != ThreadKind::Dm)
                                .then(|| DeliveryFrom::new(from, None)),
                            scheduled: None,
                            from_item: None,
                        },
                    );
                }
            }
            if self.log.contains(&inbox, &id) {
                return Ok(());
            }
            let title = title_of(&format!("Could not wake: {}", original.title))?;
            let body = format!(
                "This is Hotline's notice. I couldn't wake {name} for your item {}: {reason}. The item is still in their inbox, unread.{reset} Nothing was closed or needs to be sent again.",
                crate::fence::quoted(&original.title)
            );
            let mut notice = item(
                &receiver,
                &ThreadId::dm(&receiver.id),
                &original.from,
                title,
                body,
                InboxWants::Nothing,
            );
            notice.id = id;
            notice.notice = Some(InboxNotice::WakeFailed);
            notice.wake_failure = Some(failure.clone());
            notice.reply_to = Some(original.id.clone());
            notice.topic = Some(self.topic_for(original));
            notice.about = self.reply_about(original);
            moved(&mut notice, InboxState::New, None, None, None);
            notice.sender_told = 1;
            self.keep_item(&notice)?;
            notice
        };
        self.arrived(&saved);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::notice_id;

    #[test]
    fn failed_wakes_have_stable_keys_distinguished_by_item_and_state() {
        let id = notice_id("original", "outOfQuota");
        assert_eq!(id, notice_id("original", "outOfQuota"));
        assert_ne!(id[..8], notice_id("original", "notSignedIn")[..8]);
        assert_ne!(id[..8], notice_id("different", "outOfQuota")[..8]);
    }
}
