//! Durable automatic exchanges. A pair record is the queue and the brake in
//! one append: accepting a message never relies on a task surviving the desk.
use super::sides::{HandoffStart, Start};
use super::turns::HandoffLine;
use super::{Room, lock, new_id, now_ms, peers};
use crate::contract::{
    DeliveryCause, DeliveryFrom, ExchangePauseStatus, HumanActionStatus, PeerStatus, SideEnd,
    TranscriptEvent,
};
use crate::driver::CapabilityLease;
use crate::log::{StreamId, thread};
use crate::paths::{thread_key, thread_participants};
use crate::thread::{Opener, ThreadId};
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
    #[serde(default)]
    inline: bool,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    result_consumed: bool,
    #[serde(default)]
    human_actions: Vec<HumanGate>,
    /// The work thread that serves a handoff: it runs on the teammate it was
    /// handed to, beside that teammate's DM. Absent on an ask, and on a
    /// handoff saved before handoffs had threads of their own.
    #[serde(default)]
    thread: Option<String>,
    /// The work thread the request was sent from, when it was sent from one:
    /// the answer comes back there instead of to the sender's DM.
    #[serde(default)]
    reply_thread: Option<String>,
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
    fn exchange_pairs(&self) -> Vec<Pair> {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .filter(|v| v["kind"] == "exchange_pair")
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    }
    pub(super) fn owns_exchange_thread(&self, key: &str) -> bool {
        self.exchange_pair(key).is_some()
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
        to: &str,
        message: &str,
        intent: Intent,
        capability: Option<CapabilityLease>,
        sent_from: Option<&str>,
    ) -> Result<peers::Sent, String> {
        self.enqueue_exchange(from, to, message, intent, capability, false, sent_from)
    }

    pub(crate) async fn send_peer_waiting(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
        intent: Intent,
        capability: Option<CapabilityLease>,
    ) -> Result<peers::DeliverResult, String> {
        let (caller, target, _) = self.checked(from, to, message)?;
        let key = thread_key(&caller.id, &target.id).ok_or("Invalid pair")?;
        if self.peers.answering_in(&key).is_some() {
            return Err("That thread is already answering; do not create a circular wait.".into());
        }
        let sent = self.enqueue_exchange(from, to, message, intent, capability, true, None)?;
        loop {
            let pair = self.exchange_pair(&key).ok_or("Exchange disappeared")?;
            let request = pair
                .requests
                .iter()
                .find(|r| r.id == sent.request_id)
                .ok_or("Request disappeared")?;
            if matches!(request.phase, Phase::Done | Phase::Stopped) {
                return if request.failed {
                    Err(request.reply.clone())
                } else {
                    Ok(peers::DeliverResult {
                        from: sent.to,
                        reply: request.reply.clone(),
                    })
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue_exchange(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
        intent: Intent,
        capability: Option<CapabilityLease>,
        inline: bool,
        sent_from: Option<&str>,
    ) -> Result<peers::Sent, String> {
        let _working = self.working()?;
        if let Some(capability) = &capability {
            capability.check()?;
        }
        let (caller, target, message) = self.checked(from, to, message)?;
        let key = thread_key(&caller.id, &target.id).ok_or("Invalid teammate pair")?;
        let id = new_id();
        let leases = (
            capability
                .unwrap_or_else(|| self.capability_lease(&caller.id))
                .scoped(),
            self.capability_lease(&target.id).scoped(),
        );
        {
            let _guard = lock(&self.exchange_lock);
            let mut pair = self.exchange_pair(&key).unwrap_or(Pair {
                id: key.clone(),
                a: caller.id.clone(),
                b: target.id.clone(),
                exchanges: 0,
                paused: false,
                requests: vec![],
            });
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
                inline,
                started: false,
                result_consumed: false,
                human_actions: vec![],
                thread: None,
                reply_thread: sent_from.map(str::to_string),
            });
            self.save_pair(&pair)?;
            lock(&self.exchange_leases).insert(id.clone(), leases);
        }
        self.wake_exchange(&key);
        Ok(peers::Sent {
            to: target.name,
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
        self.persona(a)?;
        self.persona(b)?;
        let key = thread_key(a, b)
            .filter(|_| a != b)
            .ok_or("Choose two different teammates")?;
        let _guard = lock(&self.exchange_lock);
        let mut pair = self
            .exchange_pair(&key)
            .ok_or("There is no exchange for this pair")?;
        if stop {
            for request in &mut pair.requests {
                if Self::exchange_pending(request) {
                    self.cancel_handoff(request);
                    request.phase = Phase::Stopped;
                    request.failed = true;
                    request.reply = "The person stopped this exchange.".into();
                    self.exchange_notice(request);
                }
            }
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
                        .persona(other)
                        .map(|p| p.name)
                        .unwrap_or_else(|_| other.clone()),
                    exchanges: pair.exchanges,
                    status,
                },
            );
        }
    }
    fn exchange_notice(&self, request: &Request) {
        self.write_value(&request.from, &json!({"kind":"notice", "id":format!("exchange-ended:{}",request.id),
            "ts":now_ms(), "level":"info", "text":format!("Message to {} ({}): {}",request.to,peers::about(&request.message),request.reply)}));
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
        for mut pair in self.exchange_pairs() {
            let mut changed = false;
            for request in &mut pair.requests {
                if !Self::exchange_pending(request)
                    || (!persona.is_some_and(|id| pair.a == id || pair.b == id)
                        && self.exchange_lease_current(&request.id))
                {
                    continue;
                }
                self.cancel_handoff(request);
                request.phase = Phase::Stopped;
                request.failed = true;
                request.reply = "Collaboration was revoked or a participant stopped. Send again if still needed.".into();
                self.exchange_notice(request);
                changed = true;
            }
            if changed && let Err(error) = self.save_pair(&pair) {
                eprintln!("revoke exchange: {error}");
            }
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
        if request.intent == Intent::Ask {
            if request.phase == Phase::Running {
                self.cancel_peer_exchange(&request.from, &request.to);
            }
            return;
        }
        // A handoff's turn is running in its own thread: stopping it stops that
        // turn and closes the thread. One whose turn is over has nothing to
        // stop, and what the person is saying to it now is theirs.
        if request.phase == Phase::Running
            && let Some(thread) = &request.thread
        {
            self.stop_work(thread, "Stopped.");
        }
    }
    pub(super) fn begin_handoff(&self, id: &str) -> Result<(), String> {
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
    /// Saves what a handoff's turn came to. True when the turn stopped to wait
    /// on the person, so the work is not over: the result is held until the
    /// answer has carried it on.
    pub(super) fn finish_handoff(&self, id: &str, reply: String, failed: bool) -> bool {
        let _guard = lock(&self.exchange_lock);
        for mut pair in self.exchange_pairs() {
            let Some(request) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id && r.phase == Phase::Running)
            else {
                continue;
            };
            // Only a cleanly completed turn establishes a safe suspension point.
            // A crash or a failed turn is uncertain even if it posted a card.
            let waiting = !failed && request.human_actions.iter().any(|g| !g.consumed);
            request.phase = if waiting {
                Phase::WaitingHuman
            } else {
                Phase::Reply
            };
            request.reply = reply;
            request.failed = failed;
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("save handoff result: {error}");
            }
            return waiting;
        }
        false
    }
    /// Keep routing in the pair record, not the lifetime of the asking turn:
    /// a request the person was asked in a handoff's thread is a gate the
    /// handoff's result waits behind.
    pub(super) fn link_handoff_human(
        &self,
        thread: Option<&str>,
        action: &str,
    ) -> Result<(), String> {
        let handoff = thread
            .and_then(|thread| self.sides.get(thread))
            .and_then(|side| side.handoff());
        let Some(id) = handoff else { return Ok(()) };
        let _guard = lock(&self.exchange_lock);
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

    /// The handoff a card was raised in, if the card is one a handoff waits
    /// behind: the exchange request that is answered by it.
    pub(super) fn handoff_awaiting(&self, persona: &str, action: &str) -> Option<String> {
        self.exchange_pairs()
            .iter()
            .flat_map(|p| &p.requests)
            .find(|r| r.to == persona && r.human_actions.iter().any(|g| g.id == action))
            .map(|r| r.id.clone())
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
        let caller = self.persona(&request.from)?;
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
            auth = self.authorize_collaboration(&caller, &target, &a, &b, true) => auth,
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
                    // The side session waiting inline died with the process. Return
                    // its eventual result to its owner's main conversation instead.
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
                    if request.phase != Phase::Running {
                        continue;
                    }
                    // A read receipt proves a turn began, not that it finished. Never
                    // replay potentially side-effecting work after an interrupted turn.
                    // A handoff saved before it had a thread was delivered into the
                    // recipient's DM, and its receipt is there.
                    let read = self.tape(&request.to).iter().any(|v| {
                        v["cause"]["kind"] == "handoff"
                            && v["cause"]["requestId"] == request.id.as_str()
                            && v["receipt"] == "read"
                    });
                    if request.intent == Intent::Ask || request.started || read {
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
                        pair.requests
                            .iter()
                            .all(|r| matches!(r.phase, Phase::Done | Phase::Stopped))
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
        let Some(request) = pair
            .requests
            .iter()
            .find(|r| !matches!(r.phase, Phase::Done | Phase::Stopped))
            .cloned()
        else {
            return Ok(());
        };
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
            Phase::Queued => {
                // A handoff waits its turn for a place on its recipient: a
                // teammate that has as many threads live as it may, every one of
                // them mid-turn, takes it up when one has finished or can be
                // parked. Nothing is refused and nothing is interrupted.
                if request.intent == Intent::Handoff && !self.sides.has_room(&request.to) {
                    return Ok(());
                }
                let caller = self.persona(&request.from)?;
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
                    auth = self.authorize_collaboration(
                        &caller,
                        &target,
                        &caller_capability,
                        &target_capability,
                        request.intent == Intent::Handoff,
                    ) => auth,
                };
                if let Err(error) = auth {
                    self.set_exchange_reply(key, &request.id, error, true)?;
                    return Ok(());
                }
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
                if request.intent == Intent::Handoff {
                    self.dispatch_handoff(key, &request).await?;
                } else {
                    let outcome = tokio::select! {
                        biased;
                        () = peers::capabilities_expired(&caller_capability, &target_capability) => {
                            self.stop_revoked_exchanges(None);
                            return Ok(());
                        }
                        result = async {
                            let asked = self.ask(&request.from, &request.to, &request.message)?;
                            self.exchange(asked, Some(caller_capability.clone())).await.map(|r| r.reply)
                        } => result,
                    };
                    let (reply, failed) = match outcome {
                        Ok(r) => (r, false),
                        Err(e) => (e, true),
                    };
                    self.set_exchange_reply(key, &request.id, reply, failed)?;
                }
            }
            Phase::Running => {
                // Only the recovery worker finds a handoff with no live thread;
                // a live dispatch has one by the time this looks.
                if request.intent == Intent::Handoff
                    && !request.started
                    && request
                        .thread
                        .as_deref()
                        .is_none_or(|thread| self.sides.get(thread).is_none())
                {
                    self.dispatch_handoff(key, &request).await?;
                }
            }
            Phase::WaitingHuman => {
                // The card is durable before dispatch. Repair an answer saved
                // just before the desk stopped, without replaying an old turn.
                for gate in request.human_actions.iter().filter(|g| !g.consumed) {
                    if let Some(TranscriptEvent::HumanAction {
                        reason,
                        status,
                        note,
                        thread,
                        ..
                    }) = self.human_card(&request.to, &gate.id)
                        && status != HumanActionStatus::Pending
                    {
                        self.hand_over_answer(&request.to, &gate.id, &reason, status, note, thread)
                            .await?;
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
                        self.exchange_card(&pair, ExchangePauseStatus::Pending)
                    }
                }
                let target = self.persona(&request.to)?;
                let cause = DeliveryCause::Peer {
                    request_id: Some(request.id.clone()),
                    persona_id: target.id,
                    name: target.name,
                    thread_key: key.into(),
                    status: if request.failed {
                        PeerStatus::Failed
                    } else {
                        PeerStatus::Done
                    },
                    about: peers::about(&request.message),
                };
                if request.intent == Intent::Handoff {
                    self.exchange_thread_line(
                        key,
                        &request.to,
                        &format!("result:{}", request.id),
                        &request.reply,
                    )?;
                }
                if !request.inline {
                    let from = self.delivery_source(&request.from, &cause);
                    let id = format!("exchange-result:{}", request.id);
                    match request
                        .reply_thread
                        .as_deref()
                        .filter(|thread| self.work_is_open(thread))
                    {
                        // Sent from a work thread: the answer is its to hear.
                        Some(thread) => {
                            self.deliver_into_work(
                                thread,
                                &id,
                                cause,
                                from,
                                request.reply.clone(),
                                None,
                            )
                            .await?;
                        }
                        None => {
                            self.deliver_identified(
                                &request.from,
                                &id,
                                cause,
                                request.reply.clone(),
                            )
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
    /// Starts a handoff as a work thread on the teammate it was handed to. The
    /// handoff is that thread's first message, and its result is the reply the
    /// thread gives when it has done the work.
    async fn dispatch_handoff(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        let caller = self.persona(&request.from)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        // A thread this handoff had opened before the desk restarted never
        // began its turn: it is put away and the work starts in a new one.
        if let Some(earlier) = &request.thread {
            let _ = self.archive_side(
                earlier,
                SideEnd::Stopped,
                Some("The desk restarted before this began.".to_string()),
            );
        }
        let side_id = new_id();
        self.set_handoff_thread(key, &request.id, &side_id)?;
        let started = self.bring_up(
            &request.to,
            Start {
                side_id: side_id.clone(),
                title: super::sides::title_of(&request.message),
                started: now_ms(),
                opener: Some(Opener {
                    persona_id: caller.id.clone(),
                    name: caller.name.clone(),
                }),
                fresh: true,
                saved: None,
                handoff: Some(HandoffStart {
                    key: key.to_string(),
                    request: request.id.clone(),
                }),
            },
        );
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
                Some(HandoffLine {
                    request: request.id.clone(),
                    answer: None,
                }),
            )
            .await;
        // The line is queued first, so the agent's start finds it waiting.
        self.launch(launch);
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
        let stream = StreamId::Pair(key.into());
        if self.log.load(&stream).iter().any(|v| v["id"] == id) {
            return Ok(());
        }
        self.log
            .append(
                &stream,
                &json!({"kind":kind,"id":id,"ts":now_ms(),"text":text,"receipt":"sent"}),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
