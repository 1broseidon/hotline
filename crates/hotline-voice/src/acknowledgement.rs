//! A Turn by turn call to a teammate is quiet while the teammate's model
//! thinks, often for ten seconds. With `voice.acknowledge` on, the desk's
//! fast model (the call assistant) writes one short line naming what the
//! person asked, and the desk says it in the teammate's own voice while the
//! teammate starts, as the teammate's line.
//!
//! The person's turn goes to the teammate at once, as with the setting off:
//! a hosted fast model does not answer in the half second a hand-off could
//! wait, so the line is asked for beside the hand-off, never before it. Its
//! words cannot be in the teammate's prompt, so the prompt says instead that
//! the desk will acknowledge the request ([`spoken::ACKNOWLEDGED_CONTRACT`])
//! and asks for no opening line of its own. The line is said only if the
//! teammate has no piece of its own to say yet and its sound is ready within
//! [`ACK_CAP`] of the person's last word: a late acknowledgement is worse
//! than none. Nothing here changes a call with the setting off.

use super::dispatcher::Dispatcher;
use super::ledger::{self, Kind};
use super::record::Speaker;
use super::speech::Clip;
use super::synthesis::playable;
use super::{Calls, progress, spoken};
use hotline_contract::{VoiceEvent, VoiceState};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// How long after the person's last word the line's sound may be ready.
/// One later than that is never said.
pub(crate) const ACK_CAP: Duration = Duration::from_secs(4);

/// A teammate whose first words came this quickly on its last turn needs no
/// line to cover its silence.
pub(crate) const QUICK_TEAMMATE: Duration = Duration::from_secs(3);

/// A line longer than this is a model that ignored its rules, and may be
/// answering the question rather than naming it.
const MOST_WORDS: usize = 12;

/// Why no acknowledgement was said for a turn, in the log.
fn skipped(id: &str, why: &str) {
    eprintln!("[voice] call {id}: no acknowledgement: {why}");
}

impl Calls {
    /// Starts on a line to say for the teammate while it starts on `words`,
    /// and has the turn's prompt told one is coming
    /// ([`Self::turn_instruction`]). None, and nothing changed, when the
    /// setting is off, the call is a live one or the desk's own, the teammate
    /// answered quickly last time and is on no earlier turn, or there is no
    /// call assistant or Chat budget for it. Words said while the teammate is
    /// still on an earlier turn wait behind it, so they are acknowledged. The sender answers whether the
    /// teammate was told: until it says so, nothing is played.
    ///
    /// With the setting on, a direct call's turn may also have one line on
    /// what the teammate is doing if it works in silence
    /// ([`super::progress`]), whether or not an acknowledgement is asked for.
    pub(super) fn acknowledge(
        self: &Arc<Self>,
        id: &str,
        seq: u32,
        target: &str,
        words: &str,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> Option<oneshot::Sender<bool>> {
        if !self.settings().acknowledge {
            return None;
        }
        let (direct, busy, quick) = self
            .change(id, |call| {
                let direct = !call.desk && call.front.is_none();
                if direct {
                    let since_speech_end = call
                        .turn_timing
                        .as_ref()
                        .map_or(Duration::ZERO, super::TurnTiming::since_speech_end);
                    call.progress.arm(seq, since_speech_end);
                }
                Ok((
                    direct,
                    call.answering.is_some(),
                    call.last_llm_spoken
                        .is_some_and(|spoken| spoken < QUICK_TEAMMATE),
                ))
            })
            .ok()?;
        if !direct {
            return None;
        }
        // Words said while the teammate is still on an earlier turn wait for
        // that turn to end, however quick the teammate is: the line is how
        // the person hears they were taken.
        if quick && !busy {
            skipped(id, "the teammate answered quickly last time");
            return None;
        }
        let Ok(assistant) = self.dispatcher() else {
            skipped(id, "there is no call assistant");
            return None;
        };
        if !assistant.free() && self.ledger.ready(&[Kind::Dispatcher]).is_err() {
            skipped(id, "the Chat budget has no room for it");
            return None;
        }
        let name = hotline_room::room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == target)
            .map(|persona| persona.name)?;
        let cap = self
            .change(id, |call| {
                call.acknowledging = true;
                Ok(call.turn_timing.as_ref().map_or(ACK_CAP, |timing| {
                    ACK_CAP.saturating_sub(timing.since_speech_end())
                }))
            })
            .ok()?;
        let (tell, told) = oneshot::channel();
        tokio::spawn(self.clone().say_acknowledgement(
            Turn {
                id: id.to_string(),
                seq,
                name,
                words: words.to_string(),
            },
            assistant,
            cap,
            told,
            interrupted.clone(),
            ended.clone(),
        ));
        Some(tell)
    }

