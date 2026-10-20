//! What a call says: sentences synthesised into clips, in order, and stopped
//! when the person interrupts.

use super::ledger::{self, Kind};
use super::record::Speaker;
use super::settings::{self, VoiceSettings};
use super::speech::{self, Clip};
use super::{Call, CallSpeech, Calls, MAX_AUDIO, Origin, Reply, Work, lock, spoken};
use crate::contract::{VoiceEvent, VoiceState};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A line being said on a call: the call, the id its clips are sent under,
/// and the two things that stop it, being spoken over and the call ending.
struct Line<'a> {
    call: &'a str,
    id: &'a str,
    interrupted: &'a CancellationToken,
    ended: &'a CancellationToken,
}

impl Calls {
    pub(super) async fn synthesize(&self, speech: &CallSpeech, text: &str) -> Result<Clip, String> {
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&speech.tts.id().provider_id, text.chars().count()),
        )?;
        let clip = match speech.tts.speak(text).await {
            Ok(clip) => Ok(clip),
            Err(error) => match &speech.fallback_tts {
                Some(_) if speech.tts.is_subscription() => Err(error.to_string()),
                Some(fallback) => {
                    if !self.connected(fallback.as_ref()) {
                        return Err("The fallback voice provider is no longer connected.".into());
                    }
                    self.pay(
                        Kind::Tts,
                        ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
                    )?;
                    fallback.speak(text).await.map_err(|e| e.to_string())
                }
                None => Err(error.to_string()),
            },
        }?;
        if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
            || clip.bytes.is_empty()
            || clip.bytes.len() > MAX_AUDIO
        {
            return Err("The speech provider returned an unusable sentence clip.".into());
        }
        Ok(clip)
    }

    /// Speaks a line at once, kept as the call's own on a direct call's
    /// thread or the dispatcher's tape.
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
        let line = if self.change(id, |call| Ok(call.target.is_some()))? {
            self.keep(id, Speaker::Voice, &text)
        } else {
            self.record("agent", &text)?
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
        let mut failure = None;
        let mut speech = None;
        let mut index = 0;
        while let Some(sentence) = answers.recv().await {
            let sentence = sentence.trim();
            if spoken::speech_text(sentence).is_empty() {
                continue;
            }
            let (line, text) = lock(reply).add(sentence);
            self.emit(
                id,
                VoiceEvent::Said {
                    id: line.clone(),
                    text,
                },
            );
            if failure.is_some()
                || interrupted.is_cancelled()
                || ended.is_cancelled()
                || self.fronted(id)
            {
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
                        failure = Some(error);
                        continue;
                    }
                }
            }
            let (services, streaming) = speech.as_ref().expect("resolved above");
            let said = Line {
                call: id,
                id: &line,
                interrupted,
                ended,
            };
            if let Err(error) = self
                .speak_sentence(&said, services, *streaming, sentence, &mut index, false)
                .await
            {
                failure = Some(error);
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
                result = self.synthesize_stream(line, speech, sentence, index, last) => result,
            };
        }
        let clip = tokio::select! {
            () = line.interrupted.cancelled() => return Ok(()),
            () = line.ended.cancelled() => return Ok(()),
            clip = self.synthesize(speech, sentence) => clip?,
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
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&services.tts.id().provider_id, text.chars().count()),
        )?;
        let (output, chunks) = mpsc::channel(4);
        let produce = services.tts.speak_chunks(text, output);
        let mut published = false;
        let consume = async {
            let mut chunks = chunks;
            let mut bytes = 0;
            let mut finished = false;
            while let Some(chunk) = chunks.recv().await {
                if finished {
                    return Err("The speech provider sent audio after its final chunk.".to_string());
                }
                let clip = chunk.clip;
                bytes += clip.bytes.len();
                if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
                    || clip.bytes.is_empty()
                    || bytes > MAX_AUDIO
                {
                    return Err("The speech provider returned unusable audio.".to_string());
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
            if finished {
                Ok(())
            } else {
                Err("The speech provider ended before its final audio chunk.".to_string())
            }
        };
        let attempted = match tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(produce, consume)
        })
        .await
        {
            Ok((produced, consumed)) => consumed.and(produced.map_err(|e| e.to_string())),
            Err(_) => Err("The speech provider timed out.".to_string()),
        };
        if attempted.is_ok() || interrupted.is_cancelled() {
            return Ok(());
        }
        // A replacement after audible output would repeat words. Only an attempt
        // that produced no usable chunks may use the configured paid fallback.
        if !published
            && !services.tts.is_subscription()
            && let Some(fallback) = &services.fallback_tts
        {
            if interrupted.is_cancelled() {
                return Ok(());
            }
            if !self.connected(fallback.as_ref()) {
                return Err("The fallback voice provider is no longer connected.".into());
            }
            self.pay(
                Kind::Tts,
                ledger::tts_usd(&fallback.id().provider_id, text.chars().count()),
            )?;
            if interrupted.is_cancelled() {
                return Ok(());
            }
            let clip = tokio::time::timeout(Duration::from_secs(30), fallback.speak(text))
                .await
                .map_err(|_| "The fallback speech provider timed out.".to_string())?
                .map_err(|e| e.to_string())?;
            if !matches!(clip.mime.as_str(), "audio/wav" | "audio/mpeg")
                || clip.bytes.is_empty()
                || clip.bytes.len() > MAX_AUDIO
            {
                return Err("The speech provider returned unusable audio.".into());
            }
            self.clip(id, line, *index, final_sentence, &clip, Some(interrupted));
            *index += 1;
            return Ok(());
        }
        attempted
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
        let _ = self.change(id, |call| {
            if call.state != VoiceState::Held
                && !interrupted.is_some_and(CancellationToken::is_cancelled)
            {
                let sent = call.events.send(VoiceEvent::Clip {
                    id: line.into(),
                    index,
                    r#final: last,
                    mime_type: clip.mime.clone(),
                    data: STANDARD.encode(&clip.bytes),
                });
                if sent.is_ok()
                    && let Some(started) = call.first_clip_started.take()
                {
                    eprintln!(
                        "[voice] utterance to first clip call={} seq={}: {}ms",
                        call.id,
                        call.seq.unwrap_or_default(),
                        started.elapsed().as_millis()
                    );
                }
            }
            Ok(())
        });
    }
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
pub(super) fn speech_ready(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 320
        && spoken::sentences(text).len() <= 2
        && !text.contains(['`', '#', '*', '\n', '[', ']', '|'])
        && spoken::speech_text(text) == text
}

pub(super) fn goodbye(text: &str) -> bool {
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
pub(super) fn own_voice(
    settings: &VoiceSettings,
    voice: &crate::contract::PersonaVoice,
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
