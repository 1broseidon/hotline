//! How a scheduled turn finishes with nothing in the chat.
//!
//! The bug this exists for: a teammate told to check something every morning
//! and report only on a change still posted "No change — staying silent per
//! protocol", every morning. The instruction worked — the agent understood it
//! perfectly and said so. That is the problem. Any mechanism that asks the
//! model to produce no text is a mechanism the model can satisfy by producing
//! text about producing no text.
//!
//! So nothing here asks. A job the user marked quiet opens a window over its
//! own turn, and while that window is open an `agent` event is rewritten into
//! a `thought` before it reaches the tape. The rewrite is a function of the
//! event *kind* and of a boolean the user set — never of what the event says
//! — so:
//!
//! - no phrasing can defeat it, because no phrasing is read;
//! - the agent is never told it is being quiet, so it has nothing to announce;
//! - the words are not destroyed, only demoted: the transcript keeps them
//!   under thinking, where the window already leaves machinery off screen, so
//!   a quiet run is still debuggable.
//!
//! What stays loud, deliberately: `notice` (the error path), `permission`,
//! `human_action`, `peer`, `tool`, `computer_frame`, and the `turn` event
//! itself. Silence was asked of the agent's voice, not of the app.
//!
//! Both agent kinds are covered by one gate because both are downstream of
//! it: whichever driver ran the turn, its assistant text reaches the session's
//! funnel as an already-translated agent message, and that funnel is where
//! this runs.

use crate::contract::{ScheduledRun, TranscriptEvent};

/// The longest a schedule may hold a teammate's voice.
///
/// A turn that never ends — a wedged backend, a cancelled run whose boundary
/// never arrived — must not mute a teammate for the rest of the session. Past
/// this the window simply expires and the teammate speaks normally again.
pub const QUIET_MAX_MS: i64 = 30 * 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietWindow {
    /// Which job asked for the silence. Carried for diagnostics, not for
    /// logic.
    pub job_id: String,
    /// Turn boundaries owed to work that was already in flight when the
    /// schedule fired. A firing that lands mid-turn is queued behind the
    /// running one, so the first `turn` event to arrive belongs to that turn
    /// and not to ours.
    pub pending_turns: u32,
    /// Wall-clock expiry; see [`QUIET_MAX_MS`].
    pub until: i64,
}

/// Opens a window for a firing, or nothing when the job was not quiet.
pub fn open_window(run: &ScheduledRun, busy: bool, now: i64) -> Option<QuietWindow> {
    if run.quiet != Some(true) {
        return None;
    }
    Some(QuietWindow {
        job_id: run.job_id.clone(),
        pending_turns: u32::from(busy),
        until: now + QUIET_MAX_MS,
    })
}

/// Advances the window by one transcript event, and says what to write.
///
/// The answers are the window after this event — `None` once it has closed —
/// and the event to write in its place. The window closes on whichever comes
/// first: the turn boundary that belongs to the scheduled run, someone else
/// speaking, or the expiry. A human message closing it is the important one —
/// a person who types during a quiet run is owed an answer they can read, and
/// the schedule's silence was never about them.
pub fn step(
    window: QuietWindow,
    event: TranscriptEvent,
    now: i64,
) -> (Option<QuietWindow>, TranscriptEvent) {
    if now >= window.until {
        return (None, event);
    }
    match event {
        // Anything with a speaker behind it ends the silence. The firing's own
        // user event never reaches here: it opens its window after it is
        // stamped.
        event @ TranscriptEvent::User { .. } => (None, event),
        event @ TranscriptEvent::Turn { .. } if window.pending_turns > 0 => (
            Some(QuietWindow {
                pending_turns: window.pending_turns - 1,
                ..window
            }),
            event,
        ),
        event @ TranscriptEvent::Turn { .. } => (None, event),
        TranscriptEvent::Agent { id, ts, text, .. } if window.pending_turns == 0 => {
            (Some(window), TranscriptEvent::Thought { id, ts, text })
        }
        event => (Some(window), event),
    }
}

