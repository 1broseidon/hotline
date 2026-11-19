//! A `send` whose item needs the person's first-contact card
//! (`docs/design/agent-inbox.md`, The tools): the item is held, a task of its
//! own waits on the card and lets it in or drops it, and the `send` itself
//! waits a little while for that answer, so the teammate's one message about
//! it is true: in their inbox, declined, or not sent yet.

use super::{IN_INBOX, ROW_IS_THE_RECEIPT, Room, lock, moved, topics};
use crate::contract::{InboxItem, InboxOutcome, InboxState, InboxWants, Persona};
use serde_json::{Value, json};
use std::sync::Arc;

/// How long a `send` that raised a first-contact card waits for the
/// person's answer before it returns with the item still held. A teammate
/// told at once that its item waits on a card told the person it was sent
/// anyway, beside the card: one message, and a false one. Waiting lets the
/// result say what happened. Each agent bounds a tool call on its own
/// clock, and the shortest is Codex's, 60 seconds for an MCP server's tool
/// unless its config says otherwise (`tool_timeout_sec`, which Hotline does
/// not set); Claude Code allows hours, and Hotline Agent calls in process
/// with no bound. Under 60 with room for the answer to land is 45.
const HELD_SEND_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

/// Where one item a send made stands when its result is written.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::session) enum Landed {
    /// In the receiver's inbox.
    In,
    /// Held for the person's first-contact card, still unanswered.
    Held,
    /// The person answered the card no: closed as dropped, unseen.
    Declined,
    /// Closed as dropped, unseen, for this reason: the card was taken down
    /// unanswered, or the item could not be let in.
    Refused(String),
}

/// What a send's result says of the items held for a card still
/// unanswered when the wait ran out, to `names`: not sent, and what to say
/// of it, which on a call is nothing, as the call says the card itself.
fn held_note(names: &str, on_call: bool) -> String {
    let say = if on_call {
        "This turn is on a call, and the call announces the card itself: do not describe the card or read it out, and say at most that you are waiting for their answer.".to_string()
    } else {
        format!(
            "Say one short line that a card needs their answer before you can do what this was for, such as \"A card needs your answer before I can ask {names}.\", and nothing else about it."
        )
    };
    format!(
        "Nothing was sent to {names} yet: the person has not yet allowed you to reach them, and a card asks them. Do not say it was sent or delivered. {say} It goes to {names} by itself once they allow it, and you are told at the start of your next turn; do not send it again. If the person asks where it stands, `read_inbox` with list `sent` says."
    )
}

/// What a send's result says of the items the person's no on their card
/// dropped, to `names`.
fn declined_note(names: &str) -> String {
    format!(
        "The person declined your reaching {names}: nothing was sent to them, and the item was closed as dropped, unseen. Do not send it again, and do not reach {names} another way for it, through another colleague or a file. If you say anything of it, say in one line that you did not send it."
    )
}

/// Where an item's files wait while the item is held for the person's card.
pub(super) fn held_key(id: &str) -> String {
    format!("inbox-{id}")
}

/// Each recipient's line, and the note after them, of a send whose items
/// landed as `landed` says, each also written on its `sent` entry: what is
/// in, what waits on the card (said for a call when `on_call`), what the
/// person declined and what was dropped for another reason.
pub(super) fn result_words(
    targets: &[Persona],
    sent: &mut [Value],
    landed: &[Landed],
    for_person: bool,
    on_call: bool,
) -> (String, String) {
    let mut delivered = Vec::new();
    for ((entry, target), landed) in sent.iter_mut().zip(targets).zip(landed) {
        let said = topics::delivery(landed);
        entry["delivery"] = json!(said);
        delivered.push(format!("{}: {said}", target.name));
    }
    let delivery = delivered.join(" · ");
    let names = |which: &dyn Fn(&Landed) -> bool| {
        targets
            .iter()
            .zip(landed)
            .filter(|(_, landed)| which(landed))
            .map(|(target, _)| target.name.clone())
            .collect::<Vec<_>>()
            .join(" or ")
    };
    let mut note = vec![format!("{delivery}.")];
    if landed.contains(&Landed::In) {
        note.push(IN_INBOX.to_string());
        if for_person {
            note.push(ROW_IS_THE_RECEIPT.to_string());
        }
    }
    let held = names(&|landed| *landed == Landed::Held);
    if !held.is_empty() {
        note.push(held_note(&held, on_call));
    }
    let declined = names(&|landed| *landed == Landed::Declined);
    if !declined.is_empty() {
        note.push(declined_note(&declined));
    }
    for (target, landed) in targets.iter().zip(landed) {
        if let Landed::Refused(why) = landed {
            note.push(format!(
                "Nothing was sent to {}: {why} The item was closed as dropped, unseen. Do not send it again unless the person asks.",
                target.name
            ));
        }
    }
    (delivery, note.join(" "))
}

