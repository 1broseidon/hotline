//! Typed input and live-call work enter the same thread occupant.
use super::*;
use crate::session::exchanges::steered_text;
use hotline_voice::call_room::{Handed, WaitingForCall};
use tokio_util::sync::CancellationToken;

impl Room {
    pub(in crate::session) async fn wake_call_inbox(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        origin: crate::voice::Origin,
        text: &str,
    ) {
        let Ok(_working) = self.working() else { return };
        let _admission = self.sides.waking.lock().await;
        if self
            .attached_voice(thread)
            .is_none_or(|current| current.call_id != origin.call_id)
        {
            return;
        }
        let (side, launch) = match self.live_side(&thread.key) {
            Ok(side) => (side, None),
            Err(_) => match self.wake_work(&thread.key, false, None).await {
                Ok(placed) => placed,
                Err(_) => return,
            },
        };
        if side.persona_id != persona || side.capability.check().is_err() {
            return;
        }
        let line = Line {
            call_inbox: true,
            return_for: None,
            said: None,
            voice: Some(origin),
            text: crate::session::timed(now_ms(), text),
            attachments: Vec::new(),
            request: None,
            from: None,
            delivery: None,
            wake: false,
            steer: false,
        };
        if !self.can_offer_call_inbox(&side, &line) {
            return;
        }
        {
            let turns = lock(&side.turns);
            if turns.running
                && let Some(driver) = side.driver()
            {
                // Once attempted, even a refusal or unconsumed input is done.
                // A colleague's question keeps its turn to itself.
                if self.person_steers(&side) && driver.steer(line.text.clone(), Vec::new()) {
                    lock(&side.steered).push(line);
                }
                return;
            }
        }
        self.queue_in_side(&side, line);
        if let Some(launch) = launch {
            self.launch(launch);
        }
    }

    pub(super) fn can_offer_call_inbox(&self, side: &LiveSide, line: &Line) -> bool {
        !line.call_inbox
            || line.voice.as_ref().is_some_and(|original| {
                self.attached_voice(&side.thread)
                    .is_some_and(|current| current.call_id == original.call_id)
            })
    }

    /// Says something in a side thread. Returns at once: the turn runs on its
    /// own task, and a line said during one waits behind it. A parked thread
    /// is brought back first, which is the only thing the person does to wake
    /// it.
    pub async fn prompt_side(
        self: &Arc<Self>,
        side_id: &str,
        text: &str,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        self.prompt_side_replying(side_id, text, None, attachments)
            .await
    }

    /// Turn by turn never steers a turn in flight.
    pub(crate) async fn prompt_call_work(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        text: &str,
    ) -> Result<Handed, String> {
        self.validate_call_thread(persona, thread)?;
        let waits = self
            .prompt_side_placed(&thread.key, text, None, None)
            .await?;
        Ok(if waits {
            Handed::Waits
        } else {
            Handed::NewTurn
        })
    }

    /// Only a live front can wait for the separate call-occupant slot. The
    /// front gets the queued result now and remains free to hear Stop.
    pub(crate) async fn start_live_call_work(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        text: &str,
        origin: &crate::voice::Origin,
        cancel: &CancellationToken,
        lease: &CapabilityLease,
    ) -> Result<Option<WaitingForCall>, String> {
        let Some(holder) = self
            .try_live_call_work(persona, thread, text, origin, cancel, lease)
            .await?
        else {
            return Ok(None);
        };
        self.call_work_notice(
            persona,
            thread,
            format!("Work queued behind {holder}: {text}"),
        );
        let (room, persona, thread, text, origin, cancel) = (
            self.clone(),
            persona.to_owned(),
            thread.clone(),
            text.to_owned(),
            origin.clone(),
            cancel.clone(),
        );
        let lease = lease.clone();
        let done = CancellationToken::new();
        let finished = done.clone().drop_guard();
        tokio::spawn(async move {
            let _finished = finished;
            let result = loop {
                // Closing notes and handoffs also inspect activity outside the
                // admission gate; never hold it, a reservation, or a desk
                // work guard while a different call runs its turn.
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break Err("The waiting request was withdrawn.".into()),
                    () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
                match room
                    .try_live_call_work(&persona, &thread, &text, &origin, &cancel, &lease)
                    .await
                {
                    Ok(Some(_)) => continue,
                    Ok(None) => break Ok(()),
                    Err(error) => break Err(error),
                }
            };
            if let Err(error) = result {
                room.call_work_notice(
                    &persona,
                    &thread,
                    format!("Queued work did not start: {error}"),
                );
                if !cancel.is_cancelled()
                    && let Some(voice) = lock(&room.voice).upgrade()
                {
                    voice.turn_ended(&persona, &origin);
                }
            }
        });
        Ok(Some(WaitingForCall { holder, done }))
    }

