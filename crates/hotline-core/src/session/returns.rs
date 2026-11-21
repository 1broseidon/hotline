//! The line Hotline adds when work comes back to a teammate nobody is
//! waiting on.
//!
//! The house style says how to take a return ([`super::pacing::HOUSE_STYLE`]),
//! but it is read once, at the top of a long context, and a long job puts a
//! great deal between it and the moment a helper's report lands. That moment
//! is when the teammate decides whether to speak, and Hotline knows things
//! then that no standing text can: that the turn began by itself, how many
//! messages the teammate has sent since the person last wrote, and whether
//! the person last wrote from their phone. So one short line of Hotline's own
//! says those facts, then the rule. The turn it is said in is quiet (its
//! words reach nobody, `session/dm.rs`), so the rule is said in the
//! teammate's terms: what it writes there is not read, and `post_to_chat`,
//! once, is how the person hears that the job is done, a decision is theirs
//! or something failed. It describes the moment and quotes nothing for the
//! model to copy; its words vary only with the facts.
//!
//! It goes where nobody waits on a reply, and only there:
//!
//! - a line of Hotline's that begins a turn ([`super::Wired::back`]): a
//!   command left running that ended ([`crate::driver::rig::process_ended`]),
//!   a line that only wakes an idle teammate for its asides or its inbox, a
//!   pick-up of what it set aside, a thread's post that asks the main
//!   conversation to look (`dm.rs`), and a work thread's wake (`sides.rs`);
//! - the DM taking up updates from an activity the harness began by itself
//!   (`dm.rs`): a non-interrupting note if the driver can admit it, never a
//!   new prompt; and
//! - a helper's or a command's result Hotline Agent hands a turn after the
//!   teammate had replied and stopped to wait for its helpers, or hands a
//!   turn none of those began (`driver/rig/turn.rs`).
//!
//! A result that comes back while the teammate works on the person's own
//! request gets nothing: that turn is already a reply, and the line would
//! only repeat itself every round. Neither does a line of the person's or a
//! colleague's, which somebody is waiting on.
//!
//! The person's own answer to something the teammate gave them is the
//! person writing, not work that came back. A card's answer is a delivery,
//! like their line, and gets nothing. An answer that comes back through a
//! command (marks on a page the teammate is waiting on) Hotline cannot tell
//! from any other command's end, so the line goes with it and names that
//! case: in a quiet turn the acknowledgement is a reaction, which reaches the
//! chat where words would not, as the finished command's line and the house
//! style say.

use super::{Room, StreamId};
use crate::contract::{ThreadId, ThreadKind};
use serde_json::Value;
use std::fmt::Write;
use std::sync::Arc;

/// The line for a return to the teammate whose thread this is, read off the
/// thread when the return lands.
pub(crate) type Note = Arc<dyn Fn() -> String + Send + Sync>;

/// The words, for a teammate that has `sent` messages to the person since
/// they last wrote, who last wrote from their phone when `phone`. The turn
/// they are said in is quiet ([`super::dm`]), so they say so in the
/// teammate's terms: its words there do not reach the chat, and
/// `post_to_chat`, once, is how the person hears that the job is done, a
/// decision is theirs or something failed.
pub(crate) fn note(sent: usize, phone: bool) -> String {
    let mut line = String::from(
        "Hotline: this came back by itself, so this turn is quiet: what you write in it does not reach the person's chat.",
    );
    if phone {
        line.push_str(" The person is on their phone.");
    }
    let who = if phone { "they" } else { "the person" };
    let _ = match sent {
        0 => Ok(()),
        1 => write!(line, " You have sent 1 message since {who} last wrote."),
        sent => write!(
            line,
            " You have sent {sent} messages since {who} last wrote."
        ),
    };
    line.push_str(
        " To reach them, post once with post_to_chat: that the job they gave you is done and what they have now, a decision that is theirs, or a failure that matters to them. Otherwise write nothing. If this is their own answer to you, react to show you have it, then work.",
    );
    line
}

