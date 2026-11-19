//! What a call says: sentences synthesised into clips, in order, and stopped
//! when the person interrupts.

use super::ledger::{self, Kind};
use super::metering::BUDGET_ERROR;
use super::record::Speaker;
use super::settings::{self, VoiceSettings};
use super::speech::{self, Clip, SpeechError};
use super::{Call, CallSpeech, Calls, MAX_AUDIO, Origin, Reply, Voiced, Work, lock, spoken};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use futures_util::stream::FuturesOrdered;
use hotline_contract::{VoiceEvent, VoiceState};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Maximum concurrent whole-clip TTS requests pipelined ahead in `say_reply`.
/// Keeping 1 sentence in flight behind the current one eliminates inter-sentence
/// silence gaps on whole-clip providers (e.g. Ollama Phonon, OpenAI) without
/// over-synthesizing ahead of a barge-in.
const MAX_TTS_IN_FLIGHT: usize = 2;

/// A line being said on a call: the call, the id its clips are sent under,
/// and the two things that stop it, being spoken over and the call ending.
struct Line<'a> {
    call: &'a str,
    id: &'a str,
    interrupted: &'a CancellationToken,
    ended: &'a CancellationToken,
}

impl Calls {
    /// One sentence's clip, from the call's voice or else its fallback. `Err`
    /// is [`BUDGET_ERROR`], or the voice's refusal as a sentence for the
    /// person ([`SpeechError::unspoken`]): the voice they chose is the one
    /// named, even when the fallback failed after it.
    pub(super) async fn synthesize(&self, speech: &CallSpeech, text: &str) -> Result<Clip, String> {
        let provider = speech.tts.id().provider_id;
        self.pay(Kind::Tts, ledger::tts_usd(&provider, text.chars().count()))?;
        let unusable = || {
            SpeechError::Malformed {
                provider_id: provider.clone(),
            }
            .unspoken(&provider)
        };
        let unspoken = match speech.tts.speak(text).await {
            Ok(clip) if playable(&clip, clip.bytes.len()) => return Ok(clip),
            Ok(_) => return Err(unusable()),
            Err(error) => error.unspoken(&provider),
        };
        let Some(fallback) = &speech.fallback_tts else {
            return Err(unspoken);
        };
        if speech.tts.is_subscription() || !self.connected(fallback.as_ref()) {
            return Err(unspoken);
        }
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
        )?;
        match fallback.speak(text).await {
            Ok(clip) if playable(&clip, clip.bytes.len()) => Ok(clip),
            Ok(_) | Err(_) => Err(unspoken),
        }
    }

    /// Notes what became of a sentence sent to the call's voice, for the turn
    /// loop to read when the step is over ([`Voiced`]). A refusal stands for
    /// the rest of the step; the budget is no refusal, since it ends the call.
    fn voiced<T>(&self, id: &str, result: &Result<T, String>) {
        let _ = self.change(id, |call| {
            match result {
                Ok(_) if matches!(call.voiced, Voiced::Silent) => call.voiced = Voiced::Spoke,
                Err(error) if error != BUDGET_ERROR => {
                    call.voiced = Voiced::Refused(error.clone());
                }
                _ => {}
            }
            Ok(())
        });
    }

    /// Speaks a line at once, kept as the call's own on a direct call's
    /// thread, including a no-lead desk call.
    pub(super) async fn say(
        &self,
        id: &str,
        text: &str,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        let sentences: Vec<String> = spoken::sentences(text)
            .into_iter()
            .filter(|sentence| !spoken::speech_text(sentence).is_empty())
            .collect();
        if sentences.is_empty() {
            return Err("The dispatcher returned no words.".into());
        }
        let text = sentences.join(" ");
        let _ = self.change(id, |call| {
            call.conversation.remember(&text);
            Ok(())
        });
        let line = if self.fronted(id) {
            uuid::Uuid::new_v4().to_string()
        } else {
            self.keep(id, Speaker::Voice, &text)
        };
        self.emit(
            id,
            VoiceEvent::Said {
                id: line.clone(),
                text,
            },
        );
        if self.fronted(id) {
            return Ok(());
        }
        self.speaking(id, interrupted);
        let speech = self.speech_for(id)?;
        let streaming = self.change(id, |call| Ok(call.stream_audio))?;
        let said = Line {
            call: id,
            id: &line,
            interrupted,
            ended,
        };
        let mut index = 0;
        for (at, sentence) in sentences.iter().enumerate() {
            if interrupted.is_cancelled() || ended.is_cancelled() {
                return Ok(());
            }
            let last = at + 1 == sentences.len();
            self.speak_sentence(&said, &speech, streaming, sentence, &mut index, last)
                .await?;
        }
        Ok(())
    }

    /// Says a reply as the dispatcher writes it, sentence by sentence, as one
    /// line: each sentence is spoken as soon as it arrives, the clients are
    /// shown the line growing under one id, and its audio carries on under
    /// that id until an empty final clip closes it. The words are gathered in
    /// `reply` for [`Self::keep_reply`], which keeps them once, whole, even
    /// when the person speaks over the reply. Speech that fails stops the
    /// speaking, not the words.
    pub(super) async fn say_reply(
        &self,
        id: &str,
        answers: &mut mpsc::Receiver<String>,
        reply: &Mutex<Reply>,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Result<(), String> {
        type SynthFuture<'a> =
            Pin<Box<dyn Future<Output = (String, Result<Clip, String>)> + Send + 'a>>;
        let mut failure: Option<String> = None;
        let mut speech: Option<(CallSpeech, bool)> = None;
        let mut index = 0u32;
        let mut in_flight: FuturesOrdered<SynthFuture<'_>> = FuturesOrdered::new();
        let mut pending_said: VecDeque<Option<(String, String)>> = VecDeque::new();
        let mut answers_closed = false;

        let flush_pending_said = |pending: &mut VecDeque<Option<(String, String)>>| {
            while let Some(entry) = pending.pop_front() {
                if let Some((said_id, said_text)) = entry {
                    self.emit(
                        id,
                        VoiceEvent::Said {
                            id: said_id,
                            text: said_text,
                        },
                    );
                }
            }
        };

        loop {
            if interrupted.is_cancelled() || ended.is_cancelled() || failure.is_some() {
                flush_pending_said(&mut pending_said);
                in_flight.clear();
            }
            if answers_closed && in_flight.is_empty() {
                break;
            }
            let can_poll_tts = !in_flight.is_empty();
            let can_recv = !answers_closed
                && (in_flight.len() < MAX_TTS_IN_FLIGHT
                    || failure.is_some()
                    || interrupted.is_cancelled()
                    || ended.is_cancelled());

            tokio::select! {
                biased;
                () = interrupted.cancelled(), if can_poll_tts => {
                    flush_pending_said(&mut pending_said);
                    in_flight.clear();
                }
                () = ended.cancelled(), if can_poll_tts => {
                    flush_pending_said(&mut pending_said);
                    in_flight.clear();
                }
                Some((line, result)) = in_flight.next(), if can_poll_tts => {
                    self.voiced(id, &result);
                    if let Some(Some((said_id, said_text))) = pending_said.pop_front() {
                        self.emit(
                            id,
                            VoiceEvent::Said {
                                id: said_id,
                                text: said_text,
                            },
                        );
                    }
                    match result {
                        Ok(clip) => {
                            if !interrupted.is_cancelled() && !ended.is_cancelled() {
                                self.clip(id, &line, index, false, &clip, Some(interrupted));
                                index += 1;
                            }
                        }
                        Err(error) => {
                            failure = Some(error);
                            flush_pending_said(&mut pending_said);
                            in_flight.clear();
                        }
                    }
                }
                received = answers.recv(), if can_recv => {
                    let Some(sentence) = received else {
                        answers_closed = true;
                        continue;
                    };
                    let sentence = sentence.trim();
                    if spoken::speech_text(sentence).is_empty() {
                        continue;
                    }
                    let (line, text) = lock(reply).add(sentence);
                    if failure.is_some()
                        || interrupted.is_cancelled()
                        || ended.is_cancelled()
                        || self.fronted(id)
                    {
                        self.emit(
                            id,
                            VoiceEvent::Said {
                                id: line,
                                text,
                            },
                        );
                        continue;
                    }
                    if speech.is_none() {
                        self.speaking(id, interrupted);
                        let services = self.speech_for(id).and_then(|services| {
                            Ok((services, self.change(id, |call| Ok(call.stream_audio))?))
                        });
                        match services {
                            Ok(services) => speech = Some(services),
                            Err(error) => {
                                self.emit(
                                    id,
                                    VoiceEvent::Said {
                                        id: line,
                                        text,
                                    },
                                );
                                failure = Some(error);
                                continue;
                            }
                        }
                    }
                    let (services, streaming) = speech.as_ref().expect("resolved above");
                    let _ = self.change(id, |call| {
                        if let Some(timing) = &mut call.turn_timing {
                            timing.mark_tts_started();
                        }
                        Ok(())
                    });
                    if *streaming && services.tts.streams_chunks() {
                        self.emit(
                            id,
                            VoiceEvent::Said {
                                id: line.clone(),
                                text,
                            },
                        );
                        let said = Line {
                            call: id,
                            id: &line,
                            interrupted,
                            ended,
                        };
                        if let Err(error) = self
                            .speak_sentence(&said, services, true, sentence, &mut index, false)
                            .await
                        {
                            failure = Some(error);
                        }
                    } else {
                        if in_flight.is_empty() {
                            self.emit(
                                id,
                                VoiceEvent::Said {
                                    id: line.clone(),
                                    text,
                                },
                            );
                            pending_said.push_back(None);
                        } else {
                            // Keep live transcript aligned with clip order while synthesis overlaps.
                            pending_said.push_back(Some((line.clone(), text)));
                        }
                        let services = services.clone();
                        let clean = spoken::speech_text(sentence);
                        in_flight.push_back(Box::pin(async move {
                            let result = self.synthesize(&services, &clean).await;
                            (line, result)
                        }));
                    }
                }
            }
        }
        if failure.is_none() && index > 0 && !interrupted.is_cancelled() && !ended.is_cancelled() {
            let line = lock(reply).line.clone();
            self.clip(
                id,
                &line,
                index,
                true,
                &Clip {
                    mime: "audio/wav".into(),
                    bytes: Vec::new(),
                },
                Some(interrupted),
            );
        }
        failure.map_or(Ok(()), Err)
    }

    /// The call is speaking, unless it is held or was just spoken over.
    pub(super) fn speaking(&self, id: &str, interrupted: &CancellationToken) {
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held && !interrupted.is_cancelled() {
                call.state(VoiceState::Speaking, None);
            }
            Ok(())
        });
    }

    /// One sentence of `line`, sent as its next clip or clips from `index`.
    async fn speak_sentence(
        &self,
        line: &Line<'_>,
        speech: &CallSpeech,
        streaming: bool,
        sentence: &str,
        index: &mut u32,
        last: bool,
    ) -> Result<(), String> {
        let sentence = &spoken::speech_text(sentence);
        if streaming {
            return tokio::select! {
                () = line.interrupted.cancelled() => Ok(()),
                () = line.ended.cancelled() => Ok(()),
                result = self.synthesize_stream(line, speech, sentence, index, last) => {
                    self.voiced(line.call, &result);
                    result
                }
            };
        }
        let clip = tokio::select! {
            () = line.interrupted.cancelled() => return Ok(()),
            () = line.ended.cancelled() => return Ok(()),
            clip = self.synthesize(speech, sentence) => {
                self.voiced(line.call, &clip);
                clip?
            }
        };
        // A hold/interrupt can arrive at the same time as the provider.
        if line.interrupted.is_cancelled() || line.ended.is_cancelled() {
            return Ok(());
        }
        self.clip(
            line.call,
            line.id,
            *index,
            last,
            &clip,
            Some(line.interrupted),
        );
        *index += 1;
        Ok(())
    }

    async fn synthesize_stream(
        &self,
        line: &Line<'_>,
        services: &CallSpeech,
        text: &str,
        index: &mut u32,
        final_sentence: bool,
    ) -> Result<(), String> {
        let Line {
            call: id,
            id: line,
            interrupted,
            ended: _,
        } = *line;
        if interrupted.is_cancelled() {
            return Ok(());
        }
        let provider = services.tts.id().provider_id;
        self.pay(Kind::Tts, ledger::tts_usd(&provider, text.chars().count()))?;
        let unusable = || SpeechError::Malformed {
            provider_id: provider.clone(),
        };
        let (output, chunks) = mpsc::channel(4);
        let produce = services.tts.speak_chunks(text, output);
        let mut published = false;
        let consume = async {
            let mut chunks = chunks;
            let mut bytes = 0;
            let mut finished = false;
            while let Some(chunk) = chunks.recv().await {
                let clip = chunk.clip;
                bytes += clip.bytes.len();
                // Audio after the final chunk is as unusable as the wrong kind.
                if finished || !playable(&clip, bytes) {
                    return Err(unusable());
                }
                self.clip(
                    id,
                    line,
                    *index,
                    final_sentence && chunk.final_chunk,
                    &clip,
                    Some(interrupted),
                );
                *index += 1;
                published = true;
                finished = chunk.final_chunk;
            }
            if finished { Ok(()) } else { Err(unusable()) }
        };
        let attempted = match tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(produce, consume)
        })
        .await
        {
            // A provider that refused also leaves the sound unfinished: the
            // refusal is the reason, unless it only stopped because the
            // unusable sound was turned away.
            Ok((Err(refused), _)) if refused != SpeechError::Cancelled => Err(refused),
            Ok((produced, consumed)) => consumed.and(produced),
            Err(_) => Err(SpeechError::Unreachable {
                provider_id: provider.clone(),
            }),
        };
        let Err(error) = attempted else {
            return Ok(());
        };
        if interrupted.is_cancelled() {
            return Ok(());
        }
        let unspoken = error.unspoken(&provider);
        // A replacement after audible output would repeat words. Only an attempt
        // that produced no usable chunks may use the configured paid fallback.
        let Some(fallback) = services
            .fallback_tts
            .as_ref()
            .filter(|_| !published && !services.tts.is_subscription())
        else {
            return Err(unspoken);
        };
        if !self.connected(fallback.as_ref()) {
            return Err(unspoken);
        }
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
        )?;
        if interrupted.is_cancelled() {
            return Ok(());
        }
        match tokio::time::timeout(Duration::from_secs(30), fallback.speak(text)).await {
            Ok(Ok(clip)) if playable(&clip, clip.bytes.len()) => {
                self.clip(id, line, *index, final_sentence, &clip, Some(interrupted));
                *index += 1;
                Ok(())
            }
            _ => Err(unspoken),
        }
    }

    pub(super) fn clip(
        &self,
        id: &str,
        line: &str,
        index: u32,
        last: bool,
        clip: &Clip,
        interrupted: Option<&CancellationToken>,
    ) {
        let telemetry = self.change(id, |call| {
            if call.state != VoiceState::Held
                && !interrupted.is_some_and(CancellationToken::is_cancelled)
            {
                if !clip.bytes.is_empty() {
                    let now = tokio::time::Instant::now();
                    call.waiting_sound = true;
                    call.last_spoken_at = Some(now);
                    call.feedback_at = Some(now);
                }
                let sent = call.events.send(VoiceEvent::Clip {
                    id: line.into(),
                    index,
                    r#final: last,
                    mime_type: clip.mime.clone(),
                    data: STANDARD.encode(&clip.bytes),
                });
                // The desk's acknowledgement is not the teammate's first
                // word, which the turn's timing line waits for.
                if sent.is_ok()
                    && !clip.bytes.is_empty()
                    && call.ack_line.as_deref() != Some(line)
                    && let Some(started) = call.first_clip_started.take()
                {
                    let timing = call
                        .turn_timing
                        .take()
                        .unwrap_or_else(|| super::TurnTiming::start(started, None));
                    if let Some(spoken) = timing.llm_spoken() {
                        call.last_llm_spoken = Some(spoken);
                    }
                    return Ok(Some((
                        call.id.clone(),
                        call.seq.unwrap_or_default(),
                        timing,
                        call.target.clone(),
                        call.speech_services.tts.id(),
                    )));
                }
            }
            Ok(None)
        });
        if let Ok(Some((call_id, seq, timing, target, tts_id))) = telemetry {
            let model = target.as_deref().map(|persona| self.writer(persona));
            eprintln!(
                "{}",
                timing.format_line(
                    std::time::Instant::now(),
                    &call_id,
                    seq,
                    model.as_deref(),
                    &tts_id
                )
            );
        }
    }
}