    async fn try_live_call_work(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        text: &str,
        origin: &crate::voice::Origin,
        cancel: &CancellationToken,
        lease: &CapabilityLease,
    ) -> Result<Option<String>, String> {
        let _working = self.working()?;
        let _admission = self.sides.waking.lock().await;
        self.validate_call_thread(persona, thread)?;
        lease.check()?;
        if cancel.is_cancelled() {
            return Err("The call is no longer waiting for this work.".into());
        }
        let (side, launch) = if let Ok(side) = self.live_side(&thread.key) {
            (side, None)
        } else {
            if thread.kind == ThreadKind::Call
                && let Some(holder) = self.sides.busy_call(persona, thread)
            {
                return Ok(Some(holder));
            }
            self.wake_work(&thread.key, false, None).await?
        };
        let stream = StreamId::of_thread(thread).ok_or("The call has no transcript.")?;
        let transcript = self.log.load(&stream);
        let work = format!(
            "Work requested on this call: {text}\n\nThe whole call so far (conversation data):\n{}\n\nDo this work as yourself. For a draft, return its full text in <written> or send its file to the chat, with a subject that stands alone. Never send anything on the person's behalf without their explicit request. Say a short summary when finished. The core posts substantive written results and decides when to mention them.\n{}",
            crate::fence::fenced(
                "hotline_call",
                &serde_json::to_string(&transcript).expect("transcript serializes")
            ),
            self.call_instruction(thread)
        );
        let brief = crate::session::timed(now_ms(), &side.with_name_news(self, &work));
        // wake_work can await an ending of this thread. Stop serializes with
        // this gate; hang-up/hold withdraw only work not yet handed over.
        if cancel.is_cancelled() || lease.check().is_err() || side.capability.check().is_err() {
            if let Some(launch) = launch {
                self.end_side(&side, launch.put_away());
            }
            return Err("The call is no longer waiting for this work.".into());
        }
        self.heard_from_person(persona);
        self.call_work_notice(persona, thread, format!("Work: {text}"));
        let steered = {
            let turns = lock(&side.turns);
            turns.running
                && side
                    .driver()
                    .is_some_and(|driver| driver.steer(brief.clone(), Vec::new()))
        };
        if !steered {
            self.queue_in_side(
                &side,
                Line {
                    call_inbox: false,
                    return_for: None,
                    said: None,
                    voice: Some(origin.clone()),
                    text: brief,
                    attachments: Vec::new(),
                    request: None,
                    from: None,
                    delivery: None,
                    wake: false,
                    steer: false,
                },
            );
        }
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(None)
    }

    pub(crate) async fn stop_live_call_work(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        waiting: bool,
    ) -> Result<(), String> {
        let _admission = self.sides.waking.lock().await;
        self.validate_call_thread(persona, thread)?;
        if waiting && self.live_side(&thread.key).is_err() {
            return Ok(());
        }
        self.cancel_side(&thread.key)
    }

