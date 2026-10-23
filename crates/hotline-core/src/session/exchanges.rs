//! Durable automatic exchanges. A pair record is the queue and the brake in
//! one append: accepting a message never relies on a task surviving the desk.
use super::sides::{HandoffStart, Start};
use super::turns::RequestLine;
use super::{Room, lock, new_id, now_ms, peers};
use crate::contract::{
    DeliveryCause, DeliveryFrom, ExchangePauseStatus, HumanActionStatus, PeerStatus,
    TranscriptEvent,
};
use crate::contract::{ThreadId, ThreadKind};
use crate::driver::CapabilityLease;
use crate::log::{StreamId, thread};
use crate::paths::{thread_key, thread_participants};
use crate::thread::Opener;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub(crate) const EXCHANGE_CAP: i64 = 12;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Intent {
    #[default]
    Ask,
    Handoff,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Queued,
    Running,
    /// A turn ended cleanly with a card or a question still open: only its
    /// answer carries the work on. Saved as `waitinghuman`, as it always was.
    #[serde(rename = "waitinghuman")]
    Waiting,
    Reply,
    Done,
    Stopped,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Request {
    id: String,
    from: String,
    to: String,
    message: String,
    intent: Intent,
    phase: Phase,
    reply: String,
    failed: bool,
    #[serde(default)]
    reply_counted: bool,
    #[serde(default)]
    request_counted: bool,
    #[serde(default)]
    inline: bool,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    result_consumed: bool,
    #[serde(default)]
    human_actions: Vec<HumanGate>,
    /// The work thread that serves a handoff: it runs on the teammate it was
    /// handed to, beside that teammate's DM. On an ask, the one it is answered
    /// in; absent on an ask answered in the recipient's main conversation,
    /// and on a handoff saved before handoffs had threads of their own.
    #[serde(default)]
    thread: Option<String>,
    /// The work thread the request was sent from, when it was sent from one:
    /// the answer comes back there instead of to the sender's DM.
    #[serde(default)]
    reply_thread: Option<String>,
    /// The job whose thread the request was sent from, when it was: the
    /// answer comes back there while the job lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_job: Option<String>,
    /// For a request from a teammate on a linked desk that was stopped here:
    /// its desk has been told, so its sender is not left waiting.
    #[serde(default)]
    returned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lead_call: Option<ThreadId>,
    /// For a handoff into a thread an earlier one opened: the id of the
    /// handoff that opened it, which is the thread's name to its sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    continues: Option<String>,
    /// Reports from this handoff's thread that its sender has not heard yet,
    /// oldest first. On the recipient's desk of a linked sender, the ones
    /// still to cross.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    progress: Vec<String>,
    /// How many reports the limit dropped from `progress`.
    #[serde(default, skip_serializing_if = "progress::is_zero")]
    progress_left_out: u32,
    /// Whether any report in `progress` may wake its sender.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    progress_wakes: bool,
    /// The person stopped the exchange, or collaboration was revoked, after
    /// this handoff: its thread no longer reports to its sender or asks it
    /// back without a card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    released: bool,
    /// For an ask a delegated thread sent the teammate that opened it: the
    /// handoff that thread serves, whose reply route the question takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    about: Option<String>,
    /// A stopped ask sent from a thread a colleague handed over: that thread
    /// is owed its failure until it is delivered there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    owed_to_thread: bool,
}
/// What a teammate sends a colleague, as `message_teammate` takes it.
pub(crate) struct Outgoing<'a> {
    /// The colleague, by name or id.
    pub to: &'a str,
    pub message: &'a str,
    pub intent: Intent,
    /// The work thread a handoff continues, or an ask is answered in, named
    /// by the id of a handoff that ran there.
    pub thread: Option<&'a str>,
}
/// Where a handoff's work is done, or where an ask that names a thread is
/// answered.
enum Place {
    /// A thread of its own, opened for it.
    New,
    /// This thread: the one it was dispatched into, or the one it continues.
    In(String),
    /// Not known yet: the thread it continues has not been opened.
    Later,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Finished {
    Waiting,
    Result,
    Answer,
    Gone,
}
pub(super) enum Route {
    Job(String),
    Work(String),
    Dm(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct HumanGate {
    id: String,
    consumed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pair {
    id: String,
    a: String,
    b: String,
    exchanges: i64,
    paused: bool,
    requests: Vec<Request>,
}
impl Pair {
    /// The name a handoff from `from` to `to` gives its thread, found by the
    /// id of any handoff that ran there: the id of the one that opened it.
    fn thread_named(&self, named: &str, from: &str, to: &str) -> Option<String> {
        self.requests
            .iter()
            .find(|r| r.id == named && r.from == from && r.to == to && r.intent == Intent::Handoff)
            .map(|r| r.continues.clone().unwrap_or_else(|| r.id.clone()))
    }
    /// Where a request that works in a thread is done. One that names a thread
    /// follows the handoffs before it into that thread, as its sender sent
    /// them; it is refused when no handoff of its sender's to its recipient
    /// opened one by that name, whatever another desk says. An ask never
    /// opens a thread: it waits for the handoff that will, and is refused
    /// when none will.
    fn place(&self, request: &Request) -> Result<Place, String> {
        if let Some(thread) = &request.thread {
            return Ok(Place::In(thread.clone()));
        }
        let Some(opener) = &request.continues else {
            return Ok(Place::New);
        };
        let before: Vec<&Request> = self
            .requests
            .iter()
            .take_while(|r| r.id != request.id)
            .filter(|r| {
                r.from == request.from
                    && r.to == request.to
                    && r.intent == Intent::Handoff
                    && (r.id == *opener || r.continues.as_ref() == Some(opener))
            })
            .collect();
        if !before.iter().any(|r| r.id == *opener) {
            return Err(match request.intent {
                Intent::Ask => "That work thread was not opened by this teammate's handoff. Ask without a thread.",
                Intent::Handoff => "That work thread was not opened by this teammate's handoff. Send without a thread to open a new one.",
            }
            .into());
        }
        if let Some(thread) = before.iter().rev().find_map(|r| r.thread.clone()) {
            return Ok(Place::In(thread));
        }
        // None of them has a thread yet: one still to be handed over opens
        // it, and this follows; if none ever will, a handoff opens it.
        if before
            .iter()
            .any(|r| matches!(r.phase, Phase::Queued | Phase::Running))
        {
            Ok(Place::Later)
        } else if request.intent == Intent::Ask {
            Err("That work thread has not been opened yet. Ask without a thread, or wait for its handoff to start.".into())
        } else {
            Ok(Place::New)
        }
    }
}

/// Whether an `exchange_pair` record has a request an agent is answering now,
/// as the room stream says it: running, or waiting on the person or on a
/// colleague's answer.
pub(crate) fn answering(record: &serde_json::Value) -> bool {
    serde_json::from_value::<Pair>(record.clone()).is_ok_and(|pair| {
        pair.requests
            .iter()
            .any(|request| matches!(request.phase, Phase::Running | Phase::Waiting))
    })
}
impl Room {
    async fn authorize_exchange(
        self: &Arc<Self>,
        request: &Request,
        caller: &crate::contract::Persona,
        target: &crate::contract::Persona,
        a: &CapabilityLease,
        b: &CapabilityLease,
    ) -> Result<bool, String> {
        a.check()?;
        b.check()?;
        // While it stands, the delegation is the grant for its thread's
        // questions back to the teammate that handed it over. Says so, for
        // the hand-over to keep to it.
        if self.about_live(request).is_some() {
            return Ok(true);
        }
        if let Some(thread) = &request.lead_call {
            if request.intent == Intent::Handoff
                && target.node.is_none()
                && self.lead_call(&caller.id, thread)
            {
                return Ok(false);
            }
            return Err("The desk call's lead grant has ended.".into());
        }
        let origin = request
            .reply_thread
            .as_deref()
            .map(|key| self.work_thread(key));
        self.authorize_collaboration(
            caller,
            target,
            a,
            b,
            request.intent == Intent::Handoff,
            origin.as_ref(),
        )
        .await
        .map(|()| false)
    }

    fn exchange_pairs(&self) -> Vec<Pair> {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .filter(|v| v["kind"] == "exchange_pair")
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    }
    fn exchange_pair(&self, key: &str) -> Option<Pair> {
        self.exchange_pairs().into_iter().find(|p| p.id == key)
    }
    fn save_pair(&self, pair: &Pair) -> Result<(), String> {
        let mut value = serde_json::to_value(pair).map_err(|e| e.to_string())?;
        value["kind"] = json!("exchange_pair");
        self.log
            .append(&StreamId::Room, &value)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub(crate) fn send_intent(
        self: &Arc<Self>,
        from: &str,
        outgoing: &Outgoing<'_>,
        capability: Option<CapabilityLease>,
        sent_from: &ThreadId,
    ) -> Result<peers::Sent, String> {
        let _working = self.working()?;
        if let Some(capability) = &capability {
            capability.check()?;
        }
        let intent = outgoing.intent;
        let (caller, target, message) = self.checked(from, outgoing.to, outgoing.message)?;
        let key = thread_key(&caller.id, &target.id).ok_or("Invalid teammate pair")?;
        let id = new_id();
        let leases = (
            capability
                .unwrap_or_else(|| self.capability_lease(&caller.id))
                .scoped(),
            self.capability_lease(&target.id).scoped(),
        );
        let continues = {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(&key).unwrap_or(Pair {
                id: key.clone(),
                a: caller.id.clone(),
                b: target.id.clone(),
                exchanges: 0,
                paused: false,
                requests: vec![],
            });
            let continues = match outgoing.thread.map(str::trim) {
                None => None,
                Some(named) => Some(
                    pair.thread_named(named, &caller.id, &target.id)
                        .ok_or_else(|| {
                            format!(
                                "{named} is not a handoff you sent {}. Leave `thread` out to {}.",
                                target.name,
                                match intent {
                                    Intent::Ask => "ask their main conversation",
                                    Intent::Handoff => "open a new work thread",
                                }
                            )
                        })?,
                ),
            };
            let about = (intent == Intent::Ask && continues.is_none())
                .then(|| self.asked_back_about(sent_from, &target.id))
                .flatten();
            pair.requests.push(Request {
                id: id.clone(),
                from: caller.id,
                to: target.id,
                message: message.to_string(),
                intent,
                phase: Phase::Queued,
                reply: String::new(),
                failed: false,
                reply_counted: false,
                request_counted: false,
                inline: false,
                started: false,
                result_consumed: false,
                human_actions: vec![],
                thread: None,
                reply_thread: matches!(sent_from.kind, ThreadKind::Side | ThreadKind::Call)
                    .then(|| sent_from.key.clone()),
                reply_job: (sent_from.kind == ThreadKind::Job).then(|| sent_from.key.clone()),
                returned: false,
                lead_call: (intent == Intent::Handoff
                    && target.node.is_none()
                    && self.lead_call(from, sent_from))
                .then(|| sent_from.clone()),
                continues: continues.clone(),
                progress: Vec::new(),
                progress_left_out: 0,
                progress_wakes: false,
                released: false,
                about,
                owed_to_thread: false,
            });
            self.save_pair(&pair)?;
            if intent == Intent::Ask {
                self.gate_on_ask(sent_from, &id);
            }
            lock(&self.exchange_leases).insert(id.clone(), leases);
            continues
        };
        self.wake_exchange(&key);
        Ok(peers::Sent {
            to: target.name,
            thread: (intent == Intent::Handoff).then(|| continues.unwrap_or_else(|| id.clone())),
            request_id: id,
        })
    }
    pub fn resume_exchange(&self, a: &str, b: &str) -> Result<(), String> {
        self.change_exchange(a, b, false)
    }
    pub fn stop_exchange(&self, a: &str, b: &str) -> Result<(), String> {
        self.change_exchange(a, b, true)
    }
    fn change_exchange(&self, a: &str, b: &str, stop: bool) -> Result<(), String> {
        self.participant(a)?;
        self.participant(b)?;
        let key = thread_key(a, b)
            .filter(|_| a != b)
            .ok_or("Choose two different teammates")?;
        let _guard = lock(&self.exchange_lock);
        let mut pair = self
            .exchange_pair(&key)
            .ok_or("There is no exchange for this pair")?;
        if stop {
            let mut stopped = Vec::new();
            for request in &mut pair.requests {
                progress::release(request);
                if Self::exchange_pending(request) {
                    request.owed_to_thread = self.owes_thread(request, None);
                    self.cancel_handoff(request);
                    request.phase = Phase::Stopped;
                    request.failed = true;
                    request.reply = peers::STOPPED_BY_THE_PERSON.into();
                    stopped.push(request.clone());
                }
            }
            self.exchange_notices(&stopped);
            self.wake_owed(&pair);
        }
        pair.exchanges = 0;
        pair.paused = false;
        self.save_pair(&pair)?;
        self.exchange_card(
            &pair,
            if stop {
                ExchangePauseStatus::Stopped
            } else {
                ExchangePauseStatus::Resumed
            },
        );
        Ok(())
    }
    pub(super) fn heard_from_person(&self, persona: &str) {
        let _guard = lock(&self.exchange_lock);
        for mut pair in self.exchange_pairs() {
            if pair.a != persona && pair.b != persona {
                continue;
            }
            pair.exchanges = 0;
            pair.paused = false;
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("reset exchange: {error}");
            }
            self.exchange_card(&pair, ExchangePauseStatus::Resumed);
        }
    }
    fn exchange_card(&self, pair: &Pair, status: ExchangePauseStatus) {
        for (whose, other) in [(&pair.a, &pair.b), (&pair.b, &pair.a)] {
            if status != ExchangePauseStatus::Pending {
                // Settle the actual open cards, including the pair-wide ids
                // older versions wrote. Keep the pause's time and count, not
                // the reset counter, and publish the settlement to both tapes.
                for mut event in self.tape(whose).into_iter().filter(|event| {
                    event["kind"] == "exchange_paused"
                        && event["withPersonaId"] == *other
                        && event["status"] == "pending"
                }) {
                    event["status"] = json!(status);
                    self.write_value(whose, &event);
                }
                continue;
            }
            self.write(
                whose,
                &TranscriptEvent::ExchangePaused {
                    // A pause is a new decision, not a revival of the previous
                    // card. Reusing its id folds it back into old history and
                    // retains the mounted button's already-answering state.
                    id: format!("exchange-paused:{}", new_id()),
                    ts: now_ms(),
                    with_persona_id: other.clone(),
                    with_name: self
                        .participant(other)
                        .map_or_else(|_| other.clone(), |p| super::links::shown(&p)),
                    exchanges: pair.exchanges,
                    status,
                },
            );
        }
    }
    /// Tells each sender, once, what of theirs was stopped and why. A stop
    /// ends every message a teammate had waiting on another at once, and a
    /// line for each buried the sender's conversation under the same words;
    /// so the messages one sender had to one teammate share a line, which
    /// names that teammate rather than giving its id. A thread a colleague
    /// handed over is told there instead what of its own was stopped.
    fn exchange_notices(&self, stopped: &[Request]) {
        let mut groups: Vec<(&str, &str, Vec<&Request>)> = Vec::new();
        for request in stopped.iter().filter(|r| !r.owed_to_thread) {
            match groups
                .iter_mut()
                .find(|(from, to, _)| *from == request.from && *to == request.to)
            {
                Some((_, _, requests)) => requests.push(request),
                None => groups.push((&request.from, &request.to, vec![request])),
            }
        }
        for (from, to, requests) in groups {
            self.write_value(
                from,
                &json!({"kind":"notice", "id":format!("exchange-ended:{}",requests[0].id),
                "ts":now_ms(), "level":"info", "text":self.stopped_line(to, &requests)}),
            );
        }
    }
    /// What a sender is shown of the messages of theirs to `to` a stop ended.
    fn stopped_line(&self, to: &str, requests: &[&Request]) -> String {
        let first = requests[0];
        let last = requests[requests.len() - 1];
        let name = self
            .participant(to)
            .map_or_else(|_| to.to_string(), |p| super::links::shown(&p));
        match requests.len() {
            1 => format!(
                "Message to {name} ({}): {}",
                peers::about(&first.message),
                first.reply
            ),
            n => format!(
                "{n} messages to {name} stopped, the last ({}): {}",
                peers::about(&last.message),
                last.reply
            ),
        }
    }
    fn exchange_pending(request: &Request) -> bool {
        request.phase != Phase::Stopped
            && (request.phase != Phase::Done || (!request.inline && !request.result_consumed))
    }
    pub(super) fn revoke_exchanges(&self, persona: &str) {
        self.stop_revoked_exchanges(Some(persona));
    }
    fn stop_revoked_exchanges(&self, persona: Option<&str>) {
        let _guard = lock(&self.exchange_lock);
        let mut stopped = Vec::new();
        for mut pair in self.exchange_pairs() {
            let ours = persona.is_some_and(|id| pair.a == id || pair.b == id);
            let mut changed = false;
            for request in &mut pair.requests {
                if ours {
                    changed |= progress::release(request);
                }
                if !Self::exchange_pending(request)
                    || (!ours && self.exchange_lease_current(&request.id))
                {
                    continue;
                }
                request.owed_to_thread = self.owes_thread(request, persona);
                self.cancel_handoff(request);
                request.phase = Phase::Stopped;
                progress::release(request);
                request.failed = true;
                request.reply = "Collaboration was revoked or a participant stopped. Send again if still needed.".into();
                stopped.push(request.clone());
                changed = true;
            }
            if changed && let Err(error) = self.save_pair(&pair) {
                eprintln!("revoke exchange: {error}");
            }
            self.wake_owed(&pair);
        }
        self.exchange_notices(&stopped);
    }
    fn exchange_lease_current(&self, id: &str) -> bool {
        lock(&self.exchange_leases)
            .get(id)
            .is_none_or(|(a, b)| a.is_current() && b.is_current())
    }
    /// Whether this id names a request of some pair: a delivery that merely
    /// carries a peer's name (a hand-written one, say) names none.
    pub(super) fn is_exchange_request(&self, id: &str) -> bool {
        self.exchange_pairs()
            .iter()
            .any(|pair| pair.requests.iter().any(|r| r.id == id))
    }
    pub(super) fn begin_exchange_result(&self, id: &str) -> bool {
        let _guard = lock(&self.exchange_lock);
        if !self.exchange_lease_current(id) {
            return false;
        }
        for mut pair in self.exchange_pairs() {
            if let Some(request) = pair.requests.iter_mut().find(|r| {
                r.id == id && matches!(r.phase, Phase::Reply | Phase::Done) && !r.result_consumed
            }) {
                request.result_consumed = true;
                return self.save_pair(&pair).is_ok();
            }
        }
        false
    }
    pub(super) fn forget_informed_collaboration(&self, persona: &str) {
        for mut event in self.log.load(&StreamId::Room) {
            if event["kind"] == "collaboration_informed"
                && (event["from"] == persona || event["to"] == persona)
            {
                event["deleted"] = json!(true);
                if let Err(error) = self.log.append(&StreamId::Room, &event) {
                    eprintln!("expire informed collaboration: {error}");
                }
            }
        }
    }
    fn cancel_handoff(&self, request: &Request) {
        if let Some((caller, target)) = lock(&self.exchange_leases).get(&request.id) {
            caller.revoke();
            target.revoke();
        }
        self.settle_invalid_collaboration();
        // A handoff's turn is running in its own thread: stopping it stops that
        // turn and closes the thread. One whose turn is over has nothing to
        // stop, and what the person is saying to it now is theirs.
        //
        // An ask answered in a thread is only a line in it, so stopping it
        // stops only the turn answering it, when that is the turn in flight:
        // the thread, the handoff's turn and the person's own go on, and a
        // line not yet begun is skipped at the door. An ask answered in its
        // recipient's main conversation, or in the conversation that handed
        // its thread over, has no thread, and that turn goes on: only its
        // answer is no longer returned.
        if request.phase == Phase::Running
            && let Some(thread) = &request.thread
        {
            match request.intent {
                Intent::Handoff => self.stop_work(thread, "Stopped."),
                Intent::Ask => self.cancel_turn_answering(thread, &request.id),
            }
        }
    }
    pub(super) fn begin_request(&self, id: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        if !self.exchange_lease_current(id) {
            return Err("This handoff’s originating capability expired.".into());
        }
        for mut pair in self.exchange_pairs() {
            if let Some(r) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id && r.phase == Phase::Running)
            {
                r.started = true;
                return self.save_pair(&pair);
            }
        }
        Err("This handoff was stopped before its turn.".into())
    }
    /// Saves what a request's turn came to. `Answer` for an ask, whose reply
    /// the pair's thread keeps too. `Waiting` when a handoff's turn stopped to
    /// wait on the person or a colleague, so the work is not over: the result
    /// is held until the answer has carried it on. `Gone` when a handoff was
    /// stopped meanwhile; a stopped ask's turn ends as an `Answer` nobody is
    /// owed, for an ask never closes the thread it was answered in.
    pub(super) fn finish_request(&self, id: &str, reply: String, failed: bool) -> Finished {
        let _guard = lock(&self.exchange_lock);
        for mut pair in self.exchange_pairs() {
            let Some(request) = pair.requests.iter_mut().find(|r| r.id == id) else {
                continue;
            };
            if request.phase != Phase::Running {
                return if request.intent == Intent::Ask {
                    Finished::Answer
                } else {
                    Finished::Gone
                };
            }
            if request.intent == Intent::Ask {
                request.phase = Phase::Reply;
                request.reply = reply;
                request.failed = failed;
                let (to, reply) = (request.to.clone(), request.reply.clone());
                if let Err(error) = self.save_pair(&pair) {
                    eprintln!("save an ask's answer: {error}");
                }
                if let Err(error) =
                    self.exchange_thread_line(&pair.id, &to, &format!("result:{id}"), &reply)
                {
                    eprintln!("write an ask's answer to its thread: {error}");
                }
                return Finished::Answer;
            }
            // Only a cleanly completed turn establishes a safe suspension point.
            // A crash or a failed turn is uncertain even if it posted a card.
            let waiting = !failed && request.human_actions.iter().any(|g| !g.consumed);
            request.phase = if waiting {
                Phase::Waiting
            } else {
                Phase::Reply
            };
            request.reply = reply;
            request.failed = failed;
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("save handoff result: {error}");
            }
            return if waiting {
                Finished::Waiting
            } else {
                Finished::Result
            };
        }
        Finished::Gone
    }
    /// Keep routing in the pair record, not the lifetime of the asking turn:
    /// a request the person was asked in a handoff's thread is a gate the
    /// handoff's result waits behind. One asked while an ask is being
    /// answered there is a gate for nothing: the ask is answered whatever the
    /// person does, and their answer is heard as an ordinary one.
    pub(super) fn link_handoff_human(
        &self,
        thread: Option<&str>,
        action: &str,
    ) -> Result<(), String> {
        let handoff = thread
            .and_then(|thread| self.sides.get(thread))
            .and_then(|side| side.request());
        let Some(id) = handoff else { return Ok(()) };
        let _guard = lock(&self.exchange_lock);
        if self.is_ask(&id) {
            return Ok(());
        }
        if !self.exchange_lease_current(&id) {
            return Err("This handoff’s originating capability expired.".into());
        }
        for mut pair in self.exchange_pairs() {
            if let Some(r) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id && r.phase == Phase::Running)
            {
                r.human_actions.push(HumanGate {
                    id: action.into(),
                    consumed: false,
                });
                return self.save_pair(&pair);
            }
        }
        Err("This handoff was stopped before it could ask the person.".into())
    }

