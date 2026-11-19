//! A Turn by turn call to a teammate is quiet while the teammate's model
//! thinks, often for ten seconds. With `voice.acknowledge` on, the desk's
//! fast model (the call assistant) writes one short line naming what the
//! person asked, and the desk says it in the teammate's own voice while the
//! teammate starts, as the teammate's line.
//!
//! The line is said only if the teammate was told of it: its turn's prompt
//! carries the line as already said, so it goes straight to the work instead
//! of acknowledging again. The hand-off waits at most [`ACK_WAIT`] for it,
//! and a line later than that is dropped. A line whose sound is ready after
//! the teammate's own first piece is ready to say is dropped too: the
//! teammate's words come first. Nothing here changes a call with the setting
//! off.

use super::ledger::{self, Kind};
use super::record::Speaker;
use super::synthesis::playable;
use super::{Calls, spoken};
use hotline_contract::{VoiceEvent, VoiceState};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// How long a turn's hand-off to the teammate waits for the line. One that
/// comes later is never said: the teammate was not told of it.
pub(crate) const ACK_WAIT: Duration = Duration::from_millis(500);

/// A teammate whose first words came this quickly on its last turn needs no
/// line to cover its silence.
pub(crate) const QUICK_TEAMMATE: Duration = Duration::from_secs(3);

/// A line longer than this is a model that ignored its rules, and may be
/// answering the question rather than naming it.
const MOST_WORDS: usize = 12;

impl Calls {
    /// The line to say for the teammate while it starts on `words`, or None:
    /// the setting is off, the call is a live one or the desk's own, the
    /// teammate is still on an earlier turn or answered quickly last time,
    /// there is no call assistant, it wrote nothing, failed, was refused by
    /// the Chat budget, or took longer than [`ACK_WAIT`].
    pub(super) async fn acknowledgement(
        &self,
        id: &str,
        target: &str,
        words: &str,
        interrupted: &CancellationToken,
    ) -> Option<String> {
        if !self.settings().acknowledge {
            return None;
        }
        let wanted = self
            .change(id, |call| {
                Ok(!call.desk
                    && call.front.is_none()
                    && call.answering.is_none()
                    && call
                        .last_llm_spoken
                        .is_none_or(|spoken| spoken >= QUICK_TEAMMATE))
            })
            .unwrap_or(false);
        if !wanted {
            return None;
        }
        let assistant = self.dispatcher().ok()?;
        let name = hotline_room::room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == target)
            .map(|persona| persona.name)?;
        let skipped = |why: &str| {
            eprintln!("[voice] call {id}: no acknowledgement: {why}");
        };
        let line = match tokio::time::timeout(
            ACK_WAIT,
            assistant.acknowledge(&name, words, self.ledger.clone()),
        )
        .await
        {
            Ok(Ok(line)) => line,
            Ok(Err(error)) => {
                skipped(&error);
                return None;
            }
            Err(_) => {
                skipped(&format!("it took longer than {ACK_WAIT:?}"));
                return None;
            }
        };
        if interrupted.is_cancelled() {
            return None;
        }
        sayable(&line)
    }

    /// Has the next prompt of the call carry `line` as already said, and
    /// starts saying it. The sender answers whether the teammate was told:
    /// until it says so, nothing is played.
    pub(super) fn acknowledge(
        self: &Arc<Self>,
        id: &str,
        seq: u32,
        line: String,
        interrupted: &CancellationToken,
        ended: &CancellationToken,
    ) -> oneshot::Sender<bool> {
        let _ = self.change(id, |call| {
            call.acknowledged = Some(line.clone());
            Ok(())
        });
        let (tell, told) = oneshot::channel();
        tokio::spawn(self.clone().say_acknowledgement(
            id.to_string(),
            seq,
            line,
            told,
            interrupted.clone(),
            ended.clone(),
        ));
        tell
    }

    /// The hand-off is over: the teammate was told of the line if its prompt
    /// went and took it. A line no prompt took is not kept for the next one.
    pub(super) fn told(&self, id: &str, tell: oneshot::Sender<bool>, prompted: bool) {
        let carried = self
            .change(id, |call| Ok(call.acknowledged.take().is_none()))
            .unwrap_or(false);
        let _ = tell.send(prompted && carried);
    }

    /// Synthesizes the line in the call's own voice at once (never its
    /// fallback voice, which would be someone else's), and says it as
    /// the teammate's line once the teammate has been told of it, unless the
    /// person has spoken over the call, held it or hung up, a newer turn has
    /// begun, or the teammate's first piece was ready first. Its clip counts
    /// as speech on the turn, so the waiting tone and the overrun note count
    /// from it. A refused or failed voice says nothing for it.
    async fn say_acknowledgement(
        self: Arc<Self>,
        id: String,
        seq: u32,
        line: String,
        told: oneshot::Receiver<bool>,
        interrupted: CancellationToken,
        ended: CancellationToken,
    ) {
        let Ok(speech) = self.speech_for(&id) else {
            return;
        };
        let voice = speech.tts;
        let synthesized = async {
            // Its own voice or nothing: a fallback voice would be someone else.
            self.pay(
                Kind::Tts,
                ledger::tts_usd(&voice.id().provider_id, line.chars().count()),
            )?;
            match voice.speak(&line).await {
                Ok(clip) if playable(&clip, clip.bytes.len()) => Ok(clip),
                Ok(_) => Err("its voice sent no sound a client can play".to_string()),
                Err(error) => Err(error.to_string()),
            }
        };
        let (clip, told) = tokio::select! {
            () = interrupted.cancelled() => return,
            () = ended.cancelled() => return,
            both = async { tokio::join!(synthesized, told) } => both,
        };
        if !matches!(told, Ok(true)) {
            return;
        }
        let clip = match clip {
            Ok(clip) => clip,
            Err(error) => {
                eprintln!("[voice] call {id}: the acknowledgement was not said: {error}");
                return;
            }
        };
        let said = self.change(&id, |call| {
            let teammate_first = call
                .turn_timing
                .as_ref()
                .is_none_or(super::TurnTiming::first_unit_ready);
            if call.seq != Some(seq)
                || call.state == VoiceState::Held
                || interrupted.is_cancelled()
                || teammate_first
            {
                return Ok(None);
            }
            let said = uuid::Uuid::new_v4().to_string();
            call.ack_line = Some(said.clone());
            if let Some(timing) = &mut call.turn_timing {
                timing.mark_ack();
            }
            call.conversation.remember(&line);
            Ok(Some(said))
        });
        let Ok(Some(said)) = said else {
            return;
        };
        self.keep_as(&id, Speaker::Voice, &said, &line);
        self.emit(
            &id,
            VoiceEvent::Said {
                id: said.clone(),
                text: line,
            },
        );
        self.clip(&id, &said, 0, true, &clip, Some(&interrupted));
    }
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
            "a quick direct answer",
        ] {
            assert!(ACKNOWLEDGE.contains(rule), "{rule}");
        }
    }
}