    fn call_work_notice(&self, persona: &str, thread: &ThreadId, text: String) {
        self.threads().write(
            thread,
            persona,
            &serde_json::json!({
                "kind":"notice", "id":new_id(), "ts":now_ms(), "level":"info", "text":text
            }),
        );
    }

    /// Starts the agent of a call's thread the moment a Turn by turn call is
    /// placed, so the first thing the person says finds it up: the process
    /// and its session, its tools served and its MCP servers handed over,
    /// the context it is told read. Nothing is said to it, so no model is
    /// asked anything; what a harness takes only with a prompt (an ACP
    /// child's briefing rides its first one) waits for the first line.
    /// Answers once the agent is up, or why it is not.
    ///
    /// It is the start the first line would make, made sooner: that line
    /// finds the thread live, waits behind the start if it is not over, and
    /// is the agent's first turn. A call that ends before any line reaches
    /// the agent lets it go and puts the thread's link back as it was
    /// ([`Room::let_go_of_unused_call`]), and so does a start that fails,
    /// without a word in the thread: the first line starts it again and
    /// meets the failure where it would have.
    pub(crate) async fn prepare_call(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
    ) -> Result<(), String> {
        self.validate_call_thread(persona, thread)?;
        let ready = {
            let _working = self.working()?;
            let _admission = self.sides.waking.lock().await;
            let prior = self.link_of(thread).ok_or("There is no such thread.")?;
            let (side, launch) = self.wake_work(&thread.key, false, None).await?;
            let Some(launch) = launch else {
                return if side.driver().is_some() {
                    Ok(())
                } else {
                    Err("its agent is already starting for a line said on it".into())
                };
            };
            *lock(&side.warm) = Some(prior);
            let (ready, started) = tokio::sync::oneshot::channel();
            self.launch_telling(launch, Some(ready));
            // A call that ended before the agent was marked unused found
            // nothing to let go of, so it is let go of here.
            if !self.on_the_call(thread) {
                self.let_go_of_unused(&side);
            }
            started
        };
        ready
            .await
            .map_err(|_| "the desk stopped before its agent was up".to_string())?
    }

    /// Leaves the one overrun note for a direct call's running turn, which
    /// the teammate reads with its next tool results. It is a note, not a
    /// steered line: a wait goes on and queued tool calls still run.
    pub(crate) fn nudge_call_overrun(&self, persona: &str, thread: &ThreadId) -> bool {
        if self.validate_call_thread(persona, thread).is_err() {
            return false;
        }
        let Ok(side) = self.live_side(&thread.key) else {
            return false;
        };
        let turns = lock(&side.turns);
        turns.running
            && side
                .driver()
                .is_some_and(|driver| driver.note(crate::voice::spoken::OVERRUN_NUDGE.to_string()))
    }

