//! Runs: a teammate's agent started fresh for one piece of work, driven to
//! its end on a stream of its own, and what it reported handed back.
//!
//! Two kinds of hidden work share this, and a third is meant to:
//!
//! - **A subagent.** A Hotline Agent teammate hands a task to a worker that
//!   runs as itself — its working directory, its tools, the model it is on
//!   right now — with none of its conversation and none of its persona. The
//!   worker's system prompt is this module's, written for doing a job and
//!   reporting on it, never the teammate's own. [`Room::run`] is the whole of
//!   it; the teammate reaches it through its managed jobs (see
//!   [`super::jobs::Delegate`]), which is what makes it cancellable, waitable
//!   and parallel without a line of scheduling here.
//! - **A peer exchange.** One teammate asking another is a run of the other's
//!   agent on the pair's thread: [`drive`] is the loop both use, so a turn
//!   nobody in the room is watching is written down the same way whoever
//!   started it.
//! - **A scheduled firing**, next: a quiet job is a run whose report nobody
//!   sees unless it escalates.
//!
//! A harness that runs subagents of its own and shows them — Claude's and
//! Codex's ACP adapters do — has each written here as a run too
//! ([`Room::watch_subagents`]): the harness does the work, and the room keeps
//! the marker and the transcript the same way it does for its own.
//!
//! A run's agent is built by [`Room::thread_agent`] under the run kind's
//! policy, and its one turn is [`super::threads::Threads::turn`], the turn
//! a side thread's agent takes.
//!
//! A run's lines are written through [`super::threads::Threads::write`], which
//! indexes what it said for `search_thread` and expires a permission card it
//! raises: nobody is looking at a run to answer one.
//!
//! A run never writes to its teammate's tape. The one line it keeps there is
//! its [`Link`], rewritten by id as the run goes, which a client is sent as
//! the `subagent` marker the person presses to open the run's own transcript.
//!
//! Authority follows [`crate::driver::CapabilityLease`]: a run's lease is a
//! child of the session that started it, so stopping that teammate, changing
//! its policy or deleting it revokes every run it has going, and a run can
//! never hold more than its parent did.

use super::agent::{Opening, lease_of};
use super::turns::{Line, Seat};
use super::{
    CLOCK, PendingTool, Room, event_of, narration, new_id, now_ms, reach_sentence, skills_index,
    timed,
};
use crate::contract::{
    NoticeLevel, Persona, Reach, RunningSubagent, SessionInfo, SubagentStatus, ToolStatus,
    TranscriptEvent,
};
use crate::driver::{
    CapabilityLease, Driver, HOTLINE_BACKEND_ID, MessageKind, SubagentReport, Update,
};
use crate::session::jobs::{Delegate, Finished, JobState, SubagentTask};
use crate::thread::{End, Link, ThreadId, ThreadKind, ThreadState};
use futures_util::future::BoxFuture;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

/// What one driver turn came to, as the stream it was written to saw it.
#[derive(Debug, Default)]
pub(super) struct Driven {
    /// Every agent line, in order.
    pub replies: Vec<String>,
    /// The agent lines since the last tool card: what it said once the work
    /// was done, which is the report.
    pub last_words: Vec<String>,
    /// The error the turn ended on, when it ended on one.
    pub failure: Option<String>,
    /// Whether the agent raised a permission card.
    pub asked: bool,
    /// Whether the cancel handed in stopped it.
    pub cancelled: bool,
    /// How the driver said the turn ended.
    pub stop_reason: Option<String>,
}

/// Drives one prompt on a driver to its end, handing every event it becomes
/// to `write`, with whether it came of a permission request.
///
/// The funnel is the tape's: what the agent says between its tool calls is
/// thinking, a tool left running when the driver stops is failed, and the
/// words are split into bubbles the way every stream splits them. A cancel
/// asks the driver to stop and keeps reading until it has, so the turn's own
/// end is still written down.
pub(super) async fn drive(
    driver: &dyn Driver,
    text: String,
    reach: Reach,
    cancel: Option<&CancellationToken>,
    write: impl FnMut(TranscriptEvent, bool),
) -> Driven {
    drive_with(
        driver,
        timed(now_ms(), &text),
        Vec::new(),
        reach,
        cancel,
        |_, _, _, _| {},
        write,
    )
    .await
}

