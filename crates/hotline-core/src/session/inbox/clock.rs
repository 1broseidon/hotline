//! The inbox's clock: what is set aside is offered again on its own time,
//! and what nobody picks up expires (`docs/design/agent-inbox.md`).
//!
//! - **Offered again.** An item a pick-up offered that is still set aside
//!   (`pick_up.rs`) carries when it is offered next (`offer_again`): an hour
//!   after its first pick-up ([`again_after_first`]), then a day after each
//!   ([`again_every`]). When that time has come and the main conversation is
//!   idle with nothing queued, one quiet pick-up turn offers every item that
//!   is due. A busy conversation is offered it at the clock's first look
//!   once it is idle again: a turn's end never offers an item a pick-up has
//!   offered before. The times are the items' own, so no turn arms the next
//!   and a pick-up's turn still arms nothing.
//! - **Expired.** An item left open (unread, read and unsorted, or `later`)
//!   for [`expire_after`] since it arrived closes by itself as `expired`,
//!   with its receipt; a `taken` or held one never does. Its sender gets one
//!   item back, Hotline's notice (`notice: expired`, a reply to it that
//!   wants nothing and gives the receiver's last reason for later), which
//!   wakes an idle sender quietly as any item does. The receiver is not
//!   woken: it finds the item among its closed ones. A notice that expires
//!   sends none.
//! - **When it looks.** Once when the desk opens ([`Room::inbox_at_start`],
//!   after the room has handed on what it resumes): what expired while the
//!   desk was closed closes first, then each teammate is offered what it set
//!   aside once (the pick-up at start, which also counts as the offer due),
//!   and from then on every [`LOOK_EVERY_MS`].
//!
//! A unit test's room uses shorter times, and a test reads the clock at a
//! time it chooses ([`Room::inbox_due`]).

use super::{Room, kept, moved, now_ms, title_of};
use crate::contract::{InboxItem, InboxNotice, InboxOutcome, InboxState, InboxWants, ThreadId};
use std::sync::Arc;
use std::time::Duration;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// After an item's first pick-up, how long until it is offered again.
const AGAIN_AFTER_FIRST_MS: i64 = if cfg!(test) { 60_000 } else { HOUR_MS };

/// After each later offer, how long until the next.
const AGAIN_EVERY_MS: i64 = if cfg!(test) { 120_000 } else { DAY_MS };

/// How long an item may stay open since it arrived before it expires.
const EXPIRE_AFTER_MS: i64 = if cfg!(test) { 300_000 } else { 3 * DAY_MS };

/// How often the clock looks: fine enough for an hour, cheap enough to
/// read every inbox each time.
const LOOK_EVERY_MS: u64 = if cfg!(test) { 1_000 } else { 30_000 };

/// After an item's first pick-up, how long until it is offered again: an
/// hour.
pub(super) fn again_after_first() -> i64 {
    AGAIN_AFTER_FIRST_MS
}

/// After each later offer, how long until the next: a day.
pub(super) fn again_every() -> i64 {
    AGAIN_EVERY_MS
}

/// How long an item may stay open since it arrived: three days.
pub(super) fn expire_after() -> i64 {
    EXPIRE_AFTER_MS
}