impl Room {
    /// Waits, up to [`HELD_SEND_WAIT`] in all, for the card behind each
    /// held item of one send, `waits` naming each by where it is in
    /// `targets` and `sent`, and moves each answered one in `landed` to
    /// where it now stands. Each answer is the item let in or dropped
    /// already, so the wait holds no lock the answer or anyone's inbox
    /// needs. A turn that ends drops the call and this wait with it, never
    /// the card or the item.
    pub(super) async fn wait_for_cards(
        &self,
        waits: Vec<(usize, tokio::sync::oneshot::Receiver<Landed>)>,
        targets: &[Persona],
        sent: &[Value],
        landed: &mut [Landed],
    ) {
        let until = tokio::time::Instant::now() + HELD_SEND_WAIT;
        for (at, answered) in waits {
            let Ok(Ok(now)) = tokio::time::timeout_at(until, answered).await else {
                continue;
            };
            // This result tells the sender what became of it, as a send
            // that needed no card tells it the item landed.
            let id = sent[at]["id"].as_str().unwrap_or_default();
            self.change_item(&targets[at].id, id, |kept| {
                let told = kept.sender_told < 1;
                kept.sender_told = kept.sender_told.max(1);
                told
            });
            landed[at] = now;
        }
    }

    /// Waits on a task of its own for the person's first-contact card that
    /// a held item needs, and lets it in on yes, or closes it as dropped,
    /// unseen by its receiver, on no. `prompted` is the item whose quiet
    /// turn sent it, which the card names. The answer says where the item
    /// stands once that is done, to whoever still listens: the card and the
    /// item outlive the `send` that waited on them.
    pub(super) fn await_consent(
        self: &Arc<Self>,
        item: InboxItem,
        prompted: Option<String>,
    ) -> tokio::sync::oneshot::Receiver<Landed> {
        let (tell, told) = tokio::sync::oneshot::channel();
        if !lock(&self.held_awaited).insert(item.id.clone()) {
            // Waited on already: the send that held it is still waiting
            // when the start's recovery comes round.
            return told;
        }
        let room = self.clone();
        tokio::spawn(async move {
            let decided = async {
                let caller = room.participant(&item.from)?;
                let target = room.persona(&item.to)?;
                let caller_lease = room.request_lease(&caller.id);
                let target_lease = room.request_lease(&target.id);
                room.authorize_collaboration(
                    &caller,
                    &target,
                    &caller_lease,
                    &target_lease,
                    item.wants == InboxWants::Work,
                    crate::session::peers::delivery::Asked {
                        thread: Some(&item.from_thread),
                        prompted,
                    },
                )
                .await
            }
            .await;
            let landed = room.consent_decided(&item, decided);
            lock(&room.held_awaited).remove(&item.id);
            let _ = tell.send(landed);
        });
        told
    }

    pub(super) fn consent_decided(
        self: &Arc<Self>,
        item: &InboxItem,
        decided: Result<(), crate::session::peers::delivery::Refused>,
    ) -> Landed {
        let key = held_key(&item.id);
        let declined = decided.as_ref().is_err_and(|refused| refused.declined);
        let files = match decided {
            Ok(()) => self.held(&key, &item.files),
            Err(refused) => Err(refused.why),
        };
        let let_in = files.and_then(|files| {
            let mut landing = item.clone();
            self.with_files(&mut landing, &files)?;
            Ok(landing.body)
        });
        self.drop_held(&key);
        let changed = self.change_item(&item.to, &item.id, |kept| {
            if !kept.held || kept.state == InboxState::Closed {
                return false;
            }
            match &let_in {
                Ok(body) => {
                    kept.held = false;
                    kept.files.clear();
                    kept.body.clone_from(body);
                    moved(kept, InboxState::New, None, None, None);
                }
                Err(why) => moved(
                    kept,
                    InboxState::Closed,
                    Some(why.clone()),
                    Some(InboxOutcome::Dropped),
                    None,
                ),
            }
            true
        });
        match changed {
            Some(released) if !released.held => {
                self.count_message(&released.from, &released.to);
                self.arrived(&released);
                Landed::In
            }
            Some(_) if declined => Landed::Declined,
            Some(dropped) => Landed::Refused(dropped.reason.unwrap_or_default()),
            None => Landed::Refused("It was closed before the card was answered.".into()),
        }
    }
}
