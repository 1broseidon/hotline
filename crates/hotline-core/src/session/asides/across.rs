//! Asides across a link (`docs/design/thread-awareness.md` §3).
//!
//! An aside to a teammate on a linked desk goes there as the link's `aside`
//! delivery, and that desk decides it as it would a local colleague's, under
//! its own grant: the linked sender's `allowedSenders` entry or a grant the
//! person gave it there for this session, the same an ask from it would
//! pass. It never raises a first-contact card there: an aside asks nobody
//! anything, and a desk must not be able to fill the other's person's view
//! with cards by leaving asides. One that rides on a handoff, a work
//! thread's note to the teammate that handed it over, needs no grant there
//! and is resolved in that desk's own record (`exchanges/across.rs`).
//!
//! It is sent only to a desk whose roster said it takes asides, and only
//! while the channel is open: an older desk keeps every message and answer
//! and only loses asides, and the sender is told plainly that nothing was
//! left.

use super::*;
use crate::contract::{LinkedDelivery, Persona};
use crate::linked::{self, CallError, Record};
use crate::session::files_between;
use serde_json::{Value, json};

impl Room {
    /// The `aside` tool for a colleague on a linked desk: sent there with
    /// its files, and kept on the thread it was said in as sent.
    pub(super) async fn aside_across(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        said_in: &ThreadId,
        leaving: &Leaving<'_>,
        files: Vec<(String, Vec<u8>)>,
        capability: Option<CapabilityLease>,
    ) -> Result<String, String> {
        let text = leaving.text.trim();
        let (desk_id, persona_id) =
            linked::split(&target.id).ok_or("That teammate is not on a linked desk.")?;
        let name = crate::session::links::shown(target);
        match self.links.takes(desk_id, linked::ASIDES) {
            None => {
                return Err(format!(
                    "{name}'s desk is not connected now, so nothing was left. Try again later, or use message_teammate, which waits for it."
                ));
            }
            Some(false) => {
                return Err(format!(
                    "{name}'s desk is older and does not take asides, so nothing was left. Use message_teammate."
                ));
            }
            Some(true) => {}
        }
        let about = match leaving.about.map(str::trim).filter(|about| !about.is_empty()) {
            None => None,
            Some(named) => Some(
                ThreadId::parse(named)
                    .filter(|thread| thread.desk.as_deref().is_none_or(|desk| desk == desk_id))
                    .map(|thread| ThreadId::new(thread.kind, thread.key))
                    .ok_or_else(|| {
                        format!(
                            "{named} does not name a thread on {name}'s desk. Name one as kind:key, or leave `about` out."
                        )
                    })?,
            ),
        };
        if let Some(capability) = &capability {
            capability.check()?;
        }
        let aside = Aside {
            id: new_id(),
            ts: now_ms(),
            to: target.id.clone(),
            from: AsideSource::Thread(said_in.clone()),
            by: Some(caller.id.clone()),
            text: text.to_string(),
            about: about.clone().map(|thread| thread.on_desk(desk_id)),
            level: leaving.level,
            heard: None,
            seen: None,
            looped_in: false,
            source_name: None,
        };
        let delivery = LinkedDelivery::Aside {
            id: aside.id.clone(),
            from_persona_id: caller.id.clone(),
            to: persona_id.to_string(),
            from: said_in.clone(),
            text: text.to_string(),
            level: leaving.level,
            about,
            request_id: None,
            files: files_between::to_link(files),
        };
        let answer = match self
            .links
            .call(
                desk_id,
                "federation.deliver",
                json!({ "delivery": delivery }),
            )
            .await
        {
            Ok(answer) => answer,
            Err(CallError::Offline) => {
                return Err(format!(
                    "{name}'s desk did not answer, so the aside may not have been left. Try again later, or use message_teammate."
                ));
            }
            Err(CallError::Refused(error)) => return Err(error),
        };
        // The sender's record shows what it said, as for a local colleague.
        self.threads()
            .write(said_in, &caller.id, &TranscriptEvent::Aside(aside.clone()));
        Ok(left(&name, aside.level, answer["woke"] == true))
    }

    /// A linked desk's aside for one of this desk's teammates. Its sender
    /// must be on the roster that desk published here; one that rides on a
    /// handoff this desk sent is heard by that handoff's sender, and any
    /// other passes the grant an ask from its sender would, without a card.
    /// Its files land in the recipient's working directory as a local
    /// colleague's do. The same aside sent again is the one already kept.
    pub(in crate::session) fn receive_linked_aside(
        self: &Arc<Self>,
        record: &Record,
        delivery: LinkedDelivery,
    ) -> Result<Value, String> {
        let LinkedDelivery::Aside {
            id,
            from_persona_id,
            to,
            from,
            text,
            level,
            about,
            request_id,
            files,
        } = delivery
        else {
            return Err("That is not an aside.".into());
        };
        if !linked::valid_persona_id(&id) || !linked::valid_persona_id(&from_persona_id) {
            return Err("That aside does not say plainly who left it.".into());
        }
        let by = linked::qualified(&record.desk_id, &from_persona_id);
        let caller = self.participant(&by).map_err(|_| {
            "That teammate is not on the roster its desk published here.".to_string()
        })?;
        let text = text.trim();
        if text.is_empty() || text.chars().count() > ASIDE_CHARS {
            return Err(format!("An aside is 1 to {ASIDE_CHARS} characters."));
        }
        let mut aside = Aside {
            // Its id is the other desk's: kept under that desk's name, so no
            // desk can reach an aside of this one's by its id.
            id: format!("{}:{id}", record.desk_id),
            ts: now_ms(),
            to: String::new(),
            from: AsideSource::Thread(ThreadId::new(from.kind, from.key).on_desk(&record.desk_id)),
            by: Some(by.clone()),
            text: text.to_string(),
            about: None,
            level,
            heard: None,
            seen: None,
            looped_in: false,
            source_name: None,
        };
        if let Some(request_id) = request_id {
            if !files.is_empty() || about.is_some() {
                return Err("A note on a handoff carries only its words.".into());
            }
            if !linked::valid_persona_id(&request_id) {
                return Err("That note does not say plainly which handoff it is on.".into());
            }
            return self
                .receive_linked_note(&record.desk_id, &request_id, &by, aside)
                .map(|()| Value::Null);
        }
        let target = self
            .persona(&to)
            .map_err(|_| "There is no such teammate on this desk.".to_string())?;
        if self.aside_kept(&target.id, &aside.id) {
            return Ok(json!({"woke": false}));
        }
        let files = files_between::from_link(&files)?;
        self.capability_lease(&target.id).check()?;
        if !self.collaboration_granted(&caller, &target, false) {
            return Err(format!(
                "{} has not been allowed to ask {} here, so the aside was not left. message_teammate asks the person there first.",
                crate::session::links::shown(&caller),
                target.name
            ));
        }
        aside.to.clone_from(&target.id);
        aside.about = match about {
            None => None,
            Some(named) => Some(
                self.thread_of(
                    &target.id,
                    &ThreadId::new(named.kind, named.key.clone()).to_string(),
                )
                .ok_or_else(|| format!("{named} is not a thread of {}'s.", target.name))?,
            ),
        };
        if !files.is_empty() {
            let paths = self.deliver_files(&by, &target.id, &files)?;
            aside.text = format!(
                "{}\n\n{}",
                aside.text,
                files_between::files_line(&crate::session::links::shown(&caller), &paths)
            );
        }
        let woke = self.leave_aside(&aside)?;
        Ok(json!({"woke": woke}))
    }
}
