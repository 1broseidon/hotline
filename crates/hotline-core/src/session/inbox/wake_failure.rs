//! A prompt offered is not a message read. Only the end of a turn can
//! confirm that the whole message reached an agent that actually ran.

use super::{Room, clock, kept, moved, now_ms};
use crate::contract::{
    InboxItem, InboxOutcome, InboxState, InboxWakeFailure, InboxWakeFailureState as State,
    InboxWants, ThreadKind,
};
use crate::session::{exchanges::Cut, runner::Driven};
use serde_json::Value;
use std::sync::Arc;

/// A quota announcement can arrive as speech or thought, but only has
/// quota semantics when paired with a terminal refusal.
pub(in crate::session) fn quota_announcement(text: &str) -> Option<&str> {
    let after_heading = text.trim_start().strip_prefix("Usage Limit Reached")?;
    let after_heading = after_heading.strip_prefix('.').unwrap_or(after_heading);
    if !after_heading.starts_with(char::is_whitespace) {
        return None;
    }
    let tail = after_heading
        .trim_start()
        .strip_prefix("You have reached your current quota for this period.")?
        .trim();
    (tail.is_empty() || tail.starts_with("Your limit will reset in ")).then_some(tail)
}

fn quota_reset(tail: &str, now: i64) -> Option<i64> {
    let delay = tail
        .strip_prefix("Your limit will reset in ")?
        .trim_end_matches('.');
    let mut seconds = 0_i64;
    for part in delay.split(',') {
        let (n, unit) = part.trim().split_once(' ')?;
        let n = n.parse::<i64>().ok().filter(|n| *n > 0)?;
        let factor = match unit {
            "hour" | "hours" => 3600,
            "minute" | "minutes" => 60,
            "second" | "seconds" => 1,
            _ => return None,
        };
        seconds = seconds.checked_add(n.checked_mul(factor)?)?;
    }
    now.checked_add(seconds.checked_mul(1000)?)
}

fn failure(state: State) -> InboxWakeFailure {
    InboxWakeFailure {
        state,
        reset_at: None,
        detail: None,
    }
}

fn diagnostic(text: &str, default: State) -> InboxWakeFailure {
    let typed = text
        .rsplit_once("\n\n")
        .and_then(|(_, json)| serde_json::from_str::<Value>(json).ok());
    let known = typed.as_ref().map(|v| &v["hotlineFailure"]);
    let mut cause = failure(match known.and_then(|v| v["kind"].as_str()) {
        Some("quota" | "rate_limit" | "budget") => State::OutOfQuota,
        Some("auth" | "agent_auth") => State::NotSignedIn,
        _ => default,
    });
    if cause.state == State::OutOfQuota {
        cause.reset_at = known
            .and_then(|v| v["retryAfterSeconds"].as_i64())
            .filter(|secs| *secs > 0)
            .and_then(|secs| now_ms().checked_add(secs.checked_mul(1000)?));
    } else if cause.state == State::Other {
        // The provider line is data, not an instruction or a dump of an
        // opaque provider payload. Structured errors already have a summary.
        let line = text.lines().next().unwrap_or_default();
        let line = super::short_line(&crate::driver::failure::redact_text(line), 240);
        cause.detail = (!line.is_empty()).then_some(line);
    }
    cause
}

pub(super) fn fresh_at(item: &InboxItem, now: i64) -> bool {
    item.state.unread() && item.offered.is_none() && item.offer_again.is_none_or(|at| at <= now)
}

impl Room {
    /// Persists the retry clock before notifying. Reading or closing the
    /// item through a tool wins, even when the surrounding turn later fails.
    fn defer_inbox_wake(
        self: &Arc<Self>,
        receiver: &str,
        id: &str,
        cause: Option<&InboxWakeFailure>,
    ) {
        let now = now_ms();
        let deferred = self.change_item(receiver, id, |item| {
            if item.held || !item.state.unread() {
                return false;
            }
            let delay = if item.offer_again.is_some() {
                clock::again_every()
            } else {
                clock::again_after_first()
            };
            item.offered = None;
            item.offer_again = Some(cause.and_then(|cause| cause.reset_at).map_or_else(
                || now.saturating_add(delay),
                |reset| reset.max(now.saturating_add(30_000)),
            ));
            true
        });
        if let (Some(item), Some(cause)) = (deferred, cause)
            && let Err(error) = self.notify_inbox_wake_failure(&item, cause)
        {
            eprintln!("a failed inbox wake could not be reported: {error}");
        }
    }