/// A span as a notice says it: "3 days", "1 hour", "45 minutes".
fn span_words(ms: i64) -> String {
    let (count, unit) = if ms >= DAY_MS && ms % DAY_MS == 0 {
        (ms / DAY_MS, "day")
    } else if ms >= HOUR_MS && ms % HOUR_MS == 0 {
        (ms / HOUR_MS, "hour")
    } else if ms >= 60_000 {
        (ms / 60_000, "minute")
    } else {
        (ms / 1_000, "second")
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// When an item arrived: when it was let in (its last `new`), else sent.
fn arrived_at(item: &InboxItem) -> i64 {
    item.receipts
        .iter()
        .rev()
        .find(|receipt| receipt.state == InboxState::New)
        .map_or(item.ts, |receipt| receipt.at)
}

/// Whether an item is open in a way that expires: not held, not taken, not
/// closed.
fn may_expire(item: &InboxItem) -> bool {
    !item.held
        && matches!(
            item.state,
            InboxState::New | InboxState::Noticed | InboxState::Seen | InboxState::Later
        )
}

/// The last reason the receiver gave for leaving an item for later.
fn last_later_reason(item: &InboxItem) -> Option<String> {
    item.receipts
        .iter()
        .rev()
        .find(|receipt| receipt.state == InboxState::Later)
        .and_then(|receipt| receipt.reason.clone())
        .filter(|reason| !reason.trim().is_empty())
}

impl Room {
    /// The desk has opened and handed on what it resumes: what expired while
    /// it was closed closes, each teammate is offered what it set aside once,
    /// and the clock starts. Once per room.
    pub(in crate::session) fn inbox_at_start(self: &Arc<Self>) {
        if self
            .inbox_started
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        self.expire_overdue(now_ms());
        self.pick_up_at_start();
        let room = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(LOOK_EVERY_MS)).await;
                let Some(room) = room.upgrade() else {
                    return;
                };
                if !room.closing() {
                    room.inbox_due(now_ms()).await;
                }
            }
        });
    }

    /// The clock read at `now`: what has expired closes, and each teammate
    /// is offered, in one quiet turn, what of its set aside has come due.
    pub(in crate::session) async fn inbox_due(self: &Arc<Self>, now: i64) {
        self.inbox_tidy(now, false);
        self.expire_overdue(now);
        for persona in crate::room::roster(&self.log) {
            self.offer_again(&persona.id, now).await;
        }
    }

    /// Closes as `expired` every item open longer than [`expire_after`]
    /// since it arrived, and tells each one's sender with Hotline's notice.
    fn expire_overdue(self: &Arc<Self>, now: i64) {
        let after = expire_after();
        for persona in crate::room::roster(&self.log) {
            for item in kept::received(&self.log, &persona.id) {
                if !may_expire(&item) || arrived_at(&item) + after > now {
                    continue;
                }
                let reason = format!("Not picked up in {}.", span_words(after));
                let expired = self.change_item(&persona.id, &item.id, |kept| {
                    if !may_expire(kept) {
                        return false;
                    }
                    moved(
                        kept,
                        InboxState::Closed,
                        Some(reason),
                        Some(InboxOutcome::Expired),
                        None,
                    );
                    true
                });
                // A notice that expires sends none: no notice of a notice.
                if let Some(expired) = expired
                    && expired.notice.is_none()
                    && let Err(error) = self.notify_expired(&expired, after)
                {
                    eprintln!("an expired item's sender could not be told: {error}");
                }
            }
        }
    }

    /// Hotline's notice to an expired item's sender: an item in its inbox
    /// that answers the one that expired, wants nothing, and is marked as
    /// the desk's (`notice`), from the receiver it is about. It is no
    /// teammate's message, so it counts on no brake.
    fn notify_expired(self: &Arc<Self>, expired: &InboxItem, after: i64) -> Result<(), String> {
        if self.persona(&expired.from).is_err() {
            return Ok(());
        }
        let receiver = self.persona(&expired.to)?;
        let name = self.name_of(&receiver.id);
        // The title and the reason are teammates' words inside Hotline's:
        // quoted, so neither can end its quote and speak as the notice.
        let mut body = format!(
            "This is Hotline's notice, not {name}'s words. Your item {} to {name} was not picked up and expired after {}: Hotline closed it unanswered.",
            crate::fence::quoted(&expired.title),
            span_words(after)
        );
        let because = last_later_reason(expired).map_or_else(String::new, |reason| {
            format!(
                " {name}'s last reason for leaving it for later: {}.",
                crate::fence::quoted(&reason)
            )
        });
        let tell = if expired.for_person {
            format!(
                " You sent it for something the person asked of you: tell them, in one line with `post_to_chat`, that {name} never picked it up."
            )
        } else {
            " Tell the person only if it is theirs to know.".to_string()
        };
        body.push_str(&because);
        body.push_str(" Nothing more comes of it unless you send it again.");
        body.push_str(&tell);
        let mut notice = super::item(
            &receiver,
            &ThreadId::dm(&receiver.id),
            &expired.from,
            title_of(&format!("Expired: {}", expired.title))?,
            body,
            InboxWants::Nothing,
        );
        notice.notice = Some(InboxNotice::Expired);
        notice.reply_to = Some(expired.id.clone());
        notice.about = self.reply_about(expired);
        moved(&mut notice, InboxState::New, None, None, None);
        notice.sender_told = 1;
        self.keep_item(&notice)?;
        self.arrived(&notice);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_says_a_span_in_the_largest_whole_unit() {
        assert_eq!(span_words(3 * DAY_MS), "3 days");
        assert_eq!(span_words(HOUR_MS), "1 hour");
        assert_eq!(span_words(300_000), "5 minutes");
    }
}