/// Whether a clip is one the clients can play: a kind they take, some
/// sound, and with `bytes` of the line's sound so far, not too much.
pub(super) fn playable(clip: &Clip, bytes: usize) -> bool {
    matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
        && !clip.bytes.is_empty()
        && bytes <= MAX_AUDIO
}

/// The call to `persona` a line of its session belongs to: one still going
/// on which the line's turn, or a later one, was said.
pub(super) fn calling<'a>(
    calls: &'a mut VecDeque<Call>,
    persona: &str,
    origin: &Origin,
) -> Option<&'a mut Call> {
    calls.iter_mut().rev().find(|call| {
        origin.direct
            && call.id == origin.call_id
            && call.state != VoiceState::Ended
            && call.target.as_deref() == Some(persona)
            && call.seq.is_some_and(|seq| origin.seq <= seq)
    })
}

/// Queues a teammate's reply to be said on its call, and hands back where its
/// words go. None when the call has too much queued to take it: the reply is
/// then left to the phone, and never begun later from its middle.
pub(super) fn begin_answer(call: &mut Call, event: &str) -> Option<mpsc::UnboundedSender<String>> {
    let (words, chunks) = mpsc::unbounded_channel();
    let queued = call.work.try_send(Work::Answer {
        event: event.into(),
        chunks: Mutex::new(Some(chunks)),
        speech: call.deliveries.child_token(),
    });
    if queued.is_err() {
        call.cut_off(event.into());
        return None;
    }
    if call.spoken_events.len() >= 128 {
        call.spoken_events.pop_front();
    }
    call.spoken_events.push_back(event.into());
    Some(words)
}