    /// The hand-off is over: the teammate was told a line is coming if its
    /// prompt went and took the news. News no prompt took is not kept for the
    /// next one.
    pub(super) fn told(&self, id: &str, tell: oneshot::Sender<bool>, prompted: bool) {
        let carried = self
            .change(id, |call| Ok(!std::mem::take(&mut call.acknowledging)))
            .unwrap_or(false);
        let _ = tell.send(prompted && carried);
    }

    /// Asks the call assistant for the line and synthesizes it in the call's
    /// own voice (never its fallback voice, which would be someone else's),
    /// then says it as the teammate's line once the teammate has been told of
    /// it, unless the person has spoken over the call, held it or hung up, a
    /// newer turn has begun, the teammate has a piece of its own to say, or
    /// the sound was not ready within `cap`. Its clip counts as speech on the
    /// turn, so the waiting tone and the overrun note count from it, and it is
    /// kept on the call's thread and in what the teammate's later turns are
    /// told was said. Nothing it fails at ends the call.
    async fn say_acknowledgement(
        self: Arc<Self>,
        turn: Turn,
        assistant: Arc<dyn Dispatcher>,
        cap: Duration,
        told: oneshot::Receiver<bool>,
        interrupted: CancellationToken,
        ended: CancellationToken,
    ) {
        let id = turn.id.as_str();
        let ready = async {
            let line = assistant
                .acknowledge(&turn.name, &turn.words, self.ledger.clone())
                .await?;
            let line = sayable(&line).ok_or("the call assistant wrote no line to say")?;
            if self.change(id, |call| Ok(own_words(call))).unwrap_or(true) {
                return Err("the teammate spoke first".to_string());
            }
            let clip = self.teammate_sound(id, &line).await?;
            Ok((line, clip))
        };
        let outcome = tokio::select! {
            () = interrupted.cancelled() => Err("the person spoke over the call".to_string()),
            () = ended.cancelled() => return,
            both = tokio::time::timeout(cap, async { tokio::join!(ready, told) }) => match both {
                Ok((Ok(said), Ok(true))) => Ok(said),
                Ok((Err(why), _)) => Err(why),
                Ok((Ok(_), _)) => Err("the teammate's turn was not told of it".to_string()),
                Err(_) => Err(format!("it was not ready within {ACK_CAP:?} of the person's last word")),
            },
        };
        let (line, clip) = match outcome {
            Ok(said) => said,
            Err(why) => return skipped(id, &why),
        };
        let length = progress::sound_length(&clip, &line);
        let said = self.say_for_teammate(id, turn.seq, &line, &clip, &interrupted, |call, said| {
            call.ack_line = Some(said.to_string());
            if let Some(timing) = &mut call.turn_timing {
                timing.mark_ack();
            }
            call.progress.acknowledged(turn.seq, &line, length);
        });
        if let Err(why) = said {
            skipped(id, why);
        }
    }