/// What a kind does about each thing its agent does in a turn, the one place
/// the kinds differ in how a turn is driven. [`drive_updates`] reads the turn
/// off the driver and calls it; the funnel it feeds (what the agent says
/// between tool calls is thinking, a tool left running is failed, the words
/// are split into bubbles) is the same whoever is listening.
pub(super) trait Witness {
    /// An update, before it becomes events or a delta. The turn's tools still
    /// running are `in_flight`, for the one that has to fail them first.
    fn heard(&mut self, _update: &Update, _in_flight: &mut HashMap<String, PendingTool>) {}

    /// The words as they arrive (kind, message id, text, and whether the reply
    /// is being held back as narration), before the message is whole.
    fn delta(&mut self, _kind: MessageKind, _message_id: &str, _text: &str, _muted: bool) {}

    /// One event of the turn, and whether it came of a permission request.
    fn write(&mut self, event: TranscriptEvent, asked: bool);

    /// A look at the lines waiting behind the turn, between one update and the
    /// next, for a kind whose person can steer a turn in flight.
    fn steer(&mut self) {}
}

/// A witness made of two closures: how a peer turn and a run are written.
struct Calls<D, W> {
    delta: D,
    write: W,
}

impl<D, W> Witness for Calls<D, W>
where
    D: FnMut(MessageKind, &str, &str, bool),
    W: FnMut(TranscriptEvent, bool),
{
    fn delta(&mut self, kind: MessageKind, message_id: &str, text: &str, muted: bool) {
        (self.delta)(kind, message_id, text, muted);
    }

    fn write(&mut self, event: TranscriptEvent, asked: bool) {
        (self.write)(event, asked);
    }
}

/// [`drive`] for a conversation somebody is watching: the line is handed over
/// already stamped, with its attachments, and `delta` is told the words as
/// they arrive (kind, message id, text, and whether the reply is being held
/// back as narration) so they can be shown before the message is whole.
pub(super) async fn drive_with(
    driver: &dyn Driver,
    wire_text: String,
    attachments: Vec<crate::contract::Attachment>,
    reach: Reach,
    cancel: Option<&CancellationToken>,
    delta: impl FnMut(MessageKind, &str, &str, bool),
    write: impl FnMut(TranscriptEvent, bool),
) -> Driven {
    let updates = driver.prompt(wire_text, attachments, reach).await;
    drive_updates(driver, updates, cancel, None, &mut Calls { delta, write }).await
}

/// Reads one turn's updates off the driver to its end and tells `witness`
/// what each came to.
///
/// `ready` wakes the loop when a line is queued behind the turn, so a witness
/// that can steer is asked to look at once rather than at the next update. A
/// cancel asks the driver to stop and keeps reading until it has, so the
/// turn's own end is still written down.
pub(super) async fn drive_updates(
    driver: &dyn Driver,
    mut updates: mpsc::Receiver<Update>,
    cancel: Option<&CancellationToken>,
    ready: Option<&Notify>,
    witness: &mut impl Witness,
) -> Driven {
    let mut in_flight = HashMap::new();
    let mut voice = narration::Voice::new();
    let mut driven = Driven::default();
    loop {
        witness.steer();
        let received = tokio::select! {
            biased;
            () = async {
                match cancel {
                    Some(cancel) if !driven.cancelled => cancel.cancelled().await,
                    _ => std::future::pending().await,
                }
            } => {
                driven.cancelled = true;
                driver.cancel();
                continue;
            }
            () = async {
                match ready {
                    Some(ready) => ready.notified().await,
                    None => std::future::pending().await,
                }
            } => continue,
            update = updates.recv() => update,
        };
        let (batch, done) = match received {
            Some(update) => (voice.step(update), false),
            None => (voice.finish(), true),
        };
        for update in batch {
            witness.heard(&update, &mut in_flight);
            if let Update::Delta {
                kind,
                message_id,
                text,
            } = &update
            {
                let muted = *kind == MessageKind::Agent && voice.mutes_deltas();
                witness.delta(*kind, message_id, text, muted);
            }
            let asked = matches!(update, Update::Permission { .. });
            driven.asked |= asked;
            for event in event_of(update, &mut in_flight) {
                match &event {
                    TranscriptEvent::Agent { text, .. } => {
                        driven.replies.push(text.clone());
                        driven.last_words.push(text.clone());
                    }
                    TranscriptEvent::Tool { .. } => driven.last_words.clear(),
                    TranscriptEvent::Notice {
                        level: NoticeLevel::Error,
                        text,
                        ..
                    } => driven.failure = Some(text.clone()),
                    TranscriptEvent::Turn { stop_reason, .. } => {
                        driven.stop_reason = Some(stop_reason.clone());
                    }
                    _ => {}
                }
                witness.write(event, asked);
            }
        }
        if done {
            break;
        }
    }
    // A driver that stopped without a turn leaves a tool spinning in the
    // stream forever, exactly as it would on a tape.
    for (call_id, pending) in in_flight.drain() {
        witness.write(pending.event(&call_id, ToolStatus::Failed, None), false);
    }
    driven
}

