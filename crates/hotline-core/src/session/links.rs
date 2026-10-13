//! Teammates on linked desks, as this room meets them (ADR 0003).
//!
//! A teammate on a linked desk is never in this room's roster. It is a
//! participant: a persona made up on the spot from what the link knows, with
//! its `node` set, its id node-qualified (`deskId/personaId`) and nothing
//! granted, so every rule that reads a persona reads one that has no reach,
//! no tools and no standing grant of its own here.
//!
//! Messages cross in the exchange queue that local messages use. On the
//! sender's desk a request to a linked teammate is forwarded over the link
//! when it is first taken up, and waits as running until the answer comes
//! back the same way; on the recipient's desk it is queued like any other,
//! under the recipient's `allowedSenders` and the person's first-contact card,
//! and its result is sent back instead of being delivered to a tape.

use super::exchanges::Intent;
use super::{Room, TEAMMATE_MESSAGE_MAX, lock};
use crate::contract::Persona;
use crate::linked::{self, CallError, Known, LinkedDelivery, LinkedTeammate, Record};
use crate::room;
use serde_json::{Value, json};
use std::sync::Arc;

/// The most of an answer that crosses back over a link.
const REPLY_MAX: usize = 200_000;

/// What a linked teammate is shown as wherever it is not obvious that it
/// lives elsewhere: its name and its desk's.
pub(super) fn shown(persona: &Persona) -> String {
    match &persona.node {
        Some(node) => format!("{} on {}", persona.name, node.name),
        None => persona.name.clone(),
    }
}

/// A teammate on a linked desk, as a persona with nothing granted.
fn linked_persona(record: &Record, persona_id: &str, name: &str) -> Persona {
    serde_json::from_value(json!({
        "node": {"id": record.desk_id, "name": record.name},
        "id": linked::qualified(&record.desk_id, persona_id),
        "name": name,
        "goal": "",
        "backendId": "",
        "cwd": "",
        "mcpPolicy": {"mode": "none", "serverIds": []},
        "sessionCheckpoints": [],
        "createdAt": 0,
        "updatedAt": 0,
    }))
    .expect("a linked teammate is a persona with nothing granted")
}

impl Room {
    pub(crate) fn links(&self) -> Arc<linked::Links> {
        self.links.clone()
    }

    /// Every teammate on every linked desk, live where the link is open and
    /// as last heard of where it is not, each with its desk's record.
    fn linked_teammates(&self) -> Vec<(Record, LinkedTeammate)> {
        let mut teammates = Vec::new();
        for record in linked::records(&self.log) {
            let rows = self.links.roster(&record.desk_id).unwrap_or_else(|| {
                record
                    .teammates
                    .iter()
                    .map(|known| LinkedTeammate {
                        persona_id: known.persona_id.clone(),
                        name: known.name.clone(),
                        state: "offline".into(),
                        activity: None,
                        working_on: linked::WorkingOn {
                            title: String::new(),
                            last_turn_at: None,
                        },
                    })
                    .collect()
            });
            teammates.extend(rows.into_iter().map(|row| (record.clone(), row)));
        }
        teammates
    }

    /// `list_teammates`'s rows for teammates on linked desks: a local row's
    /// fields, the id node-qualified, and the desk's name as one more field.
    pub(crate) fn linked_rows(&self) -> Vec<Value> {
        self.linked_teammates()
            .into_iter()
            .map(|(record, mut row)| {
                row.persona_id = linked::qualified(&record.desk_id, &row.persona_id);
                let mut row = json!(row);
                row["desk"] = json!(record.name);
                row
            })
            .collect()
    }

    /// This desk's teammates, as a linked desk is shown them.
    pub(crate) fn federation_roster_entries(&self) -> Vec<LinkedTeammate> {
        room::roster(&self.log)
            .into_iter()
            .map(|persona| crate::mcp::server::roster_row(self, persona))
            .collect()
    }

    /// A linked desk published its teammates. Their ids and names are kept
    /// on the link's record when they change, so a message can still be
    /// addressed to one while that desk is away.
    pub(crate) fn federation_roster(
        &self,
        desk_id: &str,
        teammates: Vec<LinkedTeammate>,
    ) -> Result<(), String> {
        let mut record =
            linked::record(&self.log, desk_id).ok_or("This desk is not linked with that one.")?;
        let teammates = linked::plausible(teammates);
        let known: Vec<Known> = teammates
            .iter()
            .map(|teammate| Known {
                persona_id: teammate.persona_id.clone(),
                name: teammate.name.clone(),
            })
            .collect();
        if record.teammates != known {
            record.teammates = known;
            linked::save(&self.log, &record)?;
        }
        self.links.set_roster(desk_id, teammates);
        Ok(())
    }