/// Whether a teammate's reply on a desk call can be said as written.
pub fn speech_ready(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 320
        && spoken::sentences(text).len() <= 2
        && !text.contains(['`', '#', '*', '\n', '[', ']', '|'])
        && spoken::speech_text(text) == text
}

pub fn goodbye(text: &str) -> bool {
    static PUNCTUATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[\p{P}&&[^,-]]").expect("fixed punctuation regex")
    });
    static GOODBYE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?x)^
            (?:(?:ok(?:ay)?|alright|all\ right|thanks|thank\ you)[\ ,]+)*
            (?:goodbye|good\ bye|bye(?:[\ -]bye)?|see\ (?:you|ya)\ later
               |hang\ up|end\ (?:the\ )?call|thats\ all|that\ is\ all)
            (?:[\ ,]+(?:hotline|desk))?$",
        )
        .expect("fixed goodbye regex")
    });
    let lower = text.to_lowercase();
    let spoken = PUNCTUATION.replace_all(&lower, "");
    let spoken = spoken.split_whitespace().collect::<Vec<_>>().join(" ");
    GOODBYE.is_match(&spoken)
}

/// The desk's settings with a teammate's own voice, when the desk still
/// speaks with the provider and model that voice was picked from.
pub fn own_voice(
    settings: &VoiceSettings,
    voice: &hotline_contract::PersonaVoice,
    speaking: &speech::SpeechId,
) -> Option<VoiceSettings> {
    (speaking.provider_id == voice.provider_id
        && speaking.model_id == voice.model_id
        && speaking.voice.as_deref() != Some(voice.voice.as_str()))
    .then(|| VoiceSettings {
        tts: Some(settings::Choice {
            provider_id: voice.provider_id.clone(),
            model_id: Some(voice.model_id.clone()),
            voice: Some(voice.voice.clone()),
            effort: None,
        }),
        ..settings.clone()
    })
}
