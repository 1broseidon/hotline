//! Durable automatic exchanges. A pair record is the queue and the brake in
//! one append: accepting a message never relies on a task surviving the desk.
use super::{Room, lock, new_id, now_ms, peers};
use crate::contract::{DeliveryCause, ExchangePauseStatus, PeerStatus, TranscriptEvent};
use crate::driver::CapabilityLease;
use crate::log::{StreamId, thread};
use crate::paths::{thread_key, thread_participants};
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
    ) -> Result<peers::Sent, String> {
        self.enqueue_exchange(from, to, message, intent, capability, false)
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
        let sent = self.enqueue_exchange(from, to, message, intent, capability, true)?;
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

    fn enqueue_exchange(
        self: &Arc<Self>,
        from: &str,
        to: &str,
        message: &str,
        intent: Intent,
        capability: Option<CapabilityLease>,
        inline: bool,
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
            let id = format!("exchange-paused:{}:{whose}", pair.id);
            if status != ExchangePauseStatus::Pending
                && !self
                    .tape(whose)
                    .iter()
                    .any(|e| e["id"] == id && e["status"] == "pending")
            {
                continue;
            }
            self.write(
                whose,
                &TranscriptEvent::ExchangePaused {
                    id,
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
    pub(super) fn handoff_live(&self, id: &str) -> bool {
        self.exchange_lease_current(id)
            && self.exchange_pairs().iter().any(|p| {
                p.requests
                    .iter()
                    .any(|r| r.id == id && r.phase == Phase::Running)
            })
    }
    fn exchange_lease_current(&self, id: &str) -> bool {
        lock(&self.exchange_leases)
            .get(id)
            .is_none_or(|(a, b)| a.is_current() && b.is_current())
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
        if request.intent == Intent::Handoff
            && let Ok(session) = self.session(&request.to)
        {
            let active = lock(&session.active_handoff);
            if active.as_deref() == Some(&request.id) {
                session.driver.cancel();
            }
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
    pub(super) fn finish_handoff(&self, id: &str, reply: String, failed: bool) {
        let _guard = lock(&self.exchange_lock);
        for mut pair in self.exchange_pairs() {
            let Some(request) = pair
                .requests
                .iter_mut()
                .find(|r| r.id == id && r.phase == Phase::Running)
            else {
                continue;
            };
            request.phase = Phase::Reply;
            request.reply = reply;
            request.failed = failed;
            if let Err(error) = self.save_pair(&pair) {
                eprintln!("save handoff result: {error}");
            }
            break;
        }
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
                    if request.phase != Phase::Running {
                        continue;
                    }
                    // A read receipt proves a turn began, not that it finished. Never
                    // replay potentially side-effecting work after an interrupted turn.
                    let read = self.tape(&request.to).iter().any(|v| {
                        v["id"] == format!("handoff:{}", request.id) && v["receipt"] == "read"
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
            && request.phase != Phase::Running
            && !(request.phase == Phase::Reply && request.reply_counted)
        {
            return Ok(());
        }
        match request.phase {
            Phase::Queued => {
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
                if request.intent == Intent::Handoff {
                    let id = format!("handoff:{}", request.id);
                    // Only the recovery worker needs to dispatch an existing unread
                    // record; live dispatch is tracked by the session's pending ids.
                    if !self.handoff_queued(&request.to, &id) {
                        self.dispatch_handoff(key, &request).await?;
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
                    self.deliver_identified(
                        &request.from,
                        &format!("exchange-result:{}", request.id),
                        cause,
                        request.reply.clone(),
                    )
                    .await?;
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
    fn set_exchange_reply(
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
    async fn dispatch_handoff(
        self: &Arc<Self>,
        key: &str,
        request: &Request,
    ) -> Result<(), String> {
        let caller = self.persona(&request.from)?;
        self.exchange_thread_line(key, &request.from, &request.id, &request.message)?;
        self.deliver_identified(
            &request.to,
            &format!("handoff:{}", request.id),
            DeliveryCause::Handoff {
                request_id: request.id.clone(),
                persona_id: caller.id,
                name: caller.name,
                thread_key: key.into(),
                about: peers::about(&request.message),
            },
            request.message.clone(),
        )
        .await
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
        let stream = StreamId::Thread(key.into());
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