    /// Lets a handoff stop waiting behind a card its teammate withdrew. While
    /// the handoff's turn runs, the gate simply goes, so the turn's end hands
    /// the result back. Once the turn has ended to wait on the card, only an
    /// answer carries the work on, so the card cannot be withdrawn.
    pub(super) fn unlink_handoff_human(&self, persona: &str, action: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        for mut pair in self.exchange_pairs() {
            let Some(request) = pair.requests.iter_mut().find(|r| {
                r.to == persona
                    && r.human_actions
                        .iter()
                        .any(|g| g.id == action && !g.consumed)
            }) else {
                continue;
            };
            if request.phase != Phase::Running {
                return Err("A handoff's result is waiting on this request, so it stays up until the person answers it.".into());
            }
            request.human_actions.retain(|g| g.id != action);
            return self.save_pair(&pair);
        }
        Ok(())
    }

    /// The handoff a card or a question was raised in, if a handoff waits
    /// behind it: the exchange request that is answered by it.
    pub(super) fn handoff_awaiting(&self, persona: &str, action: &str) -> Option<String> {
        self.exchange_pairs()
            .iter()
            .flat_map(|p| &p.requests)
            .find(|r| r.to == persona && r.human_actions.iter().any(|g| g.id == action))
            .map(|r| r.id.clone())
    }

