//! Inbound sources as the room uses them: the one door a transport will
//! deliver through, and a teammate's proposal of a source, which only the
//! person's yes adds (`docs/design/thread-awareness.md`, "Inbound sources").
//! The record, its fold and its checks are `hotline_room::inbound`.
//!
//! Nothing outside Hotline reaches a teammate yet: no webhook endpoint,
//! mailbox or GitHub poller is built. When one is, it calls
//! [`Room::inbound_aside`] and nothing else, so it needs no permission of its
//! own: the source's grant is the whole of what it may do.

use super::asking::{Approves, Question};
use super::exchanges::EXCHANGE_CAP;
use super::*;
use crate::contract::{Aside, AsideLevel, AsideSource, InboundDraft, InboundSource};
use hotline_room::inbound;
use std::path::PathBuf;

/// The longest proposal card the person reads whole, as a new teammate's:
/// [`Room::ask_card`] clips there, and a source approved from a clipped card
/// is not one the person saw.
const CARD_MAX: usize = 500;

/// One event a transport hands the room.
pub(crate) struct Delivery<'a> {
    /// The teammate it is for, by id.
    pub to: &'a str,
    pub text: &'a str,
    /// A thread of the teammate's, as `kind:key`.
    pub about: Option<&'a str>,
    /// The level the transport asks for. The source's highest caps it.
    pub level: AsideLevel,
    /// Files the transport received with it, where it staged them.
    pub files: &'a [PathBuf],
}