    /// Says `line`, whose sound is `clip`, as the teammate's on turn `seq`,
    /// unless the person has spoken again or over the call, held it, or the
    /// teammate has words of its own ready: `mark` notes the line on the
    /// call by the id it is said under. It is kept on the call's thread and
    /// in what later turns are told was said, and its clip counts as speech
    /// on the turn. A call that has ended says nothing and gives no reason.
    pub(super) fn say_for_teammate(
        &self,
        id: &str,
        seq: u32,
        line: &str,
        clip: &Clip,
        interrupted: &CancellationToken,
        mark: impl FnOnce(&mut super::Call, &str),
    ) -> Result<(), &'static str> {
        let said = self.change(id, |call| {
            let why = if call.seq != Some(seq) {
                Some("the person has spoken again")
            } else if call.state == VoiceState::Held {
                Some("the call is on hold")
            } else if interrupted.is_cancelled() {
                Some("the person spoke over the call")
            } else if own_words(call) {
                Some("the teammate spoke first")
            } else {
                None
            };
            if let Some(why) = why {
                return Ok(Err(why));
            }
            let said = uuid::Uuid::new_v4().to_string();
            mark(call, &said);
            call.conversation.remember(line);
            Ok(Ok(said))
        });
        let said = match said {
            Ok(said) => said?,
            Err(_) => return Ok(()),
        };
        self.keep_as(id, Speaker::Voice, &said, line);
        self.emit(
            id,
            VoiceEvent::Said {
                id: said.clone(),
                text: line.to_string(),
            },
        );
        self.clip(id, &said, 0, true, clip, Some(interrupted));
        Ok(())
    }

    /// A line's sound in the call's own voice, its speech asked of the
    /// Voice budget first. Its own voice or nothing: a fallback voice would
    /// be someone else.
    pub(super) async fn teammate_sound(&self, id: &str, line: &str) -> Result<Clip, String> {
        let voice = self.speech_for(id)?.tts;
        self.pay(
            Kind::Tts,
            ledger::tts_usd(&voice.id().provider_id, line.chars().count()),
        )?;
        match voice.speak(line).await {
            Ok(clip) if playable(&clip, clip.bytes.len()) => Ok(clip),
            Ok(_) => Err("its voice sent no sound a client can play".to_string()),
            Err(error) => Err(format!("its voice failed: {error}")),
        }
    }
}

/// Whether the teammate has a piece of its own to say on the call's turn, or
/// is saying it: its first piece is marked before it is synthesized, and the
/// turn's timing is gone once its first clip went out.
pub(super) fn own_words(call: &super::Call) -> bool {
    call.turn_timing
        .as_ref()
        .is_none_or(super::TurnTiming::first_unit_ready)
}

/// The turn a line is for: the call, the person's turn on it, the teammate's
/// name and what the person said.
struct Turn {
    id: String,
    seq: u32,
    name: String,
    words: String,
}

/// The model's line as it can be said: one line, without quotes or markup,
/// of a few words; None for nothing, or for more than it was asked for.
fn sayable(line: &str) -> Option<String> {
    let line = line.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let line = spoken::speech_text(line.trim_matches(['"', '“', '”']).trim());
    let words = line.split_whitespace().count();
    (words > 0 && words <= MOST_WORDS).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::ACKNOWLEDGE;

    #[test]
    fn a_line_is_said_only_when_it_is_one_short_line() {
        assert_eq!(
            sayable("  \"Looking into the failing scheduler test.\"\n").as_deref(),
            Some("Looking into the failing scheduler test.")
        );
        assert_eq!(sayable(""), None);
        assert_eq!(sayable("   "), None);
        assert_eq!(sayable("\"\""), None);
        assert_eq!(sayable("The deploy logs.\nThey show a timeout."), None);
        assert_eq!(
            sayable(
                "It is the cache, which expired at noon when the job ran and the deploy failed after it."
            ),
            None,
            "a long line answers rather than names"
        );
    }

    #[test]
    fn the_prompt_names_the_topic_and_forbids_answers_promises_and_filler() {
        for rule in [
            "at most eight words",
            "Name the topic only",
            "Never say an action is done",
            "never promise a result",
            "never answer the question",
            "first person",
            "Sure, Got it, Okay",
            "let me check that for you",
            "handing that off",
            "an empty reply",
            "small talk",
            "a yes or a no",
            "a correction",
            "A question still gets a line",
        ] {
            assert!(ACKNOWLEDGE.contains(rule), "{rule}");
        }
    }
}