    pub(in crate::session) fn defer_inbox_start(
        self: &Arc<Self>,
        receiver: &str,
        waiting: &[InboxItem],
        error: &str,
    ) {
        let cause = diagnostic(error, State::CouldNotStart);
        for item in waiting {
            self.defer_inbox_wake(receiver, &item.id, Some(&cause));
        }
    }

    pub(in crate::session) fn settle_inbox_offer(
        self: &Arc<Self>,
        receiver: &str,
        offered: &[String],
        inline: Option<&str>,
        driven: &Driven,
        cut: Option<Cut>,
    ) -> bool {
        if offered.is_empty() {
            return false;
        }
        let ran = driven.model_progress
            || (driven.failure.is_none()
                && !driven.cancelled
                && matches!(
                    driven.stop_reason.as_deref(),
                    Some("end_turn" | "max_tokens" | "max_turn_requests" | "tool_use")
                ));
        if ran {
            for id in offered {
                let shown = inline == Some(id.as_str());
                let answer = shown && self.answers_own_ask(receiver, id);
                self.change_item(receiver, id, |item| {
                    let changed = item.offer_again.take().is_some();
                    if !shown || item.state != InboxState::New {
                        return changed;
                    }
                    // Shown whole, an item asking nothing back was told no
                    // sorting is needed, and an answer to the teammate's own
                    // question was what that turn told: either is done with,
                    // not left to be offered again in a quiet turn, whose
                    // line would invite telling the person a second time.
                    if item.wants == InboxWants::Nothing || answer {
                        moved(
                            item,
                            InboxState::Closed,
                            None,
                            Some(InboxOutcome::Noted),
                            None,
                        );
                    } else {
                        moved(item, InboxState::Seen, None, None, None);
                    }
                    true
                });
            }
            return false;
        }
        let cause = match cut.filter(|_| crate::session::exchanges::cut_short(driven)) {
            Some(Cut::Stop) => Some(failure(State::Stopped)),
            Some(Cut::Interrupt) => None,
            None if driven.quota_refusal.is_some() => {
                let mut cause = failure(State::OutOfQuota);
                cause.reset_at = driven
                    .quota_refusal
                    .as_deref()
                    .and_then(|tail| quota_reset(tail, now_ms()));
                Some(cause)
            }
            None if crate::session::exchanges::cut_short(driven)
                || driven.stop_reason.as_deref() == Some("revoked") =>
            {
                None
            }
            None => Some(driven.failure.as_deref().map_or_else(
                || failure(State::Other),
                |text| diagnostic(text, State::Other),
            )),
        };
        for id in offered {
            self.defer_inbox_wake(receiver, id, cause.as_ref());
        }
        true
    }

    /// Whether `id` in `receiver`'s inbox answers a question `receiver`
    /// asked, now closed: the reply its ask was waiting for.
    fn answers_own_ask(&self, receiver: &str, id: &str) -> bool {
        let Ok(item) = self.item_of(receiver, id) else {
            return false;
        };
        item.reply_to.as_deref().is_some_and(|asked| {
            self.item_of(&item.from, asked)
                .is_ok_and(|asked| asked.from == receiver && asked.state == InboxState::Closed)
        })
    }

    /// Only the ordinary clock re-arms a failed wake, never the end of the
    /// turn that failed it. Explicit reads can still open the item at once.
    pub(super) fn retry_due_inbox_wakes(self: &Arc<Self>, receiver: &str, now: i64) {
        if self.quiet_held(receiver).is_some() {
            return;
        }
        for item in kept::received(&self.log, receiver) {
            if !super::on_call::is_call_result(&item)
                && fresh_at(&item, now)
                && item.offer_again.is_some()
            {
                let thread = self.inbox_heard_in(&item);
                if thread.kind != ThreadKind::Dm
                    || self.session(receiver).is_ok()
                    || self.info(receiver).error.is_some()
                {
                    self.gather(receiver, thread);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_reset_uses_only_the_adapters_known_countdown() {
        assert_eq!(
            quota_reset("Your limit will reset in 1 hour, 16 minutes.", 10),
            Some(4_560_010)
        );
        assert_eq!(quota_reset("Your limit will reset in 1 day.", 0), None);
        assert_eq!(
            quota_reset("Your limit will reset in 999999999999999999 hours.", 0),
            None
        );
        assert_eq!(quota_announcement("I saw Usage Limit Reached."), None);
        assert_eq!(
            quota_announcement("Usage Limit Reached\n\nAnother reason."),
            None
        );
        assert_eq!(
            quota_announcement(
                "Usage Limit Reached. You have reached your current quota for this period. Your limit will reset in 1 minute."
            ),
            Some("Your limit will reset in 1 minute.")
        );
    }
}
