//! Teammates the person has linked for shared work.
//!
//! A message between two teammates normally goes to a side session: the
//! colleague answers with none of its own conversation, and nothing enters
//! either main conversation. That is right for a quick question and wrong
//! for work two teammates share, where the one answering needs everything it
//! knows and both need to remember what was agreed. So the person can link
//! two teammates, and while they are linked, `message_teammate` between them
//! is delivered into the other's own conversation as a
//! [`TranscriptEvent::Delivery`], and the thread records it for the person to
//! read.
//!
//! Only the person links (`teammates.link`, from a seat; there is no tool),
//! because a link puts words into another teammate's main conversation, the
//! same trust step as a first contact. A link ends when the person unlinks
//! the pair or either teammate's chapter closes, so a later casual message
//! goes to a side session again. Two linked teammates that go back and forth
//! [`LINK_CAP`] times without the person hearing from either pause, and the
//! person is asked whether they should keep going.
//!
//! Unlinking or a pause never recalls a message already delivered: it is on
//! the recipient's tape, and it will be heard. Only the next send changes.

use super::{Room, new_id, now_ms, peers};
use crate::contract::{DeliveryCause, LinkPauseStatus, Persona, Receipt, TranscriptEvent};
use crate::log::{StreamId, thread};
use crate::paths::{thread_key, thread_participants};
use crate::room::{self, Link};
use serde_json::Value;
use std::sync::Arc;

/// How many messages two linked teammates may send each other without the
/// person before the link pauses. Six each way is a conversation; more than
/// that unattended is more often a loop.
pub(super) const LINK_CAP: i64 = 12;

impl Room {
    /// Links two teammates. Linking a linked pair changes nothing.
    pub fn link_teammates(&self, a: &str, b: &str) -> Result<(), String> {
        let (a, b, key) = self.pair(a, b)?;
        if room::links(&self.log).iter().any(|link| link.id == key) {
            return Ok(());
        }
        room::append_link(
            &self.log,
            &Link {
                id: key,
                a: a.id,
                b: b.id,
                since: now_ms(),
                exchanges: 0,
                paused: false,
            },
        )
    }

    /// Ends a link, and settles the card a pause left on either tape.
    pub fn unlink_teammates(&self, a: &str, b: &str) -> Result<(), String> {
        let (a, b, key) = self.pair(a, b)?;
        if !room::links(&self.log).iter().any(|link| link.id == key) {
            return Ok(());
        }
        room::tombstone_link(&self.log, &key)?;
        self.settle_pause(&a, &b, LinkPauseStatus::Unlinked);
        Ok(())
    }

    /// Resumes a paused link, counting afresh. The person's word is the
    /// only thing that resumes one: a reconnect or a restart never does.
    pub fn resume_link(&self, a: &str, b: &str) -> Result<(), String> {
        let (a, b, key) = self.pair(a, b)?;
        let Some(link) = room::links(&self.log)
            .into_iter()
            .find(|link| link.id == key)
        else {
            return Err(format!("{} and {} are not linked.", a.name, b.name));
        };
        if link.paused {
            room::append_link(
                &self.log,
                &Link {
                    exchanges: 0,
                    paused: false,
                    ..link
                },
            )?;
        }
        self.settle_pause(&a, &b, LinkPauseStatus::Resumed);
        Ok(())
    }

    /// The link between two teammates, if there is one.
    pub(super) fn link_between(&self, a: &str, b: &str) -> Option<Link> {
        let key = thread_key(a, b)?;
        room::links(&self.log)
            .into_iter()
            .find(|link| link.id == key)
    }

    /// The person spoke to this teammate: its links count afresh.
    pub(super) fn heard_from_person(&self, persona_id: &str) {
        for link in room::links(&self.log) {
            if link.other(persona_id).is_some() && link.exchanges > 0 && !link.paused {
                let _ = room::append_link(
                    &self.log,
                    &Link {
                        exchanges: 0,
                        ..link
                    },
                );
            }
        }
    }

    /// A chapter closed: this teammate's links end with it.
    pub(super) fn end_links_of(&self, persona_id: &str) {
        for link in room::links(&self.log) {
            let Some(other) = link.other(persona_id) else {
                continue;
            };
            if room::tombstone_link(&self.log, &link.id).is_err() {
                continue;
            }
            if let (Ok(one), Ok(other)) = (self.persona(persona_id), self.persona(other)) {
                self.settle_pause(&one, &other, LinkPauseStatus::Unlinked);
            }
        }
    }

    /// Counts one message against a link, under the room's one lock for
    /// links, so two sends at once cannot count as one. Refused when the
    /// link is paused: nothing is sent then. `true` when this message is the
    /// one that reached the cap: it still goes, and the link pauses behind
    /// it.
    pub(super) fn count_linked(&self, target: &Persona, key: &str) -> Result<(Link, bool), String> {
        let _counting = super::lock(&self.link_counts);
        let link = room::links(&self.log)
            .into_iter()
            .find(|link| link.id == key)
            .ok_or_else(|| format!("You are no longer linked with {}.", target.name))?;
        if link.paused {
            return Err(paused_refusal(target, &link));
        }
        let exchanges = link.exchanges + 1;
        let counted = Link {
            exchanges,
            paused: exchanges >= LINK_CAP,
            ..link
        };
        room::append_link(&self.log, &counted)?;
        let reached = counted.paused;
        Ok((counted, reached))
    }