/// One subagent run, as its teammate's session asked for it.
pub(crate) struct RunSpec {
    pub persona_id: String,
    /// Minted by the job that launched it: the job id, the run's stream, and
    /// the marker's id are one name.
    pub run_id: String,
    pub title: String,
    pub task: String,
    /// The authority of the session that asked. The run's own is a child of
    /// it, never wider.
    pub capability: CapabilityLease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunEnd {
    Done,
    Failed,
    Cancelled,
}

/// How a run ended and what it reported.
#[derive(Debug)]
pub(crate) struct RunOutcome {
    pub end: RunEnd,
    pub report: String,
}

impl Room {
    /// Runs one subagent to its end: a fresh agent for the teammate, told
    /// this module's brief instead of the teammate's preamble, handed the
    /// task, and driven on `runs/<runId>` while a marker on the teammate's
    /// tape says how it is going.
    ///
    /// Always answers, never errors: a run that could not start ended, and
    /// how it ended is the answer the job hands back.
    pub(crate) async fn run(
        self: &Arc<Self>,
        spec: RunSpec,
        cancel: CancellationToken,
    ) -> RunOutcome {
        let mut running = Running {
            room: Arc::downgrade(self),
            persona_id: spec.persona_id.clone(),
            run_id: spec.run_id.clone(),
            title: spec.title.clone(),
            started: now_ms(),
            driver: None,
            lease: None,
            settled: false,
        };
        running.mark(ThreadState::Live, None);
        self.threads().write(
            &ThreadId::run(&spec.run_id),
            &spec.persona_id,
            &TranscriptEvent::User {
                id: new_id(),
                ts: running.started,
                text: spec.task.clone(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
                client: None,
            },
        );
        let outcome = match self.run_to_end(&spec, &cancel, &mut running).await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.threads().write(
                    &ThreadId::run(&spec.run_id),
                    &spec.persona_id,
                    &TranscriptEvent::Notice {
                        id: new_id(),
                        ts: now_ms(),
                        level: NoticeLevel::Error,
                        text: error.clone(),
                    },
                );
                RunOutcome {
                    end: if cancel.is_cancelled() {
                        RunEnd::Cancelled
                    } else {
                        RunEnd::Failed
                    },
                    report: format!("The subagent could not run: {error}"),
                }
            }
        };
        running.settle(outcome.end);
        outcome
    }

