//! Where a turn's words go, from its starter and the owner of its work.
use super::{Room, StreamId, ownership::WorkFor};
use crate::contract::{ThreadId, ThreadKind};
use serde_json::Value;
use std::{fmt::Write, sync::Arc};

pub(crate) type Note = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone, Copy)]
pub(crate) enum Starter<'a> {
    Person,
    Helpers,
    Command,
    Thread,
    Aside,
    Inbox,
    Quick,
    Answers(&'a str),
    Job { quiet: bool, final_run: bool },
    Desk,
}

pub(crate) fn opening(starter: Starter<'_>, owner: &WorkFor) -> String {
    let start = match starter {
        Starter::Person => return "Your reply is the chat.".into(),
        Starter::Desk => return "Hotline: this is a desk notice. There is nothing to answer or post.".into(),
        Starter::Answers(who) => return format!("Hotline: {who} answered. This work is for the person. They read this turn: tell them the result in your own words."),
        Starter::Job { final_run: true, .. } => return "Hotline: this is the job's final report. Your last words are posted; do not post them again.".into(),
        Starter::Job { quiet: true, .. } => return "Hotline: this job is for the person. Post once only if there is something worth their attention.".into(),
        Starter::Job { .. } => return "Hotline: this job is for the person. Your last words are its report unless you post one yourself.".into(),
        Starter::Helpers => "your helpers came back",
        Starter::Command => "your command came back",
        Starter::Thread => "a thread of yours reported back",
        Starter::Aside => "an aside arrived",
        Starter::Inbox => "your inbox woke you",
        Starter::Quick => "a colleague sent you a quick message",
    };
    match owner {
        WorkFor::Person => format!(
            "Hotline: {start}. This work is for the person. Your reply does not reach their chat. Post once if their work is done, a decision is theirs, or a failure matters to them."
        ),
        WorkFor::Colleague { name, item } => {
            let item = super::inbox::short(item);
            // A quick line is answered with one `send`, as the index says
            // under it. Anything else is answered by `update_inbox`, which
            // closes the item: a `send` replying to work is a question about
            // it and leaves it open, so the asker would hear the result as a
            // stray message and again when the item closed.
            let (answer, post) = if matches!(starter, Starter::Quick) {
                ("answer the item", " Nothing goes to the person's chat.")
            } else {
                (
                    "when it is done, give it as the item's `answer` in `update_inbox`, which closes it",
                    " Post to the person only for a decision that is theirs or a problem only they can fix.",
                )
            };
            format!(
                "Hotline: {start}. This work is for {name} (item {item}), not the person. Your result goes to {name}: {answer}.{post}"
            )
        }
    }
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
    pub(crate) fn return_note(&self, persona_id: &str, thread: &ThreadId) -> String {
        self.opening_note(persona_id, thread, Starter::Helpers, &self.work_for(thread))
    }

    pub(super) fn opening_note(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        starter: Starter<'_>,
        owner: &WorkFor,
    ) -> String {
        let mut line = opening(starter, owner);
        if !matches!(
            starter,
            Starter::Helpers | Starter::Thread | Starter::Command
        ) {
            return line;
        }
        let stream = if thread.kind == ThreadKind::Dm {
            Some(StreamId::Tape(persona_id.to_string()))
        } else {
            StreamId::of_thread(thread)
        };
        let events = stream
            .map(|stream| self.log.load(&stream))
            .unwrap_or_default();
        let (sent, phone) = since_the_person(&events);
        if phone {
            line.push_str(" The person is on their phone.");
        }
        if sent != 0 {
            let _ = write!(
                line,
                " You have sent {sent} message{} since the person last wrote.",
                if sent == 1 { "" } else { "s" }
            );
        }
        if matches!(starter, Starter::Command) {
            line.push_str(
                " If this is their own answer to you, react to show you have it, then work.",
            );
        }
        line
    }
}

#[cfg(test)]
mod tests;