    /// A counted message between linked teammates: into the thread for the
    /// person to read, and into the recipient's own conversation as a
    /// delivery, and if it reached the cap, the pause behind it.
    pub(super) async fn send_linked(
        self: &Arc<Self>,
        caller: &Persona,
        target: &Persona,
        link: Link,
        reached: bool,
        message: &str,
    ) -> Result<(), String> {
        let key = link.id.clone();
        thread::ensure(self.log.root(), &key)
            .map_err(|error| format!("That thread could not be opened: {error}"))?;
        self.write_linked_line(&key, caller, message);
        let cause = DeliveryCause::Linked {
            persona_id: caller.id.clone(),
            name: caller.name.clone(),
            thread_key: key,
            about: peers::about(message),
        };
        let delivered = self
            .deliver_into(&target.id, cause, message.to_string())
            .await;
        if reached {
            self.pause(caller, target, link.exchanges);
        }
        delivered
    }

    /// The two teammates a seat named, and their thread's key.
    fn pair(&self, a: &str, b: &str) -> Result<(Persona, Persona, String), String> {
        let a = self.persona(a)?;
        let b = self.persona(b)?;
        if a.id == b.id {
            return Err("A teammate cannot be linked with itself.".to_string());
        }
        let key = thread_key(&a.id, &b.id)
            .ok_or_else(|| "Those two teammates cannot share a thread.".to_string())?;
        Ok((a, b, key))
    }

    /// The message on the thread, on the sender's side of it, so the person
    /// reads a linked exchange where they read any other.
    fn write_linked_line(&self, key: &str, caller: &Persona, message: &str) {
        let id = new_id();
        let ts = now_ms();
        let text = message.to_string();
        let event = match thread_participants(key) {
            Some((user_side, _)) if user_side == caller.id => TranscriptEvent::User {
                id,
                ts,
                text,
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: Some(Receipt::Sent),
            },
            _ => TranscriptEvent::Agent {
                id,
                ts,
                text,
                attachments: None,
                reactions: None,
                ring: None,
                receipt: Some(Receipt::Sent),
            },
        };
        let Ok(value) = serde_json::to_value(&event) else {
            return;
        };
        if let Err(error) = self.log.append(&StreamId::Thread(key.to_string()), &value) {
            eprintln!("the thread {key} could not be appended to: {error}");
        }
    }

    /// The cap was reached: a card on both tapes, and the person told.
    fn pause(&self, caller: &Persona, target: &Persona, exchanges: i64) {
        let ts = now_ms();
        for (whose, other) in [(caller, target), (target, caller)] {
            self.write(
                &whose.id,
                &TranscriptEvent::LinkPaused {
                    id: format!("{}{ts}", pause_prefix(&whose.id, &other.id)),
                    ts,
                    with_persona_id: other.id.clone(),
                    with_name: other.name.clone(),
                    exchanges,
                    status: LinkPauseStatus::Pending,
                },
            );
        }
        self.push.notify(
            &format!("{} and {} paused", caller.name, target.name),
            &format!(
                "They went back and forth {exchanges} times without you. Keep them going, or unlink them."
            ),
            &caller.id,
            None,
        );
    }

    /// Supersedes a pending pause card on either tape with what the person
    /// decided.
    fn settle_pause(&self, a: &Persona, b: &Persona, status: LinkPauseStatus) {
        for (whose, other) in [(a, b), (b, a)] {
            let prefix = pause_prefix(&whose.id, &other.id);
            let pending: Vec<Value> = self
                .tape(&whose.id)
                .into_iter()
                .filter(|event| {
                    event
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id.starts_with(&prefix))
                        && event.get("status").and_then(Value::as_str) == Some("pending")
                })
                .collect();
            for card in pending {
                let Ok(TranscriptEvent::LinkPaused {
                    id,
                    with_persona_id,
                    with_name,
                    exchanges,
                    ..
                }) = serde_json::from_value::<TranscriptEvent>(card)
                else {
                    continue;
                };
                self.write(
                    &whose.id,
                    &TranscriptEvent::LinkPaused {
                        id,
                        ts: now_ms(),
                        with_persona_id,
                        with_name,
                        exchanges,
                        status,
                    },
                );
            }
        }
    }
}

/// What a teammate is told when it messages a partner its link has paused.
pub(super) fn paused_refusal(target: &Persona, link: &Link) -> String {
    format!(
        "Your link with {} is paused: the two of you went back and forth {} times without \
         the person, and they have been asked whether you should keep going. Nothing was \
         sent. Tell the person where things stand, or wait for them.",
        target.name, link.exchanges
    )
}

/// The start of every pause card's id for this pair on this tape. Each pause
/// is a card of its own, so a second one lands where it happened rather
/// than where the first one was.
fn pause_prefix(whose: &str, other: &str) -> String {
    format!("link-paused:{whose}:{other}:")
}
