//! Typed input and live-call work enter the same thread occupant.
use super::*;
use crate::session::exchanges::steered_text;

impl Room {
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

    /// Live work keeps one occupant. Its brief includes everything said on
    /// the call, including speech since an ACP session last ran a turn.
    pub(crate) async fn prompt_call_work(
        self: &Arc<Self>,
        persona: &str,
        thread: &ThreadId,
        text: &str,
    ) -> Result<(), String> {
        self.validate_call_thread(persona, thread)?;
        let live_voice = self
            .attached_voice(thread)
            .is_some_and(|origin| origin.front);
        if !live_voice {
            return self.prompt_side(&thread.key, text, None).await;
        }
        let _working = self.working()?;
        let _admission = self.sides.waking.lock().await;
        let (side, launch) = match self.live_side(&thread.key) {
            Ok(side) => (side, None),
            Err(_) => self.wake_work(&thread.key, false, None).await?,
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
        let brief = side.with_name_news(self, &work);
        self.threads().write(
            thread,
            persona,
            &serde_json::json!({
                "kind":"notice", "id":new_id(), "ts":now_ms(), "level":"info",
                "text":format!("Work: {text}")
            }),
        );
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
                    voice: self.attached_voice(thread),
                    text: brief,
                    attachments: Vec::new(),
                    request: None,
                    from: None,
                    delivery: None,
                    wake: false,
                },
            );
        }
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(())
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
            return self.answer_human(
                &persona_id,
                &action,
                crate::contract::HumanAnswer::Done,
                Some(text.to_string()),
            );
        }
        let (side, launch) = match self.live_side(side_id) {
            Ok(side) => (side, None),
            Err(_) => self.wake_work(side_id, false, None).await?,
        };
        self.say_in_side(&side, text, reply_to, attachments);
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(())
    }

    /// The card in a thread that a handoff still open waits on, the oldest if
    /// it waits on several, and whose card it is: the one the person's words
    /// answer.
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

    /// A colleague's follow-up into `side` (`follow_up`, the request of
    /// `from`) steers the turn in flight when that turn does another of that
    /// colleague's handoffs: the turn is cut as the person's Interrupt cuts
    /// it, so the handoff stays standing, and the follow-up, already queued
    /// behind it, carries the work on (`exchanges/stops.rs`). Any other line
    /// waits for the turn to end; the person's own line among them, which is
    /// theirs and never becomes a colleague's result.
    pub(in crate::session) fn steer_work(&self, side: &LiveSide, from: &str, follow_up: &str) {
        let Some(doing) = side.request().filter(|doing| doing != follow_up) else {
            return;
        };
        if self.handoff_sender(&doing).as_deref() != Some(from) {
            return;
        }
        // The person's own Stop or Interrupt of the turn stands.
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