    /// Whether an answer can still carry this handoff on: it is at work or
    /// waiting. One that ended with its card open has nothing left to resume.
    pub(super) fn handoff_is_open(&self, id: &str) -> bool {
        self.exchange_pairs()
            .iter()
            .flat_map(|p| &p.requests)
            .any(|r| r.id == id && matches!(r.phase, Phase::Running | Phase::Waiting))
    }

    /// Where a delivery came from, as the typed field a turn reads. A result
    /// comes from the thread that did the work: the work thread of a handoff,
    /// else the pair's.
    pub(super) fn delivery_source(&self, persona_id: &str, cause: &DeliveryCause) -> DeliveryFrom {
        if let DeliveryCause::Peer {
            request_id: Some(id),
            ..
        } = cause
            && let Some(thread) = self
                .exchange_pairs()
                .iter()
                .flat_map(|p| &p.requests)
                .find(|r| r.id == *id)
                .and_then(|r| r.thread.clone())
        {
            return DeliveryFrom::new(&ThreadId::side(thread), Some(id.clone()));
        }
        peers::delivery_from(persona_id, cause)
    }

    /// Restart loses session consent. Recheck today's directional grant before
    /// letting an answer start another turn; an answer itself grants nothing.
    pub(super) async fn resume_handoff_answer(
        self: &Arc<Self>,
        id: &str,
        action: &str,
    ) -> Result<(), String> {
        let (key, request) = self
            .exchange_pairs()
            .into_iter()
            .find_map(|p| {
                p.requests
                    .into_iter()
                    .find(|r| r.id == id)
                    .map(|r| (p.id, r))
            })
            .ok_or("Handoff disappeared")?;
        if request.phase != Phase::Waiting
            || !self.exchange_lease_current(id)
            || !request
                .human_actions
                .iter()
                .any(|g| g.id == action && !g.consumed)
        {
            return Err("This handoff is no longer waiting for that answer.".into());
        }
        let caller = self.participant(&request.from)?;
        let target = self.persona(&request.to)?;
        let (a, b) = lock(&self.exchange_leases)
            .entry(id.into())
            .or_insert_with(|| {
                (
                    self.capability_lease(&caller.id).scoped(),
                    self.capability_lease(&target.id).scoped(),
                )
            })
            .clone();
        let auth = tokio::select! {
            biased;
            () = peers::capabilities_expired(&a, &b) => Err("Collaboration was revoked.".into()),
            auth = self.authorize_exchange(&request, &caller, &target, &a, &b) => auth,
        };
        if let Err(error) = auth {
            self.set_exchange_reply(&key, id, error.clone(), true)?;
            self.wake_exchange(&key);
            return Err(error);
        }
        let _guard = lock(&self.exchange_lock);
        a.check()?;
        b.check()?;
        let mut pair = self.exchange_pair(&key).ok_or("Exchange disappeared")?;
        let r = pair
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.phase == Phase::Waiting)
            .ok_or("This handoff was stopped before its answer")?;
        let gate = r
            .human_actions
            .iter_mut()
            .find(|g| g.id == action && !g.consumed)
            .ok_or("This answer was already consumed")?;
        gate.consumed = true;
        r.phase = Phase::Running;
        r.started = true;
        self.save_pair(&pair)
    }

