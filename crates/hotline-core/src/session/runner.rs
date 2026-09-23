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
//! A run never writes to its teammate's tape. The one line it keeps there is
//! a [`TranscriptEvent::Subagent`] marker, rewritten by id as the run goes,
//! which is what the person presses to open the run's own transcript.
//!
//! Authority follows [`crate::driver::CapabilityLease`]: a run's lease is a
//! child of the session that started it, so stopping that teammate, changing
//! its policy or deleting it revokes every run it has going, and a run can
//! never hold more than its parent did.

use super::{Room, event_of, narration, new_id, now_ms, reach_sentence, skills_index};
use crate::contract::{
    NoticeLevel, Persona, Reach, SessionInfo, SubagentStatus, ToolStatus, TranscriptEvent,
};
use crate::driver::{CapabilityLease, Driver, HOTLINE_BACKEND_ID, Update};
use crate::log::StreamId;
use crate::mcp::server::TeammateTools;
use crate::session::jobs::{Delegate, Finished, JobState, SubagentTask};
use chrono::Local;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
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
    mut write: impl FnMut(TranscriptEvent, bool),
) -> Driven {
    let mut updates = driver.prompt(text, Vec::new(), reach).await;
    let mut in_flight = HashMap::new();
    let mut voice = narration::Voice::new();
    let mut driven = Driven::default();
    loop {
        let received = match cancel {
            Some(cancel) if !driven.cancelled => tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    driven.cancelled = true;
                    driver.cancel();
                    continue;
                }
                update = updates.recv() => update,
            },
            _ => updates.recv().await,
        };
        let (batch, done) = match received {
            Some(update) => (voice.step(update), false),
            None => (voice.finish(), true),
        };
        for update in batch {
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
                write(event, asked);
            }
        }
        if done {
            break;
        }
    }
    // A driver that stopped without a turn leaves a tool spinning in the
    // stream forever, exactly as it would on a tape.
    for (call_id, pending) in in_flight.drain() {
        write(pending.event(&call_id, ToolStatus::Failed, None), false);
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
        running.mark(SubagentStatus::Running, None);
        self.append_run(
            &spec.run_id,
            TranscriptEvent::User {
                id: new_id(),
                ts: running.started,
                text: spec.task.clone(),
                attachments: None,
                reactions: None,
                reply_to: None,
                scheduled: None,
                ring: None,
                receipt: None,
            },
        );
        let outcome = match self.run_to_end(&spec, &cancel, &mut running).await {
            Ok(outcome) => outcome,
            Err(error) => {
                self.append_run(
                    &spec.run_id,
                    TranscriptEvent::Notice {
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
        let lease = spec.capability.scoped();
        running.lease = Some(lease.clone());
        let view = run_view(persona, &self.info(&spec.persona_id));
        let reach = view.reach.unwrap_or_default();
        let driver = self.agents.agent(
            &view,
            run_preamble(&view, reach),
            Vec::new(),
            TeammateTools::new(self, &view.id)
                .for_run()
                .with_capability(lease.clone()),
            Vec::new(),
        )?;
        running.driver = Some(driver.clone());
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Ok(RunOutcome {
                    end: RunEnd::Cancelled,
                    report: "The subagent was stopped before it started.".to_string(),
                });
            }
            started = driver.start(&view) => { started?; }
        }
        lease.check()?;
        let driven = drive(
            driver.as_ref(),
            brief(&view.name, &spec.task),
            reach,
            Some(cancel),
            |event, _| self.append_run(&spec.run_id, event),
        )
        .await;
        Ok(outcome_of(driven))
    }

    /// One event onto a run's own stream. Nothing indexes it and nothing
    /// stamps it: a run's words are the teammate's working, not its
    /// conversation.
    fn append_run(&self, run_id: &str, event: TranscriptEvent) {
        let written = serde_json::to_value(&event)
            .map_err(|error| error.to_string())
            .and_then(|event| {
                self.log
                    .append(&StreamId::Run(run_id.to_string()), &event)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = written {
            eprintln!("the run {run_id} could not be written to: {error}");
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

/// The teammate as a run sees it: its record, on the model and effort its
/// session is using right now, with no conversation to reopen and no
/// computer. Two agents driving one desktop at once is a fight nobody wins,
/// so the desktop stays with the teammate.
fn run_view(mut persona: Persona, session: &SessionInfo) -> Persona {
    persona.session_checkpoints = Vec::new();
    persona.last_session_id = None;
    if let Some(computer) = persona.computer.as_mut() {
        computer.enabled = false;
    }
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
fn run_preamble(persona: &Persona, reach: Reach) -> String {
    let name = &persona.name;
    format!(
        "You are a subagent working for {name}, a teammate in Hotline. {name} handed you one task, and your last message is returned to them as your report. You are not {name}, and you are not talking with the person {name} works for: nobody reads this conversation while you work, and you cannot ask anyone a question. Where something is unclear, make the sensible choice, say which choice you made, and carry on.\n\n\
         Your working directory is {}.{}\n\n\
         Today is {}.\n\n\
         `search_thread` finds earlier messages in {name}'s conversation with the person, and `list_chapters` lists its chapters, for when the task depends on something said there.\n\n\
         {}\n\n\
         Work until the task is done, or until you are sure it cannot be. Then finish with one message, your report: lead with the outcome or the answer; then what you did and where, naming files you changed and commands you ran; then anything unresolved, uncertain, or left for {name} to decide. If something failed, say so plainly. Do not narrate while you work: the report is the only thing {name} reads.",
        persona.cwd,
        reach_sentence(Some(reach)),
        Local::now().format("%A %-d %B %Y"),
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
    /// The run's line, on the teammate's tape and at the head of the run's
    /// own stream, so the run says what it is and how it went to whoever
    /// opens it without reading the tape.
    fn mark(&self, status: SubagentStatus, elapsed_ms: Option<i64>) {
        let Some(room) = self.room.upgrade() else {
            return;
        };
        let marker = TranscriptEvent::Subagent {
            id: marker_id(&self.run_id),
            ts: self.started,
            run_id: self.run_id.clone(),
            title: self.title.clone(),
            status,
            elapsed_ms,
        };
        room.write(&self.persona_id, &marker);
        room.append_run(&self.run_id, marker);
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
        let status = match end {
            RunEnd::Done => SubagentStatus::Done,
            RunEnd::Failed => SubagentStatus::Failed,
            RunEnd::Cancelled => SubagentStatus::Cancelled,
        };
        self.mark(status, Some(now_ms() - self.started));
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.settle(RunEnd::Cancelled);
    }
}

fn marker_id(run_id: &str) -> String {
    format!("subagent:{run_id}")
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

/// Subagent lines a previous process left running on a tape. A run lives
/// only as long as the activity that started it, and no activity survives a
/// restart, so the line is settled as cancelled rather than drawn running
/// forever — on the tape, and by the caller on the run's own stream.
pub(crate) fn settle_orphaned_subagents(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|event| {
            event.get("kind").and_then(Value::as_str) == Some("subagent")
                && event.get("status").and_then(Value::as_str) == Some("running")
        })
        .filter_map(|event| {
            let mut settled = event.as_object()?.clone();
            settled.insert("status".into(), Value::from("cancelled"));
            Some(Value::Object(settled))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{PersonaComputer, SessionConfig};
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
    fn a_run_takes_the_effort_its_teammate_is_on_and_leaves_the_computer_behind() {
        let mut ada = persona("ada");
        ada.effort_id = Some("low".to_string());
        ada.computer = Some(PersonaComputer {
            enabled: true,
            image: None,
            memory: None,
            pids: None,
            mounts: None,
            secrets: None,
        });
        let mut session = idle_info("ada");
        session.configs = vec![SessionConfig {
            id: "effort".to_string(),
            name: "Effort".to_string(),
            category: None,
            current_id: Some("high".to_string()),
            options: Vec::new(),
        }];
        let view = run_view(ada, &session);
        assert_eq!(view.effort_id.as_deref(), Some("high"));
        assert_eq!(view.model_id, None, "no live model, so the record's stands");
        assert!(!view.computer.unwrap().enabled);
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