    async fn run_to_end(
        self: &Arc<Self>,
        spec: &RunSpec,
        cancel: &CancellationToken,
        running: &mut Running,
    ) -> Result<RunOutcome, String> {
        let _working = self.working()?;
        spec.capability.check()?;
        let persona = self.persona(&spec.persona_id)?;
        if persona.backend_id != HOTLINE_BACKEND_ID {
            return Err("Only a Hotline Agent teammate runs subagents.".to_string());
        }
        let lease = lease_of(ThreadKind::Run, &spec.capability);
        running.lease = Some(lease.clone());
        let view = on_the_sessions_model(persona, &self.info(&spec.persona_id));
        let thread = ThreadId::run(&spec.run_id);
        let mut agent = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Ok(RunOutcome {
                    end: RunEnd::Cancelled,
                    report: "The subagent was stopped before it started.".to_string(),
                });
            }
            built = self.thread_agent(Opening {
                thread: thread.clone(),
                persona: view,
                title: spec.title.clone(),
                opener: None,
                lease,
            }) => built?,
        };
        // The run settles its own agent, however it ends.
        agent.keep();
        running.driver = Some(agent.driver.clone());
        let reach = agent.view.reach.unwrap_or_default();
        let driven = self
            .threads()
            .turn(
                Seat {
                    thread: &thread,
                    persona_id: &spec.persona_id,
                    driver: agent.driver.as_ref(),
                },
                Line {
                    text: timed(now_ms(), &brief(&agent.view.name, &spec.task)),
                    attachments: Vec::new(),
                    handoff: None,
                },
                reach,
                Some(cancel),
                || true,
            )
            .await;
        Ok(outcome_of(driven))
    }

    /// Writes the subagents a harness reports as runs of `persona_id`'s, for
    /// as long as the harness reports them. A subagent the harness never said
    /// the end of is settled as cancelled when the reports stop.
    pub(super) fn watch_subagents(
        self: &Arc<Self>,
        persona_id: String,
        mut reports: mpsc::UnboundedReceiver<SubagentReport>,
    ) {
        let room = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut runs = HashMap::new();
            while let Some(report) = reports.recv().await {
                let Some(room) = room.upgrade() else {
                    break;
                };
                room.harness_report(&persona_id, &mut runs, report);
            }
        });
    }

    fn harness_report(
        self: &Arc<Self>,
        persona_id: &str,
        runs: &mut HashMap<String, HarnessRun>,
        report: SubagentReport,
    ) {
        match report {
            SubagentReport::Started { child, title, task } => {
                if runs.contains_key(&child) {
                    return;
                }
                let running = Running {
                    room: Arc::downgrade(self),
                    persona_id: persona_id.to_string(),
                    run_id: new_id(),
                    title,
                    started: now_ms(),
                    driver: None,
                    lease: None,
                    settled: false,
                };
                running.mark(ThreadState::Live, None);
                if !task.trim().is_empty() {
                    self.threads().write(
                        &ThreadId::run(&running.run_id),
                        persona_id,
                        &TranscriptEvent::User {
                            id: new_id(),
                            ts: running.started,
                            text: task,
                            attachments: None,
                            reactions: None,
                            reply_to: None,
                            scheduled: None,
                            ring: None,
                            receipt: None,
                            client: None,
                        },
                    );
                }
                runs.insert(
                    child,
                    HarnessRun {
                        running,
                        voice: narration::Voice::new(),
                        in_flight: HashMap::new(),
                    },
                );
            }
            SubagentReport::Update { child, update } => {
                let Some(run) = runs.get_mut(&child) else {
                    return;
                };
                for update in run.voice.step(update) {
                    for event in event_of(update, &mut run.in_flight) {
                        self.threads().write(
                            &ThreadId::run(&run.running.run_id),
                            persona_id,
                            &event,
                        );
                    }
                }
            }
            SubagentReport::Ended { child, status } => {
                let Some(mut run) = runs.remove(&child) else {
                    return;
                };
                let run_id = run.running.run_id.clone();
                for update in run.voice.finish() {
                    for event in event_of(update, &mut run.in_flight) {
                        self.threads()
                            .write(&ThreadId::run(&run_id), persona_id, &event);
                    }
                }
                for (call_id, pending) in run.in_flight.drain() {
                    self.threads().write(
                        &ThreadId::run(&run_id),
                        persona_id,
                        &pending.event(&call_id, ToolStatus::Failed, None),
                    );
                }
                run.running.settle(match status {
                    SubagentStatus::Done => RunEnd::Done,
                    SubagentStatus::Failed => RunEnd::Failed,
                    SubagentStatus::Running | SubagentStatus::Cancelled => RunEnd::Cancelled,
                });
            }
        }
    }
}