    // Reconcile before Room is published. The delayed recovery task must never
    // mistake work accepted by this process for an interrupted previous turn.
    pub(super) fn reconcile_exchanges(&self) {
        {
            let _guard = lock(&self.exchange_lock);
            for mut pair in self.exchange_pairs() {
                for request in &mut pair.requests {
                    // A sender that waited inline, as an older version's ask
                    // session did, died with the process. Return its eventual
                    // result to its owner's main conversation instead.
                    if !matches!(request.phase, Phase::Done | Phase::Stopped) {
                        request.inline = false;
                    }
                    // A handoff saved before handoffs had threads of their own
                    // was waiting on the person in the recipient's DM, which no
                    // longer resumes it. Say so to the sender instead of leaving
                    // the request waiting for an answer nobody can carry on.
                    if request.phase == Phase::Waiting && request.thread.is_none() {
                        request.phase = Phase::Reply;
                        request.failed = true;
                        request.reply = "Hotline was updated while this handoff waited on the person. Work may have started; inspect before retrying.".into();
                    }
                    // A request forwarded to a linked desk is that desk's to
                    // finish; its answer comes back over the link.
                    if request.phase != Phase::Running || crate::linked::is_linked(&request.to) {
                        continue;
                    }
                    // A read receipt proves a turn began, not that it finished. Never
                    // replay potentially side-effecting work after an interrupted turn.
                    // Every turn that takes a request up marks it started first; the
                    // receipts are read on the recipient's DM tape, where a question
                    // to its main conversation lands, as did a handoff saved before it
                    // had a thread.
                    let read = self.tape(&request.to).iter().any(|v| {
                        matches!(v["cause"]["kind"].as_str(), Some("handoff" | "question"))
                            && v["cause"]["requestId"] == request.id.as_str()
                            && v["receipt"] == "read"
                    });
                    if request.started || read {
                        request.phase = Phase::Reply;
                        request.failed = true;
                        request.reply = "Hotline restarted before the result was saved. Work may have started; inspect before retrying.".into();
                    } else {
                        request.phase = Phase::Queued;
                    }
                }
                if let Err(error) = self.save_pair(&pair) {
                    eprintln!("recover exchange: {error}");
                }
            }
        }
    }
    pub(super) fn recover_queued_exchanges(self: &Arc<Self>) {
        for pair in self.exchange_pairs() {
            self.wake_exchange(&pair.id);
        }
    }
    fn wake_exchange(self: &Arc<Self>, key: &str) {
        if !lock(&self.exchange_workers).insert(key.to_string()) {
            return;
        }
        let room = Arc::downgrade(self);
        let key = key.to_string();
        tokio::spawn(async move {
            // Keep a weak room between looks, so a paused queue never keeps a
            // desk alive. Resume is a durable state change, not a lost notification.
            loop {
                let Some(room) = room.upgrade() else {
                    break;
                };
                if let Err(error) = room.exchange_step(&key).await {
                    eprintln!("exchange {key} waiting after error: {error}");
                }
                {
                    // Serialize retirement with enqueue so a new request cannot
                    // lose its worker between the empty check and removal.
                    let _guard = lock(&room.exchange_lock);
                    if room.exchange_pair(&key).is_none_or(|pair| {
                        pair.requests.iter().all(|r| {
                            matches!(r.phase, Phase::Done | Phase::Stopped)
                                && !Self::owes_answer(r)
                                && !Self::owes_linked_progress(r)
                        })
                    }) {
                        lock(&room.exchange_workers).remove(&key);
                        break;
                    }
                }
                drop(room);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
    }
    async fn exchange_step(self: &Arc<Self>, key: &str) -> Result<(), String> {
        let Some(pair) = self.exchange_pair(key) else {
            return Ok(());
        };
        for request in pair
            .requests
            .iter()
            .filter(|r| Self::owes_linked_progress(r))
        {
            self.send_linked_progress(key, &request.id).await;
        }
        // A sender whose request was stopped here, on a linked desk or in
        // a thread a colleague handed over, hears so first.
        if let Some(owed) = pair.requests.iter().find(|r| Self::owes_answer(r)).cloned() {
            if owed.owed_to_thread {
                return self.tell_thread_stopped(key, &owed).await;
            }
            if self
                .return_linked(&owed.id, &owed.from, &owed.reply, true)
                .await
            {
                self.mark_returned(key, &owed.id)?;
            }
            return Ok(());
        }
        // Every request goes as far as it can, not only the first: a handoff
        // at work in its thread, or waiting on the person or another desk,
        // holds up nothing behind it. An ask is answered while this waits.
        let open: Vec<String> = pair
            .requests
            .iter()
            .filter(|r| !matches!(r.phase, Phase::Done | Phase::Stopped))
            .map(|r| r.id.clone())
            .collect();
        for id in open {
            if let Err(error) = self.exchange_request(key, &id).await {
                eprintln!("exchange {key} request {id} waiting after error: {error}");
            }
        }
        Ok(())
    }
    /// One request taken as far as it can go now, as the pair stands.
    async fn exchange_request(self: &Arc<Self>, key: &str, id: &str) -> Result<(), String> {
        let Some(pair) = self.exchange_pair(key) else {
            return Ok(());
        };
        let Some(request) = pair
            .requests
            .iter()
            .find(|r| r.id == id && !matches!(r.phase, Phase::Done | Phase::Stopped))
            .cloned()
        else {
            return Ok(());
        };
        // A request that came in as its link was ending has a side no
        // longer linked: it ends the way the others did.
        if let Some(desk) = [&request.from, &request.to].into_iter().find_map(|id| {
            crate::linked::split(id)
                .map(|(desk, _)| desk)
                .filter(|desk| crate::linked::record(&self.log, desk).is_none())
        }) {
            self.stop_linked_exchanges(desk, "another desk");
            return Ok(());
        }
        if !self.exchange_lease_current(&request.id) {
            self.stop_revoked_exchanges(None);
            return Ok(());
        }
        if pair.paused
            && !matches!(request.phase, Phase::Running | Phase::Waiting)
            && !(request.phase == Phase::Reply && request.reply_counted)
        {
            return Ok(());
        }
        match request.phase {
            Phase::Queued if crate::linked::is_linked(&request.to) => {
                self.forward_queued(key, &request).await?;
            }
            Phase::Queued => {
                let in_thread = request.intent == Intent::Handoff || request.continues.is_some();
                let thread = if in_thread {
                    match pair.place(&request) {
                        Ok(Place::New) => None,
                        Ok(Place::In(thread)) => Some(thread),
                        Ok(Place::Later) => return Ok(()),
                        Err(error) => {
                            return self.set_exchange_reply(key, &request.id, error, true);
                        }
                    }
                } else {
                    None
                };
                // What works in a thread waits its turn for a place on its
                // recipient: a teammate that has as many threads live as it
                // may, every one of them mid-turn, takes it up when one has
                // finished or can be parked. Nothing is refused and nothing is
                // interrupted. One continuing a thread that is live already
                // needs no place.
                if in_thread
                    && thread
                        .as_deref()
                        .is_none_or(|thread| self.sides.get(thread).is_none())
                    && !self.sides.has_room(&request.to)
                {
                    return Ok(());
                }
                let caller = self.participant(&request.from)?;
                let target = self.persona(&request.to)?;
                let (caller_capability, target_capability) = lock(&self.exchange_leases)
                    .entry(request.id.clone())
                    .or_insert_with(|| {
                        (
                            self.capability_lease(&caller.id),
                            self.capability_lease(&target.id),
                        )
                    })
                    .clone();
                let auth = tokio::select! {
                    biased;
                    () = peers::capabilities_expired(&caller_capability, &target_capability) => {
                        self.stop_revoked_exchanges(None);
                        return Ok(());
                    }
                    auth = self.authorize_exchange(&request, &caller, &target, &caller_capability, &target_capability) => auth,
                };
                let by_delegation = match auth {
                    Ok(by_delegation) => by_delegation,
                    Err(error) => return self.set_exchange_reply(key, &request.id, error, true),
                };
                {
                    let _guard = lock(&self.exchange_lock);
                    let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
                    if pair.paused {
                        return Ok(());
                    }
                    let Some(r) = pair
                        .requests
                        .iter_mut()
                        .find(|r| r.id == request.id && r.phase == Phase::Queued)
                    else {
                        return Ok(());
                    };
                    r.phase = Phase::Running;
                    if !r.request_counted {
                        r.request_counted = true;
                        pair.exchanges += 1;
                    }
                    pair.paused = pair.exchanges >= EXCHANGE_CAP;
                    self.save_pair(&pair)?;
                    if pair.paused {
                        self.exchange_card(&pair, ExchangePauseStatus::Pending);
                    }
                }
                self.hand_over_request(key, &request, thread, by_delegation)
                    .await?;
            }
            Phase::Running => {
                // A handoff whose turn has not begun, in no live thread, is
                // handed over again: the desk restarted, or its thread closed
                // with the line still waiting in it. So is an ask answered in
                // a thread after a restart; a thread that closes or parks with
                // one waiting fails it instead. A live dispatch has a thread
                // by the time this looks.
                if (request.intent == Intent::Handoff || request.thread.is_some())
                    && !request.started
                    && request
                        .thread
                        .as_deref()
                        .is_none_or(|thread| self.sides.get(thread).is_none())
                {
                    let thread = match pair.place(&request) {
                        Ok(Place::New) => None,
                        Ok(Place::In(thread)) => Some(thread),
                        Ok(Place::Later) => return Ok(()),
                        Err(error) => {
                            return self.set_exchange_reply(key, &request.id, error, true);
                        }
                    };
                    self.hand_over_request(key, &request, thread, false).await?;
                }
            }
            Phase::Waiting => {
                // The card or the question is durable before dispatch. Repair
                // an answer saved just before the desk stopped, without
                // replaying an old turn. A question's answer still at `Reply`
                // is its own pair's to deliver; a question stopped instead of
                // answered hands over its failure, which carries the work on.
                // Each is repaired on its own: one that cannot be still lets
                // the others through.
                for gate in request.human_actions.iter().filter(|g| !g.consumed) {
                    let handed_over = if let Some((
                        TranscriptEvent::HumanAction {
                            reason,
                            status,
                            note,
                            ..
                        },
                        home,
                    )) = self.human_card(&request.to, &gate.id)
                        && status != HumanActionStatus::Pending
                    {
                        self.hand_over_answer(&request.to, &gate.id, &reason, status, note, home)
                            .await
                    } else if let Some((asked, ask)) =
                        self.exchange_pairs().into_iter().find_map(|p| {
                            p.requests
                                .into_iter()
                                .find(|r| {
                                    r.id == gate.id
                                        && matches!(r.phase, Phase::Done | Phase::Stopped)
                                })
                                .map(|r| (p.id, r))
                        })
                    {
                        self.hand_over_asked(&request, &asked, &ask).await
                    } else {
                        Ok(())
                    };
                    if let Err(error) = handed_over {
                        eprintln!(
                            "handoff {} could not take up {}: {error}",
                            request.id, gate.id
                        );
                    }
                }
            }
            Phase::Reply => {
                if !request.reply_counted {
                    let _guard = lock(&self.exchange_lock);
                    let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
                    if pair.paused {
                        return Ok(());
                    }
                    let Some(r) = pair
                        .requests
                        .iter_mut()
                        .find(|r| r.id == request.id && r.phase == Phase::Reply)
                    else {
                        return Ok(());
                    };
                    r.reply_counted = true;
                    pair.exchanges += 1;
                    pair.paused = pair.exchanges >= EXCHANGE_CAP;
                    self.save_pair(&pair)?;
                    if pair.paused {
                        self.exchange_card(&pair, ExchangePauseStatus::Pending);
                    }
                }
                let (id, cause, from, text) = self.answer_delivery(key, &request)?;
                // A handoff's answer is the pair thread's last word. So is a
                // linked teammate's answer here: an ask's turn on this desk
                // wrote its own as it ended.
                if request.intent == Intent::Handoff || crate::linked::is_linked(&request.to) {
                    self.exchange_thread_line(
                        key,
                        &request.to,
                        &format!("result:{}", request.id),
                        &request.reply,
                    )?;
                }
                if crate::linked::is_linked(&request.from) {
                    // Answering a linked teammate: the answer goes back over
                    // the link, and waits saved while the link is down.
                    if !self
                        .return_linked(&request.id, &request.from, &request.reply, request.failed)
                        .await
                    {
                        return Ok(());
                    }
                } else {
                    match self.route_of(&request) {
                        Route::Job(job) => {
                            self.deliver_into_job(&job, &id, cause, from, text, None)?;
                        }
                        Route::Work(thread) => {
                            let carried = self.carried_on(&request);
                            self.deliver_into_work(&thread, &id, cause, from, text, carried)
                                .await?;
                        }
                        Route::Dm(sender) => {
                            self.deliver_identified(&sender, &id, cause, text, None)
                                .await?;
                        }
                    }
                }
                let _guard = lock(&self.exchange_lock);
                let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
                if let Some(r) = pair
                    .requests
                    .iter_mut()
                    .find(|r| r.id == request.id && r.phase == Phase::Reply)
                {
                    r.phase = Phase::Done;
                }
                self.save_pair(&pair)?;
            }
            Phase::Done | Phase::Stopped => {}
        }
        Ok(())
    }
    /// Where the sender hears what came of a request. A question asked from a
    /// thread a colleague handed over comes back to that thread, open or not.
    /// Anything else comes back to the job it was sent from while that job
    /// lives, else the work thread it was sent from while that is open, else
    /// the sender's own conversation.
    fn route_of(&self, request: &Request) -> Route {
        if let Some(thread) = self.delegated_origin(request) {
            return Route::Work(thread);
        }
        if let Some(job_id) = request
            .reply_job
            .as_deref()
            .filter(|job_id| self.job_lives(job_id))
        {
            return Route::Job(job_id.to_string());
        }
        if let Some(thread) = request
            .reply_thread
            .as_deref()
            .filter(|thread| self.work_is_open(thread))
        {
            return Route::Work(thread.to_string());
        }
        Route::Dm(request.from.clone())
    }
    pub(super) fn set_exchange_reply(
        &self,
        key: &str,
        id: &str,
        reply: String,
        failed: bool,
    ) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        if let Some(r) = pair
            .requests
            .iter_mut()
            .find(|r| r.id == id && !matches!(r.phase, Phase::Done | Phase::Stopped))
        {
            r.phase = Phase::Reply;
            r.reply = reply;
            r.failed = failed;
        }
        self.save_pair(&pair)
    }
    /// Hands an accepted request to the conversation that works it: a handoff
    /// to its work thread, an ask to the thread it names, one the delegation
    /// was the grant for to the conversation that handed its thread over,
    /// else to its recipient's main conversation. An ask that cannot be
    /// handed over fails to its sender.
    async fn hand_over_request(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
        by_delegation: bool,
    ) -> Result<(), String> {
        if request.intent == Intent::Handoff {
            return self.dispatch_handoff(key, request, thread).await;
        }
        let handed_over = match thread {
            Some(thread) => self.ask_in_thread(key, request, thread).await,
            None if by_delegation => self.ask_back(key, request).await,
            None => self.ask_in_main(key, request).await,
        };
        match handed_over {
            Ok(()) => Ok(()),
            Err(error) => self.set_exchange_reply(key, &request.id, error, true),
        }
    }

    /// Hands a handoff to a work thread on the teammate it was handed to, and
    /// its result is the reply the thread gives when it has done the work.
    /// `thread` is the one it continues, or its own from before a restart;
    /// without one, it opens a thread with the handoff as its first message.
    async fn dispatch_handoff(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
        thread: Option<String>,
    ) -> Result<(), String> {
        let caller = self.participant(&request.from)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let handoff = HandoffStart {
            key: key.to_string(),
            request: request.id.clone(),
        };
        let side_id = thread.unwrap_or_else(new_id);
        self.set_handoff_thread(key, &request.id, &side_id)?;
        // A thread that has run before comes back with what was said in it.
        // One the last process left live was parked when this room was
        // built (`Room::settle`), before any request is taken up.
        let ran_before =
            self.sides.get(&side_id).is_some() || self.link_of(&ThreadId::side(&side_id)).is_some();
        let started = if ran_before {
            self.wake_side(&side_id, true, Some(handoff)).await
        } else {
            self.bring_up(
                &request.to,
                Start {
                    thread: ThreadId::side(&side_id),
                    title: super::sides::title_of(&request.message),
                    started: now_ms(),
                    opener: Some(Opener {
                        persona_id: caller.id.clone(),
                        name: super::links::shown(&caller),
                    }),
                    fresh: true,
                    saved: None,
                    handoff: Some(handoff),
                },
            )
            .map(|(side, launch)| (side, Some(launch)))
        };
        let launch = match started {
            Ok((_, launch)) => launch,
            Err(error) => {
                let target = self.persona(&request.to)?;
                return self.set_exchange_reply(
                    key,
                    &request.id,
                    format!("{} could not start a thread for this: {error}", target.name),
                    true,
                );
            }
        };
        let delivered = self
            .deliver_into_work(
                &side_id,
                &format!("handoff:{}", request.id),
                DeliveryCause::Handoff {
                    request_id: request.id.clone(),
                    persona_id: caller.id.clone(),
                    name: caller.name,
                    thread_key: key.into(),
                    about: peers::about(&request.message),
                },
                DeliveryFrom::new(&ThreadId::dm(&caller.id), Some(request.id.clone())),
                request.message.clone(),
                Some(RequestLine {
                    request: request.id.clone(),
                    answer: None,
                }),
            )
            .await;
        // The line is queued first, so the agent's start finds it waiting.
        if let Some(launch) = launch {
            self.launch(launch);
        }
        delivered
    }

    fn set_handoff_thread(&self, key: &str, id: &str, thread: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        if let Some(request) = pair.requests.iter_mut().find(|r| r.id == id) {
            request.thread = Some(thread.to_string());
        }
        self.save_pair(&pair)
    }
    fn exchange_thread_line(
        &self,
        key: &str,
        from: &str,
        id: &str,
        text: &str,
    ) -> Result<(), String> {
        thread::ensure(self.log.root(), key).map_err(|e| e.to_string())?;
        let kind = if thread_participants(key).is_some_and(|(a, _)| a == from) {
            "user"
        } else {
            "agent"
        };
        if self
            .log
            .load(&StreamId::Pair(key.into()))
            .iter()
            .any(|v| v["id"] == id)
        {
            return Ok(());
        }
        // Through the threads' one write path, so either teammate finds it.
        self.threads().write(
            &ThreadId::pair(key),
            from,
            &json!({"kind":kind,"id":id,"ts":now_ms(),"text":text,"receipt":"sent"}),
        );
        Ok(())
    }
}

mod gates;
mod linked;
mod progress;
mod questions;

#[cfg(test)]
mod tests;