    /// One side of an exchange: a teammate of this room, or one on a linked
    /// desk.
    pub(super) fn participant(&self, id: &str) -> Result<Persona, String> {
        let Some((desk_id, persona_id)) = linked::split(id) else {
            return self.persona(id);
        };
        let record = linked::record(&self.log, desk_id)
            .ok_or("That teammate's desk is not linked with this one any more.")?;
        let name = self
            .links
            .roster(desk_id)
            .and_then(|roster| {
                roster
                    .into_iter()
                    .find(|teammate| teammate.persona_id == persona_id)
                    .map(|teammate| teammate.name)
            })
            .or_else(|| {
                record
                    .teammates
                    .iter()
                    .find(|known| known.persona_id == persona_id)
                    .map(|known| known.name.clone())
            })
            .ok_or_else(|| format!("There is no such teammate on {}.", record.name))?;
        Ok(linked_persona(&record, persona_id, &name))
    }

    /// The teammate a message is addressed to: by id, by name, or as
    /// "Name on Desk" for one on a linked desk. A name two teammates share,
    /// here or on linked desks, is refused with every choice listed. A local
    /// teammate never wins a tie silently.
    pub(super) fn teammate_named(&self, to: &str) -> Result<Persona, String> {
        let wanted = to.trim();
        let local = room::roster(&self.log);
        if let Some(found) = local.iter().find(|persona| persona.id == wanted) {
            return Ok(found.clone());
        }
        if linked::is_linked(wanted) {
            return self.participant(wanted);
        }
        let elsewhere: Vec<Persona> = self
            .linked_teammates()
            .into_iter()
            .map(|(record, row)| linked_persona(&record, &row.persona_id, &row.name))
            .collect();
        let on_desk = wanted.rsplit_once(" on ");
        let mut named: Vec<Persona> = local
            .into_iter()
            .chain(elsewhere)
            .filter(|persona| {
                persona.name.eq_ignore_ascii_case(wanted)
                    || on_desk.is_some_and(|(name, desk)| {
                        persona.name.eq_ignore_ascii_case(name.trim())
                            && persona
                                .node
                                .as_ref()
                                .is_some_and(|node| node.name.eq_ignore_ascii_case(desk.trim()))
                    })
            })
            .collect();
        named.dedup_by(|a, b| a.id == b.id);
        match named.len() {
            1 => Ok(named.remove(0)),
            0 => Err(format!(
                "There is no teammate called '{wanted}' in this room. list_teammates says who is here."
            )),
            _ => {
                let choices: Vec<String> = named
                    .iter()
                    .map(|persona| match &persona.node {
                        Some(_) => format!("{} (personaId {})", shown(persona), persona.id),
                        None => format!("{} on this desk (personaId {})", persona.name, persona.id),
                    })
                    .collect();
                Err(format!(
                    "{} teammates are called '{wanted}': {}. Name the one you mean by its personaId, \
                     or as Name on Desk for one on a linked desk.",
                    named.len(),
                    choices.join("; ")
                ))
            }
        }
    }

    /// A linked desk's frame for the exchange queue: a request for one of
    /// this desk's teammates, or the answer to one of theirs.
    pub(crate) fn federation_deliver(
        self: &Arc<Self>,
        desk_id: &str,
        delivery: LinkedDelivery,
    ) -> Result<Value, String> {
        let record =
            linked::record(&self.log, desk_id).ok_or("This desk is not linked with that one.")?;
        match delivery {
            LinkedDelivery::Request {
                request_id,
                from_persona_id,
                from_name,
                to,
                message,
                intent,
            } => self.receive_linked_request(
                &record,
                &request_id,
                &from_persona_id,
                &from_name,
                &to,
                &message,
                &intent,
            ),
            LinkedDelivery::Result {
                request_id,
                reply,
                failed,
            } => self.receive_linked_result(desk_id, &request_id, reply, failed),
        }
    }

    /// Queues a linked teammate's message for one of this desk's, exactly as
    /// a local colleague's would be: the recipient's grants and the person's
    /// card decide whether it is worked on, here, when it is taken up.
    #[allow(clippy::too_many_arguments)]
    fn receive_linked_request(
        self: &Arc<Self>,
        record: &Record,
        request_id: &str,
        from_persona_id: &str,
        from_name: &str,
        to: &str,
        message: &str,
        intent: &str,
    ) -> Result<Value, String> {
        let intent = match intent {
            "ask" => Intent::Ask,
            "handoff" => Intent::Handoff,
            _ => return Err("intent must be ask or handoff".into()),
        };
        let from_name = from_name.trim();
        if !linked::valid_persona_id(request_id)
            || !linked::valid_persona_id(from_persona_id)
            || from_name.is_empty()
            || from_name.chars().count() > 100
        {
            return Err("That message does not say plainly who sent it.".into());
        }
        let message = message.trim();
        if message.is_empty() || message.chars().count() > TEAMMATE_MESSAGE_MAX {
            return Err(format!(
                "A message to a teammate is 1 to {TEAMMATE_MESSAGE_MAX} characters."
            ));
        }
        let target = self
            .persona(to)
            .map_err(|_| "There is no such teammate on this desk.".to_string())?;
        // The sender's desk vouches for its own teammate's name; it is kept
        // so the request can be shown and answered before that desk has
        // published its roster here.
        if !record
            .teammates
            .iter()
            .any(|known| known.persona_id == from_persona_id)
        {
            let mut record = record.clone();
            record.teammates.push(Known {
                persona_id: from_persona_id.to_string(),
                name: from_name.chars().take(100).collect(),
            });
            linked::save(&self.log, &record)?;
        }
        let from = linked::qualified(&record.desk_id, from_persona_id);
        self.accept_linked_request(request_id, &from, &target.id, message, intent)?;
        Ok(json!({"accepted": true}))
    }