/// How a driven turn ends as a run: stopped if it was stopped, failed if it
/// ended on an error, done otherwise — with the words that say so.
fn outcome_of(driven: Driven) -> RunOutcome {
    let said = |lines: &[String]| lines.join("\n\n").trim().to_string();
    let report = if driven.last_words.is_empty() {
        said(&driven.replies)
    } else {
        said(&driven.last_words)
    };
    if driven.cancelled || driven.stop_reason.as_deref() == Some("aborted") {
        return RunOutcome {
            end: RunEnd::Cancelled,
            report: if report.is_empty() {
                "The subagent was stopped before it reported.".to_string()
            } else {
                format!("The subagent was stopped. What it had said:\n\n{report}")
            },
        };
    }
    if let Some(failure) = driven.failure {
        return RunOutcome {
            end: RunEnd::Failed,
            report: if report.is_empty() {
                format!("The subagent failed: {failure}")
            } else {
                format!("The subagent failed: {failure}\n\nWhat it had said:\n\n{report}")
            },
        };
    }
    RunOutcome {
        end: RunEnd::Done,
        report: if report.is_empty() {
            "The subagent finished without writing a report.".to_string()
        } else {
            report
        },
    }
}

/// The teammate as a run is started for it: on the model and effort its
/// session is using right now, which its record may not say.
fn on_the_sessions_model(mut persona: Persona, session: &SessionInfo) -> Persona {
    if let Some(model) = session.current_model_id.clone() {
        persona.model_id = Some(model);
    }
    if let Some(effort) = session
        .configs
        .iter()
        .find(|config| config.id == "effort")
        .and_then(|config| config.current_id.clone())
    {
        persona.effort_id = Some(effort);
    }
    persona
}

/// A subagent's whole system prompt. It is not the teammate's: no name to
/// answer to, no goal, no house style for chat — a worker's brief, plus the
/// facts of the place it works in.
pub(super) fn run_preamble(persona: &Persona, reach: Option<Reach>) -> String {
    let name = &persona.name;
    format!(
        "You are a subagent working for {name}, a teammate in Hotline. {name} handed you one task, and your last message is returned to them as your report. You are not {name}, and you are not talking with the person {name} works for: nobody reads this conversation while you work, and you cannot ask anyone a question. Where something is unclear, make the sensible choice, say which choice you made, and carry on.\n\n\
         Your working directory is {}.{}\n\n\
         {CLOCK}\n\n\
         `search_thread` finds earlier messages in {name}'s conversation with the person, and `list_chapters` lists its chapters, for when the task depends on something said there.\n\n\
         {}\n\n\
         Work until the task is done, or until you are sure it cannot be. Then finish with one message, your report: lead with the outcome or the answer; then what you did and where, naming files you changed and commands you ran; then anything unresolved, uncertain, or left for {name} to decide. If something failed, say so plainly. Do not narrate while you work: the report is the only thing {name} reads.",
        persona.cwd,
        reach_sentence(reach),
        skills_index(persona),
    )
}

/// The task as the subagent hears it.
fn brief(teammate: &str, task: &str) -> String {
    format!("{teammate} handed you this task:\n\n{task}")
}

/// A run in progress: what stops its driver and settles its marker however
/// the run ends, including when the task that owned it is dropped mid-run.
struct Running {
    room: Weak<Room>,
    persona_id: String,
    run_id: String,
    title: String,
    started: i64,
    driver: Option<Arc<dyn Driver>>,
    lease: Option<CapabilityLease>,
    settled: bool,
}

impl Running {
    /// The run's link, on the teammate's tape and at the head of the run's
    /// own stream, so the run says what it is and how it went to whoever
    /// opens it without reading the tape.
    fn mark(&self, state: ThreadState, elapsed_ms: Option<i64>) {
        let Some(room) = self.room.upgrade() else {
            return;
        };
        let thread = ThreadId::run(&self.run_id);
        room.write_link(&Link {
            id: room.link_id(&thread),
            ts: self.started,
            thread,
            persona_id: Some(self.persona_id.clone()),
            title: self.title.clone(),
            state,
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms,
            opener: None,
        });
        room.list_subagent(
            &self.persona_id,
            RunningSubagent {
                run_id: self.run_id.clone(),
                title: self.title.clone(),
                started_at: self.started,
            },
            state == ThreadState::Live,
        );
    }

