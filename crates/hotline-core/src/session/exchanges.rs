//! Durable automatic exchanges. A pair record is the queue and the brake in
//! one append: accepting a message never relies on a task surviving the desk.
use super::runner::Driven;
use super::sides::{HandoffStart, Start};
use super::turns::RequestLine;
use super::{Room, lock, new_id, now_ms, peers};
use crate::contract::{
    DeliveryCause, DeliveryFrom, ExchangePauseStatus, HumanActionStatus, PeerStatus,
    TranscriptEvent,
};
use crate::contract::{ThreadId, ThreadKind};
use crate::driver::CapabilityLease;
use crate::linked;
use crate::log::{StreamId, thread};
use crate::paths::{thread_key, thread_participants};
use crate::thread::Opener;
use place::Place;
use questions::Route;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub(crate) const EXCHANGE_CAP: i64 = 12;
/// The most requests a linked desk's teammates may have waiting here at once:
/// four pairs paused at their cap.
const LINKED_WAITING_MAX: usize = 4 * EXCHANGE_CAP as usize;
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
    WaitingHuman,
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
    /// Whether an older version's ask session waited inline for this
    /// request's answer. Nothing waits so any more; kept so a saved record
    /// reads as it was written.
    #[serde(default)]
    inline: bool,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    result_consumed: bool,
    #[serde(default)]
    human_actions: Vec<HumanGate>,
    /// The work thread that serves a handoff: it runs on the teammate it was
    /// handed to, beside that teammate's DM. On an ask, the work thread it is
    /// answered in: the one it names, the one its delegator handed over from,
    /// or one of its own opened while the recipient's main conversation was
    /// busy. Absent on an ask answered in a main conversation or a job, and
    /// on a handoff saved before handoffs had threads of their own.
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
    /// For a handoff: it no longer stands, whatever its phase. Stop
    /// exchange, a revocation or the end of the link released it, and its
    /// thread's link says so (`handoffEnded`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    released: bool,
    /// For an ask a delegated thread sent the teammate that opened it: the
    /// handoff that thread serves, whose reply route the question takes and
    /// whose standing is its grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    about: Option<String>,
    /// For an ask: the work thread in `thread` was opened for it alone,
    /// because the recipient's main conversation was busy, and closes with
    /// its answer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    own_thread: bool,
    /// A stopped ask sent from a work thread or a job: that thread is owed
    /// its failure until it is handed there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    owed_to_thread: bool,
    /// For an ask: the person looped its recipient into the work thread it
    /// was sent from (`loop_in.rs`), and that is its grant.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    looped_in: bool,
}
impl Request {
    /// Ends a handoff's standing, and says whether that changed it. Its
    /// thread goes in `ended`, whose link says so once the pair is saved
    /// ([`Room::mark_handoffs`]).
    fn release(&mut self, ended: &mut Vec<String>) -> bool {
        if self.intent != Intent::Handoff || self.released {
            return false;
        }
        self.released = true;
        ended.extend(self.thread.clone());
        true
    }
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
/// What a request's turn came to, for the thread it ran in.
pub(super) enum Finished {
    /// A handoff's result: the thread's work is done.
    Result,
    /// A handoff stopped to wait on the person or a colleague; its result
    /// follows the answer.
    Waiting,
    /// A question was answered; true when the thread was opened for it
    /// alone, whose work is then done too.
    Answered(bool),
    /// A handoff stopped meanwhile.
    Gone,
}
/// Whether an `exchange_pair` record has a request an agent is answering now,
/// as the room stream says it: running, or waiting on the person.
pub(crate) fn answering(record: &serde_json::Value) -> bool {
    serde_json::from_value::<Pair>(record.clone()).is_ok_and(|pair| {
        pair.requests
            .iter()
            .any(|request| matches!(request.phase, Phase::Running | Phase::WaitingHuman))
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
    ) -> Result<(), String> {
        a.check()?;
        b.check()?;
        // While it stands, the delegation is the grant for its thread's
        // questions back to the teammate that handed it over; the person's
        // Loop in… is the grant for the one it asks.
        if request.looped_in || self.about_live(request).is_some() {
            return Ok(());
        }
        if let Some(thread) = &request.lead_call {
            if request.intent == Intent::Handoff
                && target.node.is_none()
                && self.lead_call(&caller.id, thread)
            {
                return Ok(());
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
    /// Accepts what `message_teammate` sends into the pair's durable queue,
    /// from the thread it was said in, which is where its answer comes back.
    pub(crate) fn send_intent(
        self: &Arc<Self>,
        from: &str,
        outgoing: &Outgoing<'_>,
        capability: Option<CapabilityLease>,
        sent_from: &ThreadId,
    ) -> Result<peers::Sent, String> {
        self.queue_request(from, outgoing, capability, sent_from, false)
    }
    /// Queues a request, as `send_intent` does; `looped_in` marks the ask the
    /// person's Loop in… makes, which is its own grant.
    fn queue_request(
        self: &Arc<Self>,
        from: &str,
        outgoing: &Outgoing<'_>,
        capability: Option<CapabilityLease>,
        sent_from: &ThreadId,
        looped_in: bool,
    ) -> Result<peers::Sent, String> {
        let _working = self.working()?;
        let intent = outgoing.intent;
        let (caller, target, message) = self.checked(from, outgoing.to, outgoing.message)?;
        let key = thread_key(&caller.id, &target.id).ok_or("Invalid teammate pair")?;
        let id = new_id();
        // The request holds each teammate's lasting authority, not the
        // conversation's it was sent from, so the chapter or firing that sent
        // it may end before it does. Taken before the sender's own lease is
        // checked: one still current now was current as it was taken.
        let leases = (
            self.request_lease(&caller.id),
            self.request_lease(&target.id),
        );
        if let Some(capability) = &capability {
            capability.check()?;
        }
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
                                    Intent::Ask => "ask without one",
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
                released: false,
                about,
                own_thread: false,
                owed_to_thread: false,
                looped_in,
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
        let mut ended = Vec::new();
        if stop {
            let mut stopped = Vec::new();
            for request in &mut pair.requests {
                request.release(&mut ended);
                if Self::exchange_pending(request) {
                    self.stop_request(request, "The person stopped this exchange.", None);
                    stopped.push(request.clone());
                }
            }
            self.exchange_notices(&stopped);
            self.wake_owed(&pair);
        }
        pair.exchanges = 0;
        pair.paused = false;
        self.save_pair(&pair)?;
        self.mark_handoffs(&ended);
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
    /// names that teammate rather than giving its id. A work thread or a job
    /// that asked hears what of its own was stopped there instead.
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
            let first = requests[0];
            let last = requests[requests.len() - 1];
            let name = self
                .participant(to)
                .map_or_else(|_| to.to_string(), |p| super::links::shown(&p));
            let text = match requests.len() {
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
            };
            self.write_value(
                from,
                &json!({"kind":"notice", "id":format!("exchange-ended:{}",first.id),
                "ts":now_ms(), "level":"info", "text":text}),
            );
        }
    }
    fn exchange_pending(request: &Request) -> bool {
        request.phase != Phase::Stopped
            && (request.phase != Phase::Done || (!request.inline && !request.result_consumed))
    }
    /// A stopped request whose sender has not been told where it waits: a
    /// linked desk not yet sent its failure, or a work thread or job that
    /// asked and is owed it.
    fn owes_answer(request: &Request) -> bool {
        request.phase == Phase::Stopped
            && (request.owed_to_thread || (linked::is_linked(&request.from) && !request.returned))
    }
    /// Sees that a stopped request's sender is told, by the pair's worker,
    /// whichever path stopped it.
    fn wake_owed(&self, pair: &Pair) {
        if pair.requests.iter().any(Self::owes_answer)
            && let Some(room) = lock(&self.me).upgrade()
        {
            room.wake_exchange(&pair.id);
        }
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
    /// Ends a pending request as stopped, saying why. An ask sent from a
    /// work thread or a job leaves that thread owed its failure, unless the
    /// asker is what was revoked (`revoked`): a teammate the person stopped is
    /// not woken to hear it.
    fn stop_request(&self, request: &mut Request, reply: impl Into<String>, revoked: Option<&str>) {
        request.owed_to_thread = self.owes_thread(request, revoked);
        self.cancel_request(request);
        request.phase = Phase::Stopped;
        request.failed = true;
        request.reply = reply.into();
    }
    fn cancel_request(&self, request: &Request) {
        if let Some((caller, target)) = lock(&self.exchange_leases).get(&request.id) {
            caller.revoke();
            target.revoke();
        }
        self.settle_invalid_collaboration();
        // A handoff's turn is running in its own thread: stopping it stops that
        // turn and closes the thread. One whose turn is over has nothing to
        // stop, and what the person is saying to it now is theirs.
        //
        // An ask in a work thread of its own is that thread's whole work, so
        // it closes too. One answered in a thread it named, or its delegator
        // handed over from, is only a line there: stopping it stops only the
        // turn answering it, when that is the turn in flight, and a line not
        // yet begun is skipped at the door. An ask answered in a main
        // conversation or a job is that conversation's own turn, which goes
        // on: only its answer is no longer returned.
        if request.phase != Phase::Running {
            return;
        }
        let Some(thread) = &request.thread else {
            return;
        };
        match request.intent {
            Intent::Handoff => self.stop_work(thread, "Stopped."),
            Intent::Ask if request.own_thread => self.stop_work(thread, "Stopped."),
            Intent::Ask => self.cancel_turn_answering(thread, &request.id),
        }
    }
    /// Marks a request's turn begun, while the request is still running: a
    /// handoff's or a question's turn is the exchange's, and one stopped
    /// while its line waited is never taken up.
    pub(super) fn begin_request(&self, id: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        if !self.exchange_lease_current(id) {
            return Err("This request’s originating capability expired.".into());
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
        Err("This request was stopped before its turn.".into())
    }
    /// Saves what a request's turn came to: see [`Finished`]. A turn that
    /// failed or was stopped says so, for its sender to look before trying
    /// again.
    pub(super) fn finish_request(&self, id: &str, driven: &Driven) -> Finished {
        let failed = driven.stop_reason.as_deref().is_none_or(|reason| {
            matches!(
                reason,
                "failed" | "cancelled" | "canceled" | "aborted" | "revoked"
            )
        });
        let finished = {
            let _guard = lock(&self.exchange_lock);
            let found = self.exchange_pairs().into_iter().find_map(|pair| {
                let index = pair.requests.iter().position(|r| r.id == id)?;
                Some((pair, index))
            });
            let Some((mut pair, index)) = found else {
                return Finished::Gone;
            };
            let request = &mut pair.requests[index];
            let ask = request.intent == Intent::Ask;
            if request.phase != Phase::Running {
                // A stopped ask's turn ends as an answer nobody is owed: its
                // own thread still closes, and a thread it was only a line in
                // goes on.
                return if ask {
                    Finished::Answered(request.own_thread)
                } else {
                    Finished::Gone
                };
            }
            let ended = if ask {
                "The turn answering this question ended without a result; inspect before retrying."
            } else {
                "The handoff turn ended without a result; inspect before retrying."
            };
            let mut reply = driven.replies.join("\n\n");
            if failed && reply.is_empty() {
                reply = ended.to_string();
            } else if failed && ask {
                reply = format!("{reply}\n\n{ended}");
            }
            // Only a cleanly completed turn establishes a safe suspension point.
            // A crash or a failed turn is uncertain even if it posted a card.
            let waiting = !ask && !failed && request.human_actions.iter().any(|g| !g.consumed);
            request.phase = if waiting {
                Phase::WaitingHuman
            } else {
                Phase::Reply
            };
            request.reply = reply;
            request.failed = failed;
            let finished = match (ask, waiting) {
                (true, _) => Finished::Answered(request.own_thread),
                (false, true) => Finished::Waiting,
                (false, false) => Finished::Result,
            };
            let answered = (ask && request.own_thread && !failed).then(|| request.clone());
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("save a request's result: {error}");
            }
            (finished, answered)
        };
        // A work thread that answered while the main conversation was busy
        // tells that conversation what it said on its behalf.
        if let (_, Some(answered)) = &finished {
            self.answered_for_main(answered);
        }
        finished.0
    }
    /// Keep routing in the pair record, not the lifetime of the asking turn:
    /// a request the person was asked in a handoff's thread is a gate the
    /// handoff's result waits behind. One asked while a question is being
    /// answered there is a gate for nothing: the question is answered
    /// whatever the person does, and their answer is heard as an ordinary
    /// one.
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
        if self
            .exchange_pairs()
            .iter()
            .flat_map(|pair| &pair.requests)
            .any(|r| r.id == id && r.intent == Intent::Ask)
        {
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

    /// The handoff a card was raised in, if the card is one a handoff waits
    /// behind: the exchange request that is answered by it.
    pub(super) fn handoff_awaiting(&self, persona: &str, action: &str) -> Option<String> {
        self.exchange_pairs()
            .iter()
            .flat_map(|p| &p.requests)
            .find(|r| r.to == persona && r.human_actions.iter().any(|g| g.id == action))
            .map(|r| r.id.clone())
    }

    /// Whether an answer can still carry this handoff on: it is at work or
    /// waiting on the person. One that ended with its card open has nothing
    /// left to resume.
    pub(super) fn handoff_is_open(&self, id: &str) -> bool {
        self.exchange_pairs()
            .iter()
            .flat_map(|p| &p.requests)
            .any(|r| r.id == id && matches!(r.phase, Phase::Running | Phase::WaitingHuman))
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
        if request.phase != Phase::WaitingHuman
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
                    self.request_lease(&caller.id),
                    self.request_lease(&target.id),
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
            .find(|r| r.id == id && r.phase == Phase::WaitingHuman)
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
                    if request.phase == Phase::WaitingHuman && request.thread.is_none() {
                        request.phase = Phase::Reply;
                        request.failed = true;
                        request.reply = "Hotline was updated while this handoff waited on the person. Work may have started; inspect before retrying.".into();
                    }
                    // A request forwarded to a linked desk is that desk's to
                    // finish; its answer comes back over the link.
                    if request.phase != Phase::Running || linked::is_linked(&request.to) {
                        continue;
                    }
                    // A read receipt proves a turn began, not that it finished. Never
                    // replay potentially side-effecting work after an interrupted turn.
                    // Every turn that takes a request up marks it started first; the
                    // receipts are read on the recipient's DM tape, where a question
                    // to its main conversation lands, as did a handoff saved before it
                    // had a thread. One accepted and handed over whose turn never
                    // began is handed over again.
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
                            matches!(r.phase, Phase::Done | Phase::Stopped) && !Self::owes_answer(r)
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
        // A sender whose request was stopped here, on a linked desk or in a
        // work thread or job that asked, hears so first.
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
        // holds up nothing behind it, and neither does a question being
        // answered.
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
            linked::split(id)
                .map(|(desk, _)| desk)
                .filter(|desk| linked::record(&self.log, desk).is_none())
        }) {
            self.stop_linked_exchanges(desk, "another desk");
            return Ok(());
        }
        if !self.exchange_lease_current(&request.id) {
            self.stop_revoked_exchanges(None);
            return Ok(());
        }
        if pair.paused
            && !matches!(request.phase, Phase::Running | Phase::WaitingHuman)
            && !(request.phase == Phase::Reply && request.reply_counted)
        {
            return Ok(());
        }
        match request.phase {
            Phase::Queued if linked::is_linked(&request.to) => {
                self.forward_queued(key, &request).await?;
            }
            // One being taken up is its task's until it has been handed over.
            Phase::Queued if lock(&self.exchange_admitting).contains(&request.id) => {}
            Phase::Queued => {
                // A handoff works in a thread, and so does an ask that names
                // one or was handed to one before a restart.
                let in_thread = request.intent == Intent::Handoff
                    || request.continues.is_some()
                    || request.thread.is_some();
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
                // Taking a request up may wait on the person's first-contact
                // card: it does so on a task of its own, so the pair's other
                // requests, a finished handoff's result among them, go on
                // meanwhile.
                lock(&self.exchange_admitting).insert(request.id.clone());
                let room = self.clone();
                let key = key.to_string();
                tokio::spawn(async move {
                    if let Err(error) = room.admit_request(&key, &request, thread).await {
                        eprintln!(
                            "exchange {key} request {} waiting after error: {error}",
                            request.id
                        );
                    }
                    lock(&room.exchange_admitting).remove(&request.id);
                    room.wake_exchange(&key);
                });
            }
            // One being handed over is its task's until it has been.
            Phase::Running if lock(&self.exchange_admitting).contains(&request.id) => {}
            Phase::Running => {
                // A request whose turn has not begun, in a thread no longer
                // live, is handed over again: the desk restarted, or its
                // thread closed with its line still waiting in it. A live
                // dispatch has a thread by the time this looks.
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
                    self.hand_over_request(key, &request, thread).await?;
                } else if self.dropped_by_a_swap(&request) {
                    // A chapter's swap took this question off its main
                    // conversation's queue: the session that replaced it
                    // answers it.
                    self.hand_over_request(key, &request, None).await?;
                }
            }
            Phase::WaitingHuman => {
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
                    } else if let Some((asked, ask)) = self.settled_ask(&gate.id) {
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
                // The pair's thread keeps the answer beside the question: one
                // never handed over has none there.
                if self
                    .log
                    .load(&StreamId::Pair(key.into()))
                    .iter()
                    .any(|line| line["id"] == request.id.as_str())
                {
                    self.exchange_thread_line(
                        key,
                        &request.to,
                        &format!("result:{}", request.id),
                        &request.reply,
                    )?;
                }
                self.nudge_handoff_result(key, &request);
                if linked::is_linked(&request.from) {
                    // Answering a linked teammate: the answer goes back over
                    // the link, and waits saved while the link is down.
                    if !self
                        .return_linked(&request.id, &request.from, &request.reply, request.failed)
                        .await
                    {
                        return Ok(());
                    }
                } else if !request.inline {
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
                            self.deliver_identified(&sender, &id, cause, text).await?;
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
    /// Takes up a request to a linked teammate: the pair's brake applies as
    /// it does to a local one, then it crosses the link. It stays queued while
    /// the link is down, and is running once the other desk has it.
    async fn forward_queued(self: &Arc<Self>, key: &str, request: &Request) -> Result<(), String> {
        if self.exchange_pair(key).is_none_or(|pair| pair.paused) {
            return Ok(());
        }
        let forwarded = self
            .forward_linked(
                &request.id,
                &request.from,
                &request.to,
                &request.message,
                request.intent,
                request.continues.as_deref(),
            )
            .await;
        let forwarded = match forwarded {
            Ok(forwarded) => forwarded,
            Err(error) => return self.set_exchange_reply(key, &request.id, error, true),
        };
        if !forwarded {
            return Ok(());
        }
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        let Some(r) = pair
            .requests
            .iter_mut()
            .find(|r| r.id == request.id && r.phase == Phase::Queued)
        else {
            return Ok(());
        };
        r.phase = Phase::Running;
        r.started = true;
        if !r.request_counted {
            r.request_counted = true;
            pair.exchanges += 1;
        }
        pair.paused = pair.exchanges >= EXCHANGE_CAP;
        self.save_pair(&pair)?;
        if pair.paused {
            self.exchange_card(&pair, ExchangePauseStatus::Pending);
        }
        Ok(())
    }

    /// Queues a request a linked desk sent for one of this desk's teammates.
    /// The same request sent again, after a channel closed before it was
    /// answered, is the one already queued.
    pub(super) fn accept_linked_request(
        self: &Arc<Self>,
        id: &str,
        from: &str,
        to: &str,
        message: &str,
        intent: Intent,
        continues: Option<String>,
    ) -> Result<(), String> {
        let _working = self.working()?;
        let key = thread_key(from, to).ok_or("Invalid teammate pair")?;
        {
            let _guard = lock(&self.exchange_lock);
            let pairs = self.exchange_pairs();
            if let Some(pair) = pairs
                .iter()
                .find(|pair| pair.requests.iter().any(|r| r.id == id))
            {
                if pair.id != key {
                    return Err("That request id belongs to another exchange.".into());
                }
                // Sent again after it was settled here: it is over, and says so.
                return match pair.requests.iter().find(|r| r.id == id) {
                    Some(r) if matches!(r.phase, Phase::Done | Phase::Stopped) => {
                        Err(if r.reply.is_empty() {
                            "That message was already answered.".to_string()
                        } else {
                            r.reply.clone()
                        })
                    }
                    _ => Ok(()),
                };
            }
            // A linked desk's teammates may have only so much waiting here
            // at once, whatever that desk sends.
            let desk = linked::split(from).map(|(desk, _)| desk);
            let waiting = pairs
                .iter()
                .flat_map(|pair| &pair.requests)
                .filter(|r| Self::exchange_pending(r))
                .filter(|r| linked::split(&r.from).map(|(desk, _)| desk) == desk)
                .count();
            if waiting >= LINKED_WAITING_MAX {
                return Err("Too many messages from that desk are already waiting here.".into());
            }
            let mut pair = pairs.into_iter().find(|p| p.id == key).unwrap_or(Pair {
                id: key.clone(),
                a: from.to_string(),
                b: to.to_string(),
                exchanges: 0,
                paused: false,
                requests: vec![],
            });
            pair.requests.push(Request {
                id: id.to_string(),
                from: from.to_string(),
                to: to.to_string(),
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
                reply_thread: None,
                reply_job: None,
                returned: false,
                lead_call: None,
                continues,
                released: false,
                about: None,
                own_thread: false,
                owed_to_thread: false,
                looped_in: false,
            });
            self.save_pair(&pair)?;
            lock(&self.exchange_leases).insert(
                id.to_string(),
                (self.request_lease(from), self.request_lease(to)),
            );
        }
        self.wake_exchange(&key);
        Ok(())
    }

    /// Saves the answer a linked desk sent for a request forwarded to it.
    pub(super) fn settle_linked_request(
        self: &Arc<Self>,
        desk_id: &str,
        id: &str,
        reply: String,
        failed: bool,
    ) -> Result<(), String> {
        let key = {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self
                .exchange_pairs()
                .into_iter()
                .find(|pair| {
                    pair.requests.iter().any(|r| {
                        r.id == id && linked::split(&r.to).is_some_and(|(desk, _)| desk == desk_id)
                    })
                })
                .ok_or("This desk sent no such request.")?;
            let request = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id)
                .expect("the pair was found by this request, under the exchange lock");
            if !matches!(request.phase, Phase::Queued | Phase::Running) {
                return Ok(());
            }
            request.phase = Phase::Reply;
            request.reply = reply;
            request.failed = failed;
            self.save_pair(&pair)?;
            pair.id
        };
        self.wake_exchange(&key);
        Ok(())
    }

    /// A link ended: every request still waiting to or from a teammate of
    /// that desk fails, and each sender here is told in a sentence.
    pub(super) fn stop_linked_exchanges(&self, desk_id: &str, desk_name: &str) {
        let theirs = |id: &str| linked::split(id).is_some_and(|(desk, _)| desk == desk_id);
        let _guard = lock(&self.exchange_lock);
        let (mut stopped, mut ended) = (Vec::new(), Vec::new());
        for mut pair in self.exchange_pairs() {
            if !theirs(&pair.a) && !theirs(&pair.b) {
                continue;
            }
            let mut changed = false;
            for request in &mut pair.requests {
                changed |= request.release(&mut ended);
                if !Self::exchange_pending(request) {
                    continue;
                }
                let linked_side = if theirs(&request.to) {
                    &request.to
                } else {
                    &request.from
                };
                let name = self
                    .participant(linked_side)
                    .map_or_else(|_| "That teammate".to_string(), |persona| persona.name);
                self.stop_request(
                    request,
                    format!("{name} is on {desk_name}, which isn't linked any more."),
                    None,
                );
                // Its desk is not linked to be told.
                request.returned = true;
                stopped.push(request.clone());
                changed = true;
            }
            if changed && let Err(error) = self.save_pair(&pair) {
                eprintln!("unlink exchange: {error}");
            }
            self.wake_owed(&pair);
        }
        self.mark_handoffs(&ended);
        self.exchange_notices(&stopped);
    }

    fn mark_returned(&self, key: &str, id: &str) -> Result<(), String> {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).ok_or("Exchange disappeared")?;
        if let Some(request) = pair.requests.iter_mut().find(|r| r.id == id) {
            request.returned = true;
        }
        self.save_pair(&pair)
    }
    fn set_request_thread(&self, key: &str, id: &str, thread: &str) -> Result<(), String> {
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
        let stream = StreamId::Pair(key.into());
        if self.log.load(&stream).iter().any(|v| v["id"] == id) {
            return Ok(());
        }
        // Through the shared write path, so both teammates' searches find it.
        self.threads().write(
            &ThreadId::pair(key),
            from,
            &json!({"kind":kind,"id":id,"ts":now_ms(),"text":text,"receipt":"sent"}),
        );
        Ok(())
    }
}

mod asides;
mod ended;
mod gates;
mod handover;
mod loop_in;
mod place;
mod questions;
mod revoked;

#[cfg(test)]
mod tests;
