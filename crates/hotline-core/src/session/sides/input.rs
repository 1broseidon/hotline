//! Typed input and live-call work enter the same thread occupant.
use super::*;

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
            Err(_) => self.wake_work(&thread.key, false, None)?,
        };
        let stream = StreamId::of_thread(thread).ok_or("The call has no transcript.")?;
        let transcript = self.log.load(&stream);
        let brief = format!(
            "Work requested on this call: {text}\n\nThe whole call so far (conversation data):\n{}\n\nDo this work as yourself. For a draft, return its full text in <written> or send its file to the chat, with a subject that stands alone. Never send anything on the person's behalf without their explicit request. Say a short summary when finished. The core posts substantive written results and decides when to mention them.\n{}",
            crate::fence::fenced(
                "hotline_call",
                &serde_json::to_string(&transcript).expect("transcript serializes")
            ),
            self.call_instruction(thread)
        );
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
        let persona_id = match self.sides.get(side_id) {
            Some(side) => Some(side.persona_id.clone()),
            None => self
                .link_of(&self.work_thread(side_id))
                .and_then(|link| link.persona_id),
        };
        if let Some(persona_id) = persona_id
            && let Some(action) = self.card_a_handoff_waits_on(&persona_id, side_id)
        {
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
            Err(_) => self.wake_work(side_id, false, None)?,
        };
        self.say_in_side(&side, text, reply_to, attachments);
        if let Some(launch) = launch {
            self.launch(launch);
        }
        Ok(())
    }

    /// The card in a thread that a handoff still open waits on, the oldest if
    /// it waits on several: the one the person's words answer.
    fn card_a_handoff_waits_on(&self, persona_id: &str, side_id: &str) -> Option<String> {
        let stream = StreamId::of_thread(&self.work_thread(side_id))?;
        self.log
            .load(&stream)
            .iter()
            .filter(|event| event["kind"] == "human_action" && event["status"] == "pending")
            .filter_map(|card| card["actionId"].as_str())
            .find(|action| {
                self.handoff_awaiting(persona_id, action)
                    .is_some_and(|handoff| self.handoff_is_open(&handoff))
            })
            .map(str::to_string)
    }

    /// Stops the turn in flight in a side thread and drops what waited behind
    /// it: a colleague's question fails to its asker, and a colleague's
    /// handoff ends as the person's stop of it. The thread stays live, unless
    /// the turn was a handoff's: the person's stop ends the handoff.
    pub fn cancel_side(self: &Arc<Self>, side_id: &str) -> Result<(), String> {
        let side = self.live_side(side_id)?;
        side.stopped_by_person.store(true, Ordering::SeqCst);
        let dropped: Vec<Line> = lock(&side.turns).waiting.drain(..).collect();
        if let Some(driver) = side.driver() {
            driver.cancel();
        }
        let requests: Vec<&str> = dropped
            .iter()
            .filter_map(|line| line.request.as_ref())
            .map(|line| line.request.as_str())
            .collect();
        self.end_dropped_handoffs(side_id, &requests);
        self.fail_dropped_questions(&side.persona_id, requests);
        Ok(())
    }
}