/// Whether a live agent delta should be shown as thinking instead.
///
/// Without this the composer's writing indicator runs for a message that will
/// never land — the app visibly typing and then producing nothing, which reads
/// as a bug rather than as silence.
pub fn mutes_deltas(window: Option<&QuietWindow>, now: i64) -> bool {
    match window {
        Some(window) => window.pending_turns == 0 && now < window.until,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    //! The gate that makes a scheduled turn able to say nothing.
    //!
    //! The property worth pinning down is negative: there is no event *text*
    //! that changes any answer here. Two of the cases below say the same
    //! sentence the bug report quoted — "No change — staying silent per
    //! protocol" — and one is muted while the other is not, decided entirely
    //! by whose turn it is.

    use super::*;
    use crate::contract::{
        HumanActionStatus, NoticeLevel, ScheduleKind, ToolStatus, TranscriptEvent,
    };

    const NOW: i64 = 1_700_000_000_000;

    fn quiet_job() -> ScheduledRun {
        ScheduledRun {
            job_id: "job-1".to_string(),
            kind: ScheduleKind::Loop,
            name: "Apple order check".to_string(),
            operator_created: false,
            quiet: Some(true),
        }
    }

    fn loud_job() -> ScheduledRun {
        ScheduledRun {
            job_id: "job-2".to_string(),
            kind: ScheduleKind::Loop,
            name: "Standup".to_string(),
            operator_created: false,
            quiet: None,
        }
    }

    fn agent(text: &str) -> TranscriptEvent {
        TranscriptEvent::Agent {
            id: "a1".to_string(),
            ts: NOW,
            text: text.to_string(),
            attachments: None,
            reactions: None,
            ring: None,
            receipt: None,
        }
    }

    fn thought(text: &str) -> TranscriptEvent {
        TranscriptEvent::Thought {
            id: "t1".to_string(),
            ts: NOW,
            text: text.to_string(),
        }
    }

    fn turn() -> TranscriptEvent {
        TranscriptEvent::Turn {
            id: "z".to_string(),
            ts: NOW,
            stop_reason: "end_turn".to_string(),
            usage: None,
        }
    }

    fn user(text: &str) -> TranscriptEvent {
        TranscriptEvent::User {
            id: "u1".to_string(),
            ts: NOW,
            text: text.to_string(),
            attachments: None,
            reactions: None,
            reply_to: None,
            scheduled: None,
            ring: None,
            receipt: None,
        }
    }

    fn open(busy: bool) -> QuietWindow {
        open_window(&quiet_job(), busy, NOW).expect("a quiet job opens a window")
    }

    /// The kind of each event, as the tape would name it.
    fn kinds(events: &[TranscriptEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| {
                serde_json::to_value(event).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    /// Walks a run of events through the machine, collecting what reached the
    /// tape.
    fn run(
        window: Option<QuietWindow>,
        events: Vec<TranscriptEvent>,
        now: i64,
    ) -> (Vec<TranscriptEvent>, Option<QuietWindow>) {
        let mut tape = Vec::new();
        let mut current = window;
        for event in events {
            match current.take() {
                Some(window) => {
                    let (next, written) = step(window, event, now);
                    current = next;
                    tape.push(written);
                }
                None => tape.push(event),
            }
        }
        (tape, current)
    }

    // -- opening ------------------------------------------------------------

    #[test]
    fn only_a_quiet_job_opens_a_window() {
        assert_eq!(open_window(&loud_job(), false, NOW), None);
        assert_eq!(
            open(false),
            QuietWindow {
                job_id: "job-1".to_string(),
                pending_turns: 0,
                until: NOW + QUIET_MAX_MS,
            }
        );
    }

    #[test]
    fn a_firing_that_lands_mid_turn_owes_the_running_turn_its_boundary() {
        assert_eq!(open(true).pending_turns, 1);
    }

    // -- the whole point ----------------------------------------------------

    #[test]
    fn a_quiet_turn_leaves_no_assistant_event_on_the_tape() {
        let (tape, _) = run(
            Some(open(false)),
            vec![
                thought("Checking the order page."),
                agent("No change — staying silent per protocol."),
                turn(),
            ],
            NOW,
        );
        assert_eq!(kinds(&tape), ["thought", "thought", "turn"]);
    }

    #[test]
    fn the_muted_words_are_demoted_not_destroyed() {
        let (tape, _) = run(
            Some(open(false)),
            vec![agent("No change — staying silent per protocol.")],
            NOW,
        );
        assert_eq!(
            tape[0],
            TranscriptEvent::Thought {
                id: "a1".to_string(),
                ts: NOW,
                text: "No change — staying silent per protocol.".to_string(),
            }
        );
    }

    #[test]
    fn the_same_sentence_is_a_bubble_when_no_window_is_open() {
        let (tape, _) = run(
            None,
            vec![agent("No change — staying silent per protocol.")],
            NOW,
        );
        assert_eq!(kinds(&tape), ["agent"]);
    }

    #[test]
    fn nothing_but_the_events_kind_is_read_so_any_text_mutes_identically() {
        for text in ["", "ok", "I will now be silent", "🙊", &"x".repeat(5_000)] {
            let (tape, _) = run(Some(open(false)), vec![agent(text)], NOW);
            assert_eq!(kinds(&tape), ["thought"]);
        }
    }

    // -- what stays loud ----------------------------------------------------

    #[test]
    fn an_error_notice_is_never_quiet() {
        let notice = TranscriptEvent::Notice {
            id: "n1".to_string(),
            ts: NOW,
            level: NoticeLevel::Error,
            text: "Turn failed: the model returned an error".to_string(),
        };
        let (tape, _) = run(Some(open(false)), vec![notice.clone(), turn()], NOW);
        assert_eq!(tape[0], notice);
    }

    #[test]
    fn tools_permissions_plans_and_hand_to_human_cards_all_survive_a_quiet_turn() {
        let events = vec![
            TranscriptEvent::Tool {
                id: "tool:1".to_string(),
                ts: NOW,
                tool_call_id: "1".to_string(),
                title: "Read page".to_string(),
                tool_kind: None,
                status: ToolStatus::Completed,
                locations: None,
                output: None,
            },
            TranscriptEvent::Permission {
                id: "p1".to_string(),
                ts: NOW,
                request_id: "r1".to_string(),
                title: "Run curl".to_string(),
                options: Vec::new(),
                decision: None,
                decided_option_name: None,
            },
            TranscriptEvent::Plan {
                id: "pl1".to_string(),
                ts: NOW,
                entries: Vec::new(),
            },
            TranscriptEvent::HumanAction {
                id: "h1".to_string(),
                ts: NOW,
                action_id: "h".to_string(),
                reason: "Tap 2FA".to_string(),
                status: HumanActionStatus::Pending,
                note: None,
            },
        ];
        let (tape, _) = run(Some(open(false)), events.clone(), NOW);
        assert_eq!(tape, events);
    }

    // -- closing ------------------------------------------------------------

    #[test]
    fn the_turn_boundary_closes_the_window_and_the_next_turn_speaks() {
        let (_, window) = run(Some(open(false)), vec![agent("quiet one"), turn()], NOW);
        assert_eq!(window, None);
        let (tape, _) = run(window, vec![agent("loud one")], NOW);
        assert_eq!(kinds(&tape), ["agent"]);
    }

    #[test]
    fn a_firing_behind_a_running_turn_stays_quiet_across_that_turns_boundary() {
        let (tape, window) = run(
            Some(open(true)),
            vec![
                agent("the answer to what the human asked"),
                turn(),
                agent("the scheduled run's own words"),
                turn(),
            ],
            NOW,
        );
        assert_eq!(kinds(&tape), ["agent", "turn", "thought", "turn"]);
        assert_eq!(window, None);
    }

    #[test]
    fn a_person_typing_during_a_quiet_run_gets_answered() {
        let (tape, window) = run(
            Some(open(false)),
            vec![user("wait, what did you find?"), agent("It moved.")],
            NOW,
        );
        assert_eq!(window, None);
        assert_eq!(kinds(&tape), ["user", "agent"]);
    }

    #[test]
    fn a_wedged_turn_cannot_mute_a_teammate_forever() {
        let late = NOW + QUIET_MAX_MS + 1;
        let (tape, window) = run(Some(open(false)), vec![agent("hours later")], late);
        assert_eq!(window, None);
        assert_eq!(kinds(&tape), ["agent"]);
    }

    // -- the live stream ----------------------------------------------------

    #[test]
    fn deltas_are_muted_exactly_while_the_tape_is() {
        assert!(!mutes_deltas(None, NOW));
        assert!(mutes_deltas(Some(&open(false)), NOW));
        assert!(!mutes_deltas(Some(&open(true)), NOW));
        assert!(!mutes_deltas(Some(&open(false)), NOW + QUIET_MAX_MS));
    }
}