impl Room {
    /// Delivers one event from an inbound source as an aside, only as far as
    /// the person's grant goes: a source on this desk that is not muted, a
    /// teammate it names, its level lowered to the source's highest, within
    /// its hourly rate, and files only where it allows them. The words are
    /// one line with no control characters and no token in them, kept with
    /// the source's name, and heard fenced as untrusted data from outside
    /// Hotline ([`super::asides::render`]). A nudge that wakes its teammate
    /// counts on the source's brake, which mutes it at a pair's cap until
    /// the person unmutes it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the door a transport delivers through, and no transport is built yet (task-326)"
        )
    )]
    pub(crate) fn inbound_aside(
        self: &Arc<Self>,
        source_id: &str,
        delivery: &Delivery<'_>,
    ) -> Result<Aside, String> {
        let _working = self.working()?;
        let aside = {
            // Counting what was delivered this hour and keeping one more is
            // one step, so two deliveries at once cannot both fit the last
            // place.
            let _counting = lock(&self.asides_lock);
            let source = self.deliverable(source_id, delivery.to)?;
            let text = clip(&inbound::untrusted_text(delivery.text), ASIDE_CHARS);
            if text.is_empty() {
                return Err("An inbound aside needs text.".into());
            }
            if !delivery.files.is_empty() {
                if !source.files_allowed {
                    return Err(format!("{} may not carry files.", source.name));
                }
                self.inbound_files(&source, delivery.to, delivery.files)?;
            }
            let about = match delivery
                .about
                .map(str::trim)
                .filter(|about| !about.is_empty())
            {
                None => None,
                Some(named) => Some(
                    self.thread_of(delivery.to, named)
                        .ok_or_else(|| format!("{named} is not a thread of {}'s.", delivery.to))?,
                ),
            };
            let ts = now_ms();
            let delivered =
                inbound::delivered_since(&self.log, &source.id, ts - inbound::RATE_WINDOW_MS);
            if delivered >= source.rate as usize {
                return Err(format!(
                    "{} has left its {} asides for this hour.",
                    source.name, source.rate
                ));
            }
            let aside = Aside {
                id: new_id(),
                ts,
                to: delivery.to.to_string(),
                from: AsideSource::Inbound(source.id.clone()),
                by: None,
                text,
                about,
                level: delivery.level.min(source.max_level),
                heard: None,
                seen: None,
                looped_in: false,
                source_name: Some(source.name),
            };
            self.keep_aside(&aside)?;
            aside
        };
        if self.act_on_aside(&aside)? {
            self.count_inbound_wake(source_id);
        }
        Ok(aside)
    }

    /// The source, when it may deliver to `to` now: on this desk, not muted,
    /// naming `to`, who is a teammate here.
    fn deliverable(&self, source_id: &str, to: &str) -> Result<InboundSource, String> {
        let source =
            inbound::source(&self.log, source_id).ok_or_else(|| inbound::missing(source_id))?;
        if source.muted {
            return Err(format!("{} is muted, so it delivers nothing.", source.name));
        }
        if !source.teammates.iter().any(|id| id == to) {
            return Err(format!("{} may not aside {to}.", source.name));
        }
        self.persona(to)?;
        Ok(source)
    }

    /// Files a source that allows them carries into its teammate's workspace.
    ///
    /// TODO(task-325): they go through the size cap and the `inbox/` path
    /// rules phase 8 sets for files between agents, called from here once it
    /// has merged, and come back as the paths the aside names. Until then
    /// none is taken, even from a source that allows them.
    fn inbound_files(
        &self,
        source: &InboundSource,
        _to: &str,
        _files: &[PathBuf],
    ) -> Result<(), String> {
        Err(format!(
            "{} may carry files, but files from outside Hotline are not taken yet.",
            source.name
        ))
    }

    /// Counts a nudge's wake on its source's brake, and mutes the source at
    /// a pair's cap: from there it delivers nothing until the person unmutes
    /// it, as a pair waits for their Keep going.
    fn count_inbound_wake(&self, source_id: &str) {
        let _counting = lock(&self.asides_lock);
        let Some(mut source) = inbound::source(&self.log, source_id) else {
            return;
        };
        source.wakes += 1;
        source.muted = source.muted || i64::from(source.wakes) >= EXCHANGE_CAP;
        if let Err(error) = inbound::write(&self.log, &source) {
            eprintln!("count a wake on inbound source {source_id}: {error}");
        }
    }

    /// The `propose_inbound` tool: a card showing the source exactly as it
    /// would be added, and nothing written until the person says yes
    /// ([`Room::settle_inbound_source`]). A teammate names who it may aside
    /// by name, itself when it names nobody, and never a secret.
    pub(crate) fn propose_inbound(
        &self,
        persona_id: &str,
        mut draft: InboundDraft,
        home: &ThreadId,
        capability: CapabilityLease,
    ) -> Result<String, String> {
        capability.check()?;
        let mut teammates = Vec::new();
        for named in &draft.teammates {
            let teammate = self.teammate_named(named)?;
            if teammate.node.is_some() {
                return Err(format!(
                    "{} is on another desk: a source here asides teammates on this desk.",
                    teammate.name
                ));
            }
            teammates.push(teammate.id);
        }
        if teammates.is_empty() {
            teammates.push(persona_id.to_string());
        }
        draft.teammates = teammates;
        draft.secret = None;
        let source = inbound::drafted(&self.log, new_id(), &draft, now_ms())?;
        let names: Vec<String> = source
            .teammates
            .iter()
            .map(|id| self.persona(id).map_or_else(|_| id.clone(), |p| p.name))
            .collect();
        let reason = format!(
            "Add the inbound source \"{}\" ({})? It may leave {} asides, at most {} ({} an hour), {}. Nothing reaches Hotline through it until a transport for it exists.",
            source.name,
            source.kind.name(),
            names.join(", "),
            source.max_level.name(),
            source.rate,
            if source.files_allowed {
                "with files"
            } else {
                "no files"
            },
        );
        if reason.chars().count() > CARD_MAX {
            return Err("That card would be too long to show whole: shorten the name.".into());
        }
        let asked = self.ask_card(
            persona_id,
            &reason,
            home,
            None,
            Some(Question {
                capability,
                approves: Some(Approves::InboundSource(Box::new(source))),
            }),
        )?;
        Ok(format!(
            "{asked} The source does not exist until the person says yes, so do not say it does."
        ))
    }

    /// Called under the card-answer lock, so a second answer or a withdrawal
    /// cannot race the write. The source written is the one the card showed,
    /// stamped now, once its teammates are checked again: one removed since
    /// cannot be granted.
    pub(super) fn settle_inbound_source(
        &self,
        action_id: &str,
        status: HumanActionStatus,
        expires_at: Option<i64>,
    ) -> Result<(), String> {
        if status != HumanActionStatus::Done {
            return Ok(());
        }
        let questions = lock(&self.voice_questions);
        let question = questions
            .get(action_id)
            .ok_or("This source approval no longer has a live session. Ask again.")?;
        let Some(Approves::InboundSource(source)) = &question.approves else {
            return Err("This card cannot add an inbound source.".into());
        };
        question.capability.check()?;
        if expires_at.is_none_or(|expires| expires <= now_ms()) {
            return Err("This source approval has expired. Ask again.".into());
        }
        inbound::checked_teammates(&self.log, &source.teammates)?;
        let source = InboundSource {
            created_at: now_ms(),
            ..(**source).clone()
        };
        inbound::write(&self.log, &source)
    }
}

#[cfg(test)]
mod tests;