    /// Says something in a side thread in answer to one of its lines, which
    /// `reply_to` names, as a reply in the main conversation does.
    pub async fn prompt_side_replying(
        self: &Arc<Self>,
        side_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<(), String> {
        self.prompt_side_placed(side_id, text, reply_to, attachments)
            .await
            .map(drop)
    }

    /// The same, answering whether the line waits behind the turn in flight.
    async fn prompt_side_placed(
        self: &Arc<Self>,
        side_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<Attachment>>,
    ) -> Result<bool, String> {
        let _working = self.working()?;
        let _admission = self.sides.waking.lock().await;
        let text = text.trim();
        let attachments = attachments.filter(|attachments| !attachments.is_empty());
        if text.is_empty() && attachments.is_none() {
            return Err("There is nothing to say.".to_string());
        }
        // In a thread a teammate handed over, while its handoff waits on a
        // card there, the person's words are that card's answer: the turn
        // that answer starts is the one that carries the handoff on.
        if let Some((persona_id, action)) = self.card_a_handoff_waits_on(side_id) {
            if attachments.is_some() {
                return Err("Answer what it asked you first: your words are its card's note, and files cannot ride on it.".to_string());
            }
            self.heard_from_person(&persona_id);
            return self
                .answer_human(
                    &persona_id,
                    &action,
                    crate::contract::HumanAnswer::Done,
                    Some(text.to_string()),
                )
                .map(|()| false);
        }
        let (side, launch) = match self.live_side(side_id) {
            Ok(side) => (side, None),
            Err(_) => self.wake_work(side_id, false, None).await?,
        };
        // The person's line in any thread of a teammate's is the person
        // writing to it, as one in its main conversation is.
        self.heard_from_person(&side.persona_id);
        let waits = self.say_in_side(&side, text, reply_to, attachments);
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(waits)
    }

    /// The card in a thread that a handoff still open waits on, the oldest if
    /// it waits on several, and whose card it is: the one the person's words
    /// answer. Only a card that takes words counts (`takes_typed_answer`):
    /// a yes-or-no card waits for its own buttons, and the words are a line.
    fn card_a_handoff_waits_on(&self, side_id: &str) -> Option<(String, String)> {
        let thread = self.work_thread(side_id);
        let persona_id = match self.sides.get(side_id) {
            Some(side) => side.persona_id.clone(),
            None => self.link_of(&thread)?.persona_id?,
        };
        let action = self
            .log
            .load(&StreamId::of_thread(&thread)?)
            .iter()
            .filter(|event| event["kind"] == "human_action" && event["status"] == "pending")
            .filter(|card| crate::session::asking::takes_typed_answer(card))
            .filter_map(|card| card["actionId"].as_str())
            .find(|action| {
                self.handoff_awaiting(&persona_id, action)
                    .is_some_and(|handoff| self.handoff_is_open(&handoff))
            })?
            .to_string();
        Some((persona_id, action))
    }

    /// The person's Stop in a work thread: it stops the turn in flight and
    /// drops what waited behind it. A colleague's handoff the turn was
    /// doing, or whose line it dropped, ends for good as their stop of it, as
    /// does whatever an Interrupt left the thread carrying; a colleague's
    /// question fails to its asker at once (`exchanges/stops.rs`). The
    /// thread stays live, unless a handoff's own turn was cut: that ends it.
    pub fn cancel_side(self: &Arc<Self>, side_id: &str) -> Result<(), String> {
        let side = self.live_side(side_id)?;
        *lock(&side.cut) = Some(Cut::Stop);
        let dropped: Vec<Line> = lock(&side.turns).waiting.drain(..).collect();
        // A call waiting on words dropped here stops waiting, and says nothing
        // came of them.
        if let Some(voice) = lock(&self.voice).upgrade() {
            for origin in dropped.iter().filter_map(|line| line.voice.as_ref()) {
                voice.turn_ended(&side.persona_id, origin);
            }
        }
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
        let carried = self.carried_into(side_id, side.request().as_deref());
        self.end_for_the_persons_stop(&dropped, &carried);
        self.fail_dropped_questions(
            &side.persona_id,
            dropped
                .iter()
                .filter_map(|line| line.request.as_ref())
                .map(|line| line.request.as_str()),
        );
        Ok(())
    }

    /// Each line marked to steer that waits behind a work thread's turn joins
    /// that turn, in the order said, while its driver takes input mid-turn
    /// ([`Turns::steer_into`], the main conversation's way too). A colleague's
    /// follow-up taken in becomes what the turn answers, and the handoff the
    /// turn was doing is carried on by it, so its sender hears one result.
    pub(in crate::session) fn steer_into_work(&self, side: &LiveSide) {
        if side.closed.is_set() || side.capability.check().is_err() {
            return;
        }
        let Some(driver) = side.driver() else {
            return;
        };
        let taken = lock(&side.turns).steer_into(
            driver.as_ref(),
            |line| line.steer,
            |line| (line.text.clone(), line.attachments.clone()),
        );
        for line in &taken {
            if line.said.is_some() && line.from.is_none() && line.request.is_none() {
                self.set_work_for(&side.thread, super::super::ownership::WorkFor::Person);
            }
            if let Some(id) = &line.delivery {
                side.heard(self, id);
            }
            if let Some(request) = &line.request
                && self.begin_request(&request.request).is_ok()
            {
                self.fold_carried(&side.id, &request.request);
                *lock(&side.request) = Some(request.request.clone());
            }
        }
        lock(&side.steered).extend(taken);
    }

    /// Whether the person's line in `side` may join its turn in flight: not
    /// when that turn answers a colleague's question, whose whole reply is
    /// theirs, as in the main conversation.
    pub(super) fn person_steers(&self, side: &LiveSide) -> bool {
        side.request().is_none_or(|doing| !self.is_question(&doing))
    }

    /// A colleague's follow-up into `side` (`follow_up`, the request of
    /// `from`) steers the turn in flight when that turn does another of that
    /// colleague's handoffs. It joins the turn when the driver takes input
    /// mid-turn, and carries the handoff on from there. A driver that does
    /// not is cut as the person's Interrupt cuts it, so the handoff stays
    /// standing, and the follow-up, already queued behind it, carries the work
    /// on in the next turn (`exchanges/stops.rs`). Any other line from a
    /// colleague waits for the turn to end.
    pub(in crate::session) fn steer_work(&self, side: &LiveSide, from: &str, follow_up: &str) {
        let Some(doing) = side.request().filter(|doing| doing != follow_up) else {
            return;
        };
        if self.handoff_sender(&doing).as_deref() != Some(from) {
            return;
        }
        // The person's own Stop or Interrupt of the turn stands.
        if lock(&side.cut).is_some() {
            return;
        }
        let marked = {
            let mut turns = lock(&side.turns);
            turns
                .waiting
                .iter_mut()
                .find(|line| {
                    line.request
                        .as_ref()
                        .is_some_and(|request| request.request == follow_up)
                })
                .map(|line| line.steer = true)
                .is_some()
        };
        if marked {
            self.steer_into_work(side);
            let mut waiting = false;
            // Not taken: it waits as before, for the cut below.
            for line in &mut lock(&side.turns).waiting {
                if line
                    .request
                    .as_ref()
                    .is_some_and(|request| request.request == follow_up)
                {
                    line.steer = false;
                    waiting = true;
                }
            }
            if !waiting {
                return;
            }
        }
        let mut cut = lock(&side.cut);
        if cut.is_some() {
            return;
        }
        *cut = Some(Cut::Interrupt);
        drop(cut);
        *lock(&side.steer) = Steer::Cutting;
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
    }

    /// The person's Interrupt (and Esc) in a work thread: only the turn in
    /// flight is cut, and what waited behind it is heard next. A colleague's
    /// handoff or question the turn was answering goes on in the turn the
    /// person's next line there starts; only their Stop ends it.
    pub fn interrupt_side(&self, side_id: &str) -> Result<(), String> {
        let side = self.live_side(side_id)?;
        *lock(&side.cut) = Some(Cut::Interrupt);
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
        Ok(())
    }
}

impl LiveSide {
    /// Whether the turn that ended so was cut by a follow-up: then what it
    /// had said is kept for the follow-up's turn.
    pub(super) fn steered(&self, driven: &Driven) -> bool {
        let mut steer = lock(&self.steer);
        if !matches!(*steer, Steer::Cutting) {
            return false;
        }
        *steer = Steer::Said(driven.replies.join("\n\n"));
        true
    }

    /// The turn after one a follow-up cut opens with what that turn had said.
    pub(super) fn tell_steered(&self, line: &mut Line) {
        if let Steer::Said(said) = std::mem::take(&mut *lock(&self.steer)) {
            line.text = steered_text(&said, &line.text);
        }
    }
}