    /// Stops what is left of the run and writes how it ended.
    fn settle(&mut self, end: RunEnd) {
        if self.settled {
            return;
        }
        self.settled = true;
        if let Some(driver) = self.driver.take() {
            driver.invalidate();
        }
        if let Some(lease) = &self.lease {
            lease.revoke();
        }
        let end = match end {
            RunEnd::Done => End::Done,
            RunEnd::Failed => End::Failed,
            RunEnd::Cancelled => End::Cancelled,
        };
        self.mark(ThreadState::Closed(end), Some(now_ms() - self.started));
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.settle(RunEnd::Cancelled);
    }
}

/// A subagent a harness is running, as its run is being written.
struct HarnessRun {
    running: Running,
    voice: narration::Voice,
    in_flight: HashMap<String, PendingTool>,
}

/// A teammate's subagents, as its managed jobs start them.
pub(crate) struct Subagents {
    pub(crate) room: Weak<Room>,
    pub(crate) persona_id: String,
    pub(crate) capability: Option<CapabilityLease>,
}

impl Delegate for Subagents {
    fn run(
        &self,
        run_id: String,
        task: SubagentTask,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Finished> {
        let room = self.room.clone();
        let persona_id = self.persona_id.clone();
        let capability = self.capability.clone();
        Box::pin(async move {
            let Some(room) = room.upgrade() else {
                return Finished {
                    state: JobState::Failed,
                    output: "This room has closed.".to_string(),
                };
            };
            let capability = capability.unwrap_or_else(|| room.capability_lease(&persona_id));
            let outcome = room
                .run(
                    RunSpec {
                        persona_id,
                        run_id,
                        title: task.title,
                        task: task.task,
                        capability,
                    },
                    cancel,
                )
                .await;
            Finished {
                state: match outcome.end {
                    RunEnd::Done => JobState::Succeeded,
                    RunEnd::Failed => JobState::Failed,
                    RunEnd::Cancelled => JobState::Cancelled,
                },
                output: outcome.report,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::SessionConfig;
    use crate::session::idle_info;
    use crate::session::tests::persona;

    fn said(replies: &[&str], last_words: &[&str]) -> Driven {
        Driven {
            replies: replies.iter().map(|line| line.to_string()).collect(),
            last_words: last_words.iter().map(|line| line.to_string()).collect(),
            ..Driven::default()
        }
    }

    #[test]
    fn a_run_takes_the_effort_its_teammate_is_on() {
        let mut ada = persona("ada");
        ada.effort_id = Some("low".to_string());
        let mut session = idle_info("ada");
        session.configs = vec![SessionConfig {
            id: "effort".to_string(),
            name: "Effort".to_string(),
            category: None,
            current_id: Some("high".to_string()),
            options: Vec::new(),
        }];
        let view = on_the_sessions_model(ada, &session);
        assert_eq!(view.effort_id.as_deref(), Some("high"));
        assert_eq!(view.model_id, None, "no live model, so the record's stands");
    }

    #[test]
    fn the_report_is_what_the_run_said_once_its_work_was_done() {
        let done = outcome_of(said(
            &["Build started.", "Two tests fail."],
            &["Two tests fail."],
        ));
        assert_eq!(done.end, RunEnd::Done);
        assert_eq!(done.report, "Two tests fail.");

        // A run whose last act was a tool still reports what it said.
        let quiet_end = outcome_of(said(&["Found it in crane.rs."], &[]));
        assert_eq!(quiet_end.report, "Found it in crane.rs.");

        let silent = outcome_of(Driven::default());
        assert_eq!(silent.end, RunEnd::Done);
        assert_eq!(
            silent.report,
            "The subagent finished without writing a report."
        );
    }

    #[test]
    fn a_failed_or_stopped_run_says_so_and_keeps_what_it_had_said() {
        let failed = outcome_of(Driven {
            failure: Some("the provider refused the key".to_string()),
            ..said(&["Half done."], &["Half done."])
        });
        assert_eq!(failed.end, RunEnd::Failed);
        assert!(
            failed
                .report
                .starts_with("The subagent failed: the provider refused the key")
        );
        assert!(failed.report.ends_with("Half done."));

        let aborted = outcome_of(Driven {
            stop_reason: Some("aborted".to_string()),
            ..Driven::default()
        });
        assert_eq!(aborted.end, RunEnd::Cancelled);
        assert_eq!(
            aborted.report,
            "The subagent was stopped before it reported."
        );
    }
}
