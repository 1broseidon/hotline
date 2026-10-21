//! A call's turn: hearing the person, answering through the dispatcher or a
//! teammate, and keeping what was said.

use super::dispatcher::Context;
use super::ledger::{self, Kind};
use super::metering::BUDGET_ERROR;
use super::record::Speaker;
use super::speech::{self, Clip};
use super::spoken::sentences;
use super::synthesis::{goodbye, speech_ready};
use super::{
    BUDGET_LINE, Calls, Delivery, ERROR_LINE, MAX_FAILURES, RETRY_LINE, REWRITE_WAIT, Reply,
    TranscriptSource, Work, lock, realtime, spoken,
};
use hotline_contract::{VoiceEndReason, VoiceEvent, VoiceState};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl Calls {
    /// Playback context for the next call-thread turn, separate from the caller's words.
    pub fn take_interruption(&self, id: &str) -> Option<String> {
        self.change(id, |call| Ok(call.speech_line.pending.take()))
            .ok()
            .flatten()
    }

    pub fn file_posted(&self, thread: &hotline_contract::ThreadId, name: &str) {
        if let Some(origin) = self.thread_origin(thread) {
            let _ = self.change(&origin.call_id, |call| {
                call.conversation.file_sent(name);
                Ok(())
            });
        }
    }

    /// State travels with each Legacy turn, outside the saved caller words.
    pub fn turn_instruction(&self, id: &str) -> String {
        self.change(id, |call| {
            Ok(format!(
                "{}\n{}",
                spoken::CONTRACT,
                call.conversation.instruction(false)
            ))
        })
        .unwrap_or_else(|_| spoken::CONTRACT.into())
    }

    fn narration_instruction(&self, id: Option<&str>, written: &str) -> String {
        id.and_then(|id| {
            self.change(id, |call| {
                let pointer = call.conversation.pointer(written);
                Ok(call.conversation.instruction(pointer))
            })
            .ok()
        })
        .unwrap_or_else(|| spoken::CallContext::default().instruction(false))
    }

    pub(super) fn emit(&self, id: &str, event: VoiceEvent) {
        let _ = self.change(id, |call| {
            if let VoiceEvent::Said { id, text } = &event
                && call.front.is_none()
            {
                call.waiting_sound = true;
                call.speech_line.update(id, text);
            }
            let _ = call.events.send(event);
            Ok(())
        });
    }

    pub(super) fn system_line(
        &self,
        id: &str,
        text: &str,
        audio: &[u8],
        interrupted: Option<&CancellationToken>,
    ) {
        let line = self.keep(id, Speaker::Voice, text);
        self.emit(
            id,
            VoiceEvent::Said {
                id: line.clone(),
                text: text.into(),
            },
        );
        self.clip(
            id,
            &line,
            0,
            true,
            &Clip {
                mime: "audio/wav".into(),
                bytes: audio.to_vec(),
            },
            interrupted,
        );
    }

    pub(super) async fn run(
        self: Arc<Self>,
        id: String,
        context: Context,
        mut work: mpsc::Receiver<Work>,
    ) {
        let mut failures = 0u8;
        loop {
            let next = tokio::select! { biased; () = context.cancel.cancelled() => None, work = work.recv() => work };
            let Some(next) = next else {
                break;
            };
            let result = match &next {
                Work::Text {
                    seq,
                    text,
                    source,
                    speech,
                } => {
                    self.respond(
                        &id,
                        &context.for_utterance(),
                        *seq,
                        text.clone(),
                        *source,
                        speech,
                    )
                    .await
                }
                Work::Live {
                    seq,
                    input,
                    committed,
                    stt,
                    speech,
                } => {
                    let receiver = lock(input).take().expect("one live consumer");
                    let transcribed = tokio::select! {
                        () = context.cancel.cancelled() => Ok(String::new()),
                        () = speech.cancelled() => Ok(String::new()),
                        result = tokio::time::timeout(Duration::from_secs(50), stt.transcribe_live(receiver, 16_000)) =>
                            result.map_err(|_| "The live voice input timed out.".to_string()).and_then(|r| r.map_err(|e| e.to_string())),
                    };
                    let bytes = committed.load(Ordering::SeqCst);
                    match transcribed {
                        Ok(text) if bytes != 0 && !speech.is_cancelled() => {
                            self.respond(
                                &id,
                                &context.for_utterance(),
                                *seq,
                                text,
                                TranscriptSource::Audio {
                                    duration: bytes / 32,
                                    bytes: bytes as usize,
                                },
                                speech,
                            )
                            .await
                        }
                        Ok(_) => Ok(()),
                        Err(error) => Err(error),
                    }
                }
                Work::Utterance {
                    seq,
                    clip,
                    duration,
                    speech,
                } => {
                    self.hear(
                        &id,
                        &context.for_utterance(),
                        *seq,
                        clip.clone(),
                        *duration,
                        speech,
                    )
                    .await
                }
                Work::Delivery(delivery, speech) | Work::Notice(delivery, speech) => {
                    let held = self
                        .change(&id, |c| Ok(c.state == VoiceState::Held))
                        .unwrap_or(true);
                    if held || speech.is_cancelled() {
                        self.push(delivery, &delivery.text);
                        Ok(())
                    } else if matches!(next, Work::Notice(..)) && self.fronted(&id) {
                        self.front_notice(&id, &delivery.text).await
                    } else if matches!(next, Work::Notice(..)) {
                        self.say(&id, &delivery.text, speech, &context.cancel).await
                    } else {
                        self.narrate(&id, &context, delivery, speech).await
                    }
                }
                Work::Answer {
                    event,
                    chunks,
                    speech,
                } => {
                    let chunks = lock(chunks).take().expect("one answer consumer");
                    self.answer(&id, event, chunks, speech, &context.cancel)
                        .await
                }
                Work::TurnOver(seq) => {
                    let _ = self.change(&id, |call| {
                        if call.answering.is_some_and(|answering| answering <= *seq) {
                            call.answering = None;
                        }
                        Ok(())
                    });
                    Ok(())
                }
            };
            // A refused reservation is what ends a call on its budget.
            let budget_failed = result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR);
            if !context.cancel.is_cancelled() {
                if budget_failed {
                    self.system_line(&id, BUDGET_LINE, include_bytes!("assets/budget.wav"), None);
                    let _ = self.finish(&id, VoiceEndReason::Budget);
                } else if result.is_err() {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        self.system_line(&id, ERROR_LINE, include_bytes!("assets/error.wav"), None);
                        let _ = self.finish(&id, VoiceEndReason::Error);
                    } else {
                        let speech = match &next {
                            Work::Utterance { speech, .. }
                            | Work::Text { speech, .. }
                            | Work::Live { speech, .. }
                            | Work::Answer { speech, .. }
                            | Work::Delivery(_, speech)
                            | Work::Notice(_, speech) => Some(speech),
                            Work::TurnOver(_) => None,
                        };
                        self.system_line(
                            &id,
                            RETRY_LINE,
                            include_bytes!("assets/retry.wav"),
                            speech,
                        );
                    }
                } else {
                    failures = 0;
                }
            }
            if let Work::Delivery(delivery, _) | Work::Notice(delivery, _) = &next
                && (result.is_err() || context.cancel.is_cancelled())
            {
                self.push(delivery, &delivery.text);
            }
            let _ = self.change(&id, |call| {
                if matches!(
                    next,
                    Work::Utterance { .. } | Work::Live { .. } | Work::Text { .. }
                ) {
                    call.input = None;
                    call.utterance_pending = false;
                    if call.target.is_none() {
                        call.first_clip_started = None;
                    }
                }
                if call.state != VoiceState::Held {
                    call.state(call.resting(), None);
                }
                Ok(())
            });
        }
        while let Ok(work) = work.try_recv() {
            if let Work::Delivery(d, _) | Work::Notice(d, _) = work {
                self.push(&d, &d.text);
            }
        }
    }

    pub(super) async fn hear(
        &self,
        id: &str,
        context: &Context,
        seq: u32,
        clip: Clip,
        duration: u32,
        speech_cancel: &CancellationToken,
    ) -> Result<(), String> {
        let services = self.speech_for(id)?;
        let stt = services
            .stt
            .as_ref()
            .ok_or("This call does not have a remote transcription provider.")?;
        let bytes = clip.bytes.len();
        let billable_ms = speech::billable_ms(&clip.mime, bytes, duration);
        self.pay(
            Kind::Stt,
            ledger::stt_usd(&stt.id().provider_id, f64::from(billable_ms) / 1000.0),
        )?;
        let text = tokio::select! {
            () = context.cancel.cancelled() => return Ok(()),
            () = speech_cancel.cancelled() => return Ok(()),
            text = stt.transcribe(clip) => text.map_err(|e| e.to_string())?,
        };
        if context.cancel.is_cancelled() {
            return Ok(());
        }
        self.respond(
            id,
            context,
            seq,
            text,
            TranscriptSource::Audio { duration, bytes },
            speech_cancel,
        )
        .await
    }

    pub(super) async fn respond(
        &self,
        id: &str,
        context: &Context,
        seq: u32,
        text: String,
        source: TranscriptSource,
        speech_cancel: &CancellationToken,
    ) -> Result<(), String> {
        if context.cancel.is_cancelled() || speech_cancel.is_cancelled() {
            return Ok(());
        }
        let text = text.trim().chars().take(8_000).collect::<String>();
        // What a front relays, it heard: the call was shown and kept that.
        let relayed = matches!(source, TranscriptSource::Front);
        if !relayed {
            self.emit(
                id,
                VoiceEvent::Heard {
                    seq,
                    text: text.clone(),
                },
            );
        }
        if text.is_empty() {
            return Ok(());
        }
        let target = self.change(id, |call| {
            if !relayed {
                call.conversation.begin_turn();
            }
            Ok(call.target.clone())
        })?;
        if target.is_none() {
            if !relayed {
                self.keep(id, Speaker::Person, &text);
            }
        } else {
            let _ = self.change(id, |call| {
                call.words.clone_from(&text);
                Ok(())
            });
        }
        if goodbye(&text) && source.permits_goodbye() {
            if self
                .say(id, "Goodbye.", speech_cancel, &context.cancel)
                .await
                .is_err()
            {
                self.system_line(
                    id,
                    "Goodbye.",
                    include_bytes!("assets/goodbye.wav"),
                    Some(speech_cancel),
                );
            }
            let _ = self.finish(id, VoiceEndReason::Goodbye);
            return Ok(());
        }
        self.ready_for(id)?;
        if let Some(target) = target {
            let thread = self
                .change(id, |call| Ok(call.thread.clone()))?
                .ok_or("The call has no thread.")?;
            let room = self.live_room().ok_or("The desk has closed.")?;
            room.call_prompt(&target, &thread, &text).await?;
            return self.change(id, |call| {
                call.answering = Some(seq);
                Ok(())
            });
        }
        let dispatcher = self.dispatcher()?;
        let context = context.clone();
        let (output, mut answers) = mpsc::channel(8);
        let produce = async {
            tokio::time::timeout(
                Duration::from_secs(60),
                dispatcher.answer_stream(context.clone(), &text, self.ledger.clone(), output),
            )
            .await
            .map_err(|_| "The voice dispatcher timed out.".to_string())?
        };
        let reply = Mutex::new(Reply::default());
        let consume = self.say_reply(id, &mut answers, &reply, speech_cancel, &context.cancel);
        tokio::select! {
            () = context.cancel.cancelled() => Ok(()),
            result = async {
                let (produced, spoken) = tokio::join!(produce, consume);
                let kept = self.keep_reply(id, std::mem::take(&mut *lock(&reply)));
                if [&produced, &spoken].iter().any(|result| result.as_ref().err().is_some_and(|e| e == BUDGET_ERROR)) {
                    Err(BUDGET_ERROR.to_string())
                } else {
                    produced.and(spoken).and(kept)
                }
            } => result,
        }
    }

    pub(super) async fn summary(
        &self,
        id: Option<&str>,
        delivery: &Delivery,
    ) -> Result<String, String> {
        if speech_ready(&delivery.text) && !spoken::carries_more(&delivery.text) {
            return Ok(delivery.text.trim().to_string());
        }
        let fallback = || {
            sentences(&delivery.text)
                .into_iter()
                .next()
                .unwrap_or_default()
        };
        if delivery.text.len() > 32_000 {
            return Ok(fallback());
        }
        let Ok(dispatcher) = self.dispatcher() else {
            return Ok(fallback());
        };
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            dispatcher.narrate(
                &delivery.name,
                &delivery.text,
                &self.narration_instruction(id, &delivery.text),
                self.ledger.clone(),
            ),
        )
        .await;
        match result {
            Ok(Err(error)) if error == BUDGET_ERROR => Err(error),
            Ok(Ok(text)) if !text.trim().is_empty() => Ok(sentences(&text).join(" ")),
            _ => Ok(fallback()),
        }
    }

    pub async fn narrate(
        &self,
        id: &str,
        context: &Context,
        delivery: &Delivery,
        speech: &CancellationToken,
    ) -> Result<(), String> {
        self.ready_for(id)?;
        let text = tokio::select! {
            () = context.cancel.cancelled() => return Ok(()),
            () = speech.cancelled() => { self.push(delivery, &delivery.text); return Ok(()); },
            text = self.summary(Some(id), delivery) => text?,
        };
        if text.is_empty() {
            return Ok(());
        }
        // Hold/interrupt may win while the narrator is completing. Decide publication
        // under the same lock as those commands, just as for audio clips.
        let published = self.change(id, |call| {
            if call.state == VoiceState::Held || speech.is_cancelled() {
                return Ok(false);
            }
            let _ = call.events.send(VoiceEvent::Delivery {
                persona_id: delivery.persona.clone(),
                event_id: delivery.event.clone(),
                text: text.clone(),
            });
            Ok(true)
        })?;
        if !published {
            self.push(delivery, &delivery.text);
            return Ok(());
        }
        self.say(id, &text, speech, &context.cancel).await
    }

    /// A teammate's reply on its own call, said as it streams in: its spoken
    /// version ([`spoken::Spoken`]) sentence by sentence as one line, kept on
    /// the call's thread once it is over. A reply that wrote none is said
    /// as its own prose on Legacy; live work may use [`Self::unspoken`].
    /// Speaking over it stops it at once, so the call is free for what the person says; the chat has the
    /// written version in any case. A reply that reaches its end is counted
    /// by how it was said ([`Self::count`]).
    pub(super) async fn answer(
        &self,
        id: &str,
        event: &str,
        mut chunks: mpsc::UnboundedReceiver<String>,
        speech: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        if speech.is_cancelled() {
            return Ok(());
        }
        let legacy = !self.fronted(id);
        let (sentences, mut said) = mpsc::channel(8);
        let pointed = std::sync::atomic::AtomicBool::new(false);
        let pointer_added = &pointed;
        let split = async move {
            let mut spoken = if legacy {
                spoken::Spoken::streaming()
            } else {
                spoken::Spoken::default()
            };
            let mut whole = String::new();
            while let Some(chunk) = chunks.recv().await {
                whole.push_str(&chunk);
                for sentence in spoken.push(&chunk) {
                    if sentences.send(sentence).await.is_err() {
                        return;
                    }
                }
            }
            let rest = match spoken.finish() {
                spoken::Ending::Spoken(rest) => {
                    self.count(id, event, spoken::path(&whole));
                    rest
                }
                spoken::Ending::Untagged(opening) if legacy => {
                    if !spoken::said_as_written(&spoken::versions(&whole).written) {
                        self.count(id, event, spoken::Path::Untagged);
                    }
                    opening
                }
                spoken::Ending::Untagged(opening) => {
                    self.unspoken(id, event, &whole, opening, speech).await
                }
            };
            for sentence in rest {
                if sentences.send(sentence).await.is_err() {
                    return;
                }
            }
            let versions = spoken::versions(&whole);
            let written = versions.written;
            let written = self
                .change(id, |call| Ok(call.conversation.result_text(&written)))
                .unwrap_or(written);
            if !speech.is_cancelled() && versions.spoken.as_deref() != Some(written.as_str()) {
                let instruction = self.change(id, |call| {
                    Ok(call.conversation.pointer(&written).then(|| {
                        format!("{} Only supply the brief contextual pointer now; the answer has already been spoken. Do not repeat it.", call.conversation.instruction(true))
                    }))
                }).ok().flatten();
                if let Some(instruction) = instruction
                    && let Ok(assistant) = self.dispatcher()
                    && let Ok(Ok(pointer)) = tokio::time::timeout(
                        REWRITE_WAIT,
                        assistant.rewrite("", &written, &instruction, self.ledger.clone()),
                    )
                    .await
                {
                    pointer_added.store(!pointer.trim().is_empty(), Ordering::Relaxed);
                    for sentence in spoken::sentences(&pointer) {
                        if sentences.send(sentence).await.is_err() {
                            return;
                        }
                    }
                }
            }
        };
        let reply = Mutex::new(Reply::default());
        let say = self.say_reply(id, &mut said, &reply, speech, ended);
        let result = tokio::select! {
            () = speech.cancelled() => Ok(()),
            () = ended.cancelled() => Ok(()),
            ((), said) = async { tokio::join!(split, say) } => said,
        };
        let _ = self.change(id, |call| {
            call.streams.remove(event);
            Ok(())
        });
        let reply = std::mem::take(&mut *lock(&reply));
        if pointed.load(Ordering::Relaxed)
            && let Ok((Some(persona), thread, false)) = self.change(id, |call| {
                Ok((
                    call.target.clone(),
                    call.thread.clone(),
                    call.front.is_some(),
                ))
            })
        {
            self.keep_spoken(&persona, thread, event, &reply.text);
        }
        let kept = self.keep_reply(id, reply);
        result.and(kept)
    }

    /// What is said of a teammate's whole reply that wrote no spoken version.
    /// One short line, such as the line the contract asks for before a tool,
    /// is said as written and not counted. Anything longer was written to be
    /// read, so the call assistant says it again to be heard
    /// ([`Self::rewrite`]); without one, or when that fails, the call says
    /// the reply's opening.
    pub(super) async fn unspoken(
        &self,
        id: &str,
        event: &str,
        whole: &str,
        opening: Vec<String>,
        speech: &CancellationToken,
    ) -> Vec<String> {
        let written = spoken::versions(whole).written;
        if written.trim().is_empty() || spoken::said_as_written(&written) {
            return opening;
        }
        if let Some(rewritten) = self.rewrite(id, event, &written, speech).await {
            self.count(id, event, spoken::Path::Rewritten);
            sentences(&rewritten)
        } else {
            self.count(id, event, spoken::Path::Untagged);
            opening
        }
    }

    /// A reply written only to be read, said again by the call assistant as
    /// it would be on a phone call ([`Dispatcher::rewrite`]), from the reply
    /// and the person's last words, and kept as what was said for it. The
    /// call remains responsive while it is written. `None`, and a line in
    /// the log, when the desk has no call assistant, the Chat budget refuses it, it fails,
    /// or it takes longer than [`REWRITE_WAIT`]: none of those ends the call.
    pub(super) async fn rewrite(
        &self,
        id: &str,
        event: &str,
        written: &str,
        speech: &CancellationToken,
    ) -> Option<String> {
        let failed = |why: &str| {
            eprintln!(
                "[voice] reply {event} on call {id} was not rewritten, so its opening is said: {why}"
            );
        };
        let assistant = match self.dispatcher() {
            Ok(assistant) => assistant,
            Err(error) => {
                failed(&error);
                return None;
            }
        };
        let (persona, words) = self
            .change(id, |call| {
                if call.state != VoiceState::Held && !speech.is_cancelled() {
                    call.state(VoiceState::Thinking, None);
                }
                Ok((call.target.clone(), call.words.clone()))
            })
            .ok()?;
        let rewritten = tokio::time::timeout(
            REWRITE_WAIT,
            assistant.rewrite(
                &words,
                written,
                &self.narration_instruction(Some(id), written),
                self.ledger.clone(),
            ),
        )
        .await;
        match rewritten {
            Ok(Ok(text)) if !spoken::speech_text(&text).is_empty() => {
                let text = text.trim().to_string();
                // A live result is work; the provider records the words it actually says.
                if let Some(persona) = persona
                    && !self.fronted(id)
                {
                    let thread = self
                        .change(id, |call| Ok(call.thread.clone()))
                        .ok()
                        .flatten();
                    self.keep_spoken(&persona, thread, event, &text);
                }
                Some(text)
            }
            Ok(Ok(_)) => {
                failed("it came back empty");
                None
            }
            Ok(Err(error)) => {
                failed(&error);
                None
            }
            Err(_) => {
                failed(&format!("it took longer than {REWRITE_WAIT:?}"));
                None
            }
        }
    }

    /// Keeps a rewrite as what was said for its reply, on the reply's first
    /// bubble, where a reply's own spoken version is kept
    /// (`Room::voice_spoken`). The session writes the reply just after it
    /// hands the reply to the call, so a rewrite that comes back first waits
    /// a moment for it.
    pub(super) fn keep_spoken(
        &self,
        persona: &str,
        thread: Option<hotline_contract::ThreadId>,
        event: &str,
        spoken: &str,
    ) {
        let room = self.room.clone();
        let (persona, event, spoken) = (persona.to_string(), event.to_string(), spoken.to_string());
        tokio::spawn(async move {
            for _ in 0..20 {
                let Some(room) = room.upgrade() else {
                    return;
                };
                if thread.as_ref().map_or_else(
                    || room.voice_spoken(&persona, &event, &spoken),
                    |thread| room.call_spoken(&persona, thread, &event, &spoken),
                ) {
                    return;
                }
                drop(room);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            eprintln!(
                "[voice] reply {event} was rewritten and said, but was never written, so the rewrite is not kept"
            );
        });
    }

    /// Counts a teammate's reply on a call to it under the model that wrote
    /// it, and says so in one line of the log.
    pub(super) fn count(&self, id: &str, event: &str, path: spoken::Path) {
        let Ok(Some(persona)) = self.change(id, |call| Ok(call.target.clone())) else {
            return;
        };
        let model = self.writer(&persona);
        eprintln!(
            "[voice] reply {event} on call {id} by {model}: {}",
            path.name()
        );
        self.replies.count(&model, path);
    }

    /// The agent and model a teammate's replies come from, as the counts
    /// name them: `hotline/` and the model for Hotline Agent, `acp/` and the
    /// adapter for an ACP agent, then the model its session reports, if any.
    pub(super) fn writer(&self, persona_id: &str) -> String {
        let persona = hotline_room::room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == persona_id);
        let reported = self
            .live_room()
            .and_then(|room| room.info(persona_id))
            .and_then(|info| info.current_model_id);
        let model = reported
            .or_else(|| {
                persona
                    .as_ref()
                    .and_then(|persona| persona.model_id.clone())
            })
            .filter(|model| !model.is_empty());
        let agent = match persona.map(|persona| persona.backend_id) {
            Some(backend) if backend == "hotline" => backend,
            Some(backend) => format!("acp/{backend}"),
            None => "unknown".to_string(),
        };
        match model {
            Some(model) => format!("{agent}/{model}"),
            None => agent,
        }
    }

    /// Whether a call with this front may begin: a front that costs nothing
    /// is never refused, a paid one needs the Voice budget to have room.
    pub fn front_ready(&self, front: &dyn realtime::Realtime) -> Result<(), String> {
        let provider = front.id().provider_id;
        let kinds: &[Kind] = if front.is_subscription() || ledger::front_usd(&provider, 1.0) == 0.0
        {
            &[]
        } else {
            &[Kind::Front]
        };
        self.ledger
            .ready(kinds)
            .map_err(|_| BUDGET_ERROR.to_string())
    }

    /// Counts `seconds` of a call's front against the Voice budget. Nothing
    /// for a subscription login. `Err` says the budget is spent and the call
    /// should end.
    pub fn front_spend(&self, front: &dyn realtime::Realtime, seconds: f64) -> Result<(), String> {
        if front.is_subscription() {
            return Ok(());
        }
        let usd = ledger::front_usd(&front.id().provider_id, seconds);
        self.pay(Kind::Front, usd)
    }

    /// What the person said to the front, as the front heard it. Kept on the
    /// call's thread and nowhere else: the teammate is handed the words by
    /// [`front::Front`], not by this.
    pub fn front_heard(&self, id: &str, text: &str) -> String {
        self.keep(id, Speaker::Person, text)
    }

    /// What the front said to the person, kept on the call's thread and never
    /// on the teammate's tape.
    pub fn front_said(&self, id: &str, text: &str) -> String {
        self.keep(id, Speaker::Voice, text)
    }

    /// A line said on a direct call, kept on the call's thread. The line's id
    /// is what the clients know it by; the write is made behind the call.
    pub(super) fn keep(&self, id: &str, speaker: Speaker, text: &str) -> String {
        let line = Uuid::new_v4().to_string();
        self.keep_as(id, speaker, &line, text);
        line
    }

    pub(super) fn keep_as(&self, id: &str, speaker: Speaker, line: &str, text: &str) {
        let _ = self.change(id, |call| {
            if let Some(record) = &call.record {
                record.said(speaker, line, text);
            }
            Ok(())
        });
    }

    /// A reply said sentence by sentence, kept once and whole: on a direct
    /// call as one line of the call's thread under the id the clients were
    /// shown, including a fallback desk call's own transcript.
    pub(super) fn keep_reply(&self, id: &str, reply: Reply) -> Result<(), String> {
        if reply.text.is_empty() {
            return Ok(());
        }
        let direct = self.change(id, |call| {
            call.conversation.remember(&reply.text);
            Ok(call.target.is_some())
        })?;
        if !direct {
            self.keep_as(id, Speaker::Voice, &reply.line, &reply.text);
        }
        Ok(())
    }

    /// Speech is priced from what is sent (seconds of audio, characters of
    /// text) and no provider reports usage back, so the reservation is the
    /// charge and nothing is settled afterwards.
    pub(super) fn pay(&self, kind: Kind, usd: f64) -> Result<(), String> {
        self.ledger
            .reserve(kind, usd)
            .map(drop)
            .map_err(|_| BUDGET_ERROR.to_string())
    }
}