/// How many messages the teammate has sent since the person's last line in
/// `events`, and whether that line came from their phone. A message is a
/// bubble the person sees: each `agent` event.
fn since_the_person(events: &[Value]) -> (usize, bool) {
    let last = events.iter().rposition(|event| event["kind"] == "user");
    let sent = events[last.map_or(0, |at| at + 1)..]
        .iter()
        .filter(|event| event["kind"] == "agent")
        .count();
    let phone = last.is_some_and(|at| events[at]["client"] == "phone");
    (sent, phone)
}

impl Room {
    /// The line for work that came back to `persona_id` in `thread`, with the
    /// facts as that thread stands now.
    pub(crate) fn return_note(&self, persona_id: &str, thread: &ThreadId) -> String {
        let stream = match thread.kind {
            ThreadKind::Dm => Some(StreamId::Tape(persona_id.to_string())),
            _ => StreamId::of_thread(thread),
        };
        let events = stream
            .map(|stream| self.log.load(&stream))
            .unwrap_or_default();
        let (sent, phone) = since_the_person(&events);
        note(sent, phone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Short, Hotline's own, and only the facts that hold: no count when the
    /// teammate has said nothing since, no phone when the person is not on it.
    #[test]
    fn the_note_is_short_and_says_only_the_facts_that_hold() {
        let fullest = note(12, true);
        assert!(fullest.split_whitespace().count() < 90, "{fullest}");
        assert!(fullest.starts_with(
            "Hotline: this came back by itself, so this turn is quiet: what you write in it does not reach the person's chat."
        ));
        assert!(fullest.contains(
            "The person is on their phone. You have sent 12 messages since they last wrote."
        ));
        assert!(fullest.contains(
            "To reach them, post once with post_to_chat: that the job they gave you is done and what they have now, a decision that is theirs, or a failure that matters to them. Otherwise write nothing."
        ));
        assert!(fullest.ends_with(
            "If this is their own answer to you, react to show you have it, then work."
        ));
        let quiet = note(0, false);
        assert!(!quiet.contains("sent"), "{quiet}");
        assert!(!quiet.contains("phone"), "{quiet}");
        assert!(note(1, false).contains("You have sent 1 message since the person last wrote."));
    }

    /// The count is of bubbles since the person's last line, and the phone is
    /// that line's.
    #[test]
    fn the_facts_are_read_from_the_person_s_last_line() {
        let events = [
            json!({"kind": "user", "text": "first", "client": "phone"}),
            json!({"kind": "agent", "text": "a"}),
            json!({"kind": "user", "text": "second", "client": "desktop"}),
            json!({"kind": "agent", "text": "b"}),
            json!({"kind": "outcome", "message": "made"}),
            json!({"kind": "agent", "text": "c"}),
            json!({"kind": "turn"}),
        ];
        assert_eq!(since_the_person(&events), (2, false));
        assert_eq!(since_the_person(&events[..2]), (1, true));
        assert_eq!(since_the_person(&[]), (0, false));
    }

    /// The room reads the teammate's own conversation for them.
    #[tokio::test]
    async fn the_room_reads_the_facts_off_the_conversation() {
        let log = super::super::tests::scratch("return-note");
        for event in [
            json!({"kind": "user", "id": "u1", "ts": 1, "text": "build it", "client": "phone"}),
            json!({"kind": "agent", "id": "a1", "ts": 2, "text": "On it."}),
            json!({"kind": "agent", "id": "a2", "ts": 3, "text": "Done."}),
        ] {
            log.append(&StreamId::Tape("ada".into()), &event).unwrap();
        }
        let room = Room::new(log, Arc::new(super::super::tests::DeskKeys));
        assert_eq!(room.return_note("ada", &ThreadId::dm("ada")), note(2, true));
    }
}