    /// The answer to a request this desk forwarded, saved so the queue
    /// delivers it as it would a local colleague's. An answer that arrives
    /// twice, or after the person stopped the exchange, changes nothing.
    fn receive_linked_result(
        self: &Arc<Self>,
        desk_id: &str,
        request_id: &str,
        reply: String,
        failed: bool,
    ) -> Result<Value, String> {
        let reply: String = reply.chars().take(REPLY_MAX).collect();
        self.settle_linked_request(desk_id, request_id, reply, failed)?;
        Ok(Value::Null)
    }

    /// Sends a request this desk's teammate made to a linked teammate. Not
    /// yet sent while the link is down, so it waits queued; refused, it
    /// fails with the other desk's sentence.
    pub(super) async fn forward_linked(
        &self,
        request_id: &str,
        from: &str,
        to: &str,
        message: &str,
        intent: Intent,
    ) -> Result<bool, String> {
        let (desk_id, persona_id) =
            linked::split(to).ok_or("That teammate is not on a linked desk.")?;
        let caller = self.persona(from)?;
        let delivery = LinkedDelivery::Request {
            request_id: request_id.to_string(),
            from_persona_id: caller.id,
            from_name: caller.name,
            to: persona_id.to_string(),
            message: message.to_string(),
            intent: match intent {
                Intent::Ask => "ask",
                Intent::Handoff => "handoff",
            }
            .to_string(),
        };
        match self
            .links
            .call(
                desk_id,
                "federation.deliver",
                json!({ "delivery": delivery }),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(CallError::Offline) => Ok(false),
            Err(CallError::Refused(error)) => Err(error),
        }
    }

    /// Sends the answer to a linked teammate's request back over the link.
    /// False while the link is down: the answer waits, saved, and goes when
    /// it is up again.
    pub(super) async fn return_linked(
        &self,
        request_id: &str,
        from: &str,
        reply: &str,
        failed: bool,
    ) -> bool {
        let Some((desk_id, _)) = linked::split(from) else {
            return true;
        };
        let delivery = LinkedDelivery::Result {
            request_id: request_id.to_string(),
            reply: reply.to_string(),
            failed,
        };
        match self
            .links
            .call(
                desk_id,
                "federation.deliver",
                json!({ "delivery": delivery }),
            )
            .await
        {
            Ok(_) => true,
            Err(CallError::Offline) => false,
            // The other desk has nothing waiting for it any more: the
            // person there stopped it, or the link was made again since.
            Err(CallError::Refused(error)) => {
                eprintln!("[link] an answer for {from} was refused: {error}");
                true
            }
        }
    }

    /// Ends a link on this desk, whichever side ended it. The record goes
    /// and with it every linked teammate of that desk; messages waiting to
    /// or from them fail with a sentence; approvals waiting for them are
    /// withdrawn, and work running for them is stopped.
    pub(crate) async fn unlinked(self: &Arc<Self>, desk_id: &str) -> Result<(), String> {
        let record = linked::record(&self.log, desk_id);
        // While the record is here the teammates of that desk still have
        // names, for the sentences their senders are told.
        let name = record
            .as_ref()
            .map_or("another desk", |record| record.name.as_str());
        self.stop_linked_exchanges(desk_id, name);
        if record.is_some() {
            linked::remove(&self.log, desk_id)?;
        }
        self.links.forget(desk_id);
        // Every lease a teammate of that desk was holding here — a
        // collaboration card's, a session grant's, a peer session's — ends.
        let theirs: Vec<String> = lock(&self.capabilities)
            .keys()
            .filter(|id| linked::split(id).is_some_and(|(desk, _)| desk == desk_id))
            .cloned()
            .collect();
        for id in theirs {
            self.capability_epoch(&id).invalidate();
            self.peers.invalidate(&id);
        }
        self.settle_invalid_collaboration();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
