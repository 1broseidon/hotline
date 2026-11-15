//! Loop in…: the person brings a colleague into a teammate's work thread
//! (`docs/design/thread-awareness.md`, decision 5). There are no threads of
//! three, so the colleague joins it for one answer.
//!
//! The colleague is sent an item from the thread, in its inbox like any
//! other (`session/inbox.rs`): a question from the thread's teammate with the
//! thread's last lines, which the colleague reads and answers when it sorts
//! its inbox. Its answer is an item in the thread's teammate's inbox, heard
//! in the thread while it is live, for the teammate to take on. The thread's
//! own record draws the quiet "Quill was looped in" line from a looped-in
//! aside written there. The person's act is the grant: no card asks it. The
//! pair's brake still counts the item and its answer.

use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource};
use crate::thread::ThreadState;

/// How many of the thread's last lines the colleague is shown, and the most
/// of each.
const TAIL_LINES: usize = 8;
const LINE_CHARS: usize = 800;

impl Room {
    /// Loops `persona_id` into the work thread `side_id`, for the person.
    pub async fn loop_in(self: &Arc<Self>, side_id: &str, persona_id: &str) -> Result<(), String> {
        let _working = self.working()?;
        let thread = ThreadId::side(side_id);
        let link = self
            .link_of(&thread)
            .ok_or("There is no such work thread.")?;
        if matches!(link.state, ThreadState::Closed(_)) {
            return Err("That thread has ended. Continue it to loop someone in.".into());
        }
        let owner = self.persona(link.persona_id.as_deref().unwrap_or_default())?;
        let colleague = self.persona(persona_id)?;
        if colleague.id == owner.id {
            return Err(format!("{} is this thread's own teammate.", owner.name));
        }
        if colleague.node.is_some() {
            return Err(format!(
                "{} is on another desk, and Loop in does not cross a link yet.",
                colleague.name
            ));
        }
        let person = self.person_name().unwrap_or_else(|| "The person".into());
        let title = if link.title.trim().is_empty() {
            "untitled".to_string()
        } else {
            link.title
        };
        let lines = self.log.load(&StreamId::Side(side_id.to_string()));
        let message = format!(
            "{person} looped you into {owner}'s work thread \"{title}\" for what you know. \
             Answer once, in a few lines, with update_inbox: your answer is heard in that thread, \
             and {owner} takes it on from there. Its last lines:\n\n{tail}",
            owner = owner.name,
            tail = tail(&lines, &owner.name),
        );
        let title = format!("{person} looped you into {}'s \"{title}\"", owner.name);
        self.loop_in_item(&owner, &colleague, &thread, &title, message)?;
        // The thread's own record of it, which draws the quiet line. It is
        // not kept as an aside of the colleague's: the item is what it hears.
        self.threads().write(
            &thread,
            &owner.id,
            &TranscriptEvent::Aside(Aside {
                id: new_id(),
                ts: now_ms(),
                to: colleague.id,
                from: AsideSource::Thread(thread.clone()),
                by: Some(owner.id.clone()),
                text: format!("{person} looped you in."),
                about: None,
                level: AsideLevel::Note,
                heard: None,
                seen: None,
                looped_in: true,
                source_name: None,
            }),
        );
        Ok(())
    }
}

/// The thread's last lines, each named: what the person and its teammate
/// said, and what came back to it. Nothing for a thread nobody has spoken in.
fn tail(lines: &[serde_json::Value], owner: &str) -> String {
    let said: Vec<String> = lines
        .iter()
        .filter_map(|line| {
            let text = line["text"].as_str()?.trim();
            let who = match line["kind"].as_str()? {
                "user" => "Asked".to_string(),
                "agent" => owner.to_string(),
                "delivery" => line["cause"]["name"].as_str()?.to_string(),
                _ => return None,
            };
            (!text.is_empty()).then(|| format!("{who}: {}", crate::driver::clip(text, LINE_CHARS)))
        })
        .collect();
    if said.is_empty() {
        return "(Nothing has been said in it yet.)".into();
    }
    said[said.len().saturating_sub(TAIL_LINES)..].join("\n\n")
}
