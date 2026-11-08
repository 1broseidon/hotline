//! Runs: a clone of the teammate's home agent started for one piece of work,
//! driven to its end on a stream of its own, and what it reported handed back.
//!
//! A run is the teammate itself with one focus (ADR 0008 §0): its model as
//! the home session is on it now, its working directory, its tools, computer,
//! folders and integrations, less whatever the home agent left out for the
//! task ([`super::agent::clone_of`]). It starts from its brief, not from the
//! home thread, and reaches back into that with `search_thread` and
//! `list_chapters` when the work needs it. Its system prompt is the
//! teammate's own, with a brief that says it is a run and how to report.
//! [`Room::run`] is the whole of it, on either driver; a Hotline Agent
//! teammate reaches it through its managed jobs (see
//! [`super::jobs::Delegate`]), which is what makes it cancellable, waitable
//! and parallel without scheduling here.
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
//! never hold more than its parent did: what it is handed is read off the
//! teammate's record when it starts, and can only be narrowed.

use super::agent::{Opening, clone_of, lease_of};
use super::prompt::{PendingTool, event_of};
use super::turns::{Line, Seat};
use super::{Brief, Report, Room, Trigger, lock, narration, new_id, now_ms, timed};
use crate::contract::{End, ThreadId, ThreadKind};
use crate::contract::{
    NoticeLevel, Persona, ProcessOutput, Reach, RunKind, RunningSubagent, SharedSecret,
    SubagentStatus, ToolStatus, TranscriptEvent,
};
use crate::driver::{CapabilityLease, Driver, MessageKind, SubagentReport, Update};
use crate::session::jobs::{Delegate, Finished, JobState, SubagentTask};
use crate::thread::{Link, ThreadState};
use crate::voice::spoken::Shown;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
    /// Whether the turn had its one post to the chat.
    pub posted: bool,
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

    /// The words as they arrive (kind, message id, the words, and whether the
    /// reply is being held back as narration), before the message is whole.
    fn delta(&mut self, _kind: MessageKind, _message_id: &str, _words: Words<'_>, _muted: bool) {}

    /// Whether the turn was said on a call to this teammate, so a reply's
    /// spoken version is kept with what is written of it.
    fn said_on_call(&self) -> bool {
        false
    }

    /// Work results and sent-file records can carry facts beyond a short
    /// spoken completion, even when their text has no visual formatting.
    fn keep_written(&self) -> bool {
        false
    }

    /// One event of the turn, and whether it came of a permission request.
    fn write(&mut self, event: TranscriptEvent, asked: bool);

    /// A look at the lines waiting behind the turn, between one update and the
    /// next, for a kind whose person can steer a turn in flight.
    fn steer(&mut self) {}
}

/// One chunk of a message's words as they arrive.
#[derive(Clone, Copy, Debug)]
pub(super) struct Words<'a> {
    /// As the agent wrote them, both versions and their tags: what a call
    /// says its reply from.
    pub written: &'a str,
    /// As the chat shows them: the written version without its tags, and the
    /// start of a tag held back until the next chunk says whether it was.
    /// Empty while the spoken version streams.
    pub shown: &'a str,
}

/// Reads one turn's updates off the driver to its end and tells `witness`
/// what each came to.
///
/// No message is shown or written with the tags a call asks the agent to
/// write around the version to say and the version to show
/// ([`crate::voice::spoken`]), on a call or not: an agent that keeps its own
/// history keeps the call's contract in it and may write them in a typed
/// reply long after. The chat is shown the written version.
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
    // Each message's words as the chat is shown them, by message id, until
    // the message is whole.
    let mut shown: HashMap<String, Shown> = HashMap::new();
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
                let showing = shown.entry(message_id.clone()).or_default().push(text);
                let words = Words {
                    written: text,
                    shown: &showing,
                };
                witness.delta(*kind, message_id, words, muted);
            }
            if let Update::Message { id, .. } = &update {
                shown.remove(id);
            }
            let asked = matches!(update, Update::Permission { .. });
            driven.asked |= asked;
            for event in event_of(
                update,
                &mut in_flight,
                witness.said_on_call(),
                witness.keep_written(),
            ) {
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
    /// What it is to do, now, reporting to whoever asked, with what the home
    /// agent left out of its abilities.
    pub brief: Brief,
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
            kind: RunKind::Subagent,
            title: spec.title.clone(),
            command: None,
            started: now_ms(),
            model: self.info(&spec.persona_id).model_label,
            stop: Some(cancel.clone()),
            driver: None,
            lease: None,
            settled: false,
        };
        running.mark(ThreadState::Live, None);
        self.threads().write(
            &ThreadId::run(&spec.run_id),
            &spec.persona_id,
            &TranscriptEvent::User {
                handoff_thread: None,
                id: new_id(),
                ts: running.started,
                text: spec.brief.task.clone(),
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
        // Work for later, or for the person, is a job's: the scheduler fires
        // it on a thread of its own.
        if spec.brief.trigger != Trigger::Now || spec.brief.report != Report::Asker {
            return Err(
                "A run starts now and reports to whoever asked; anything else is a job."
                    .to_string(),
            );
        }
        let persona = self.persona(&spec.persona_id)?;
        let lease = lease_of(ThreadKind::Run, &spec.capability);
        running.lease = Some(lease.clone());
        let view = clone_of(persona, &self.info(&spec.persona_id), &spec.brief.without);
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
                heard_from: None,
            }) => built?,
        };
        // The run settles its own agent, however it ends.
        agent.keep();
        running.driver = Some(agent.driver.clone());
        let reach = agent.view.reach.unwrap_or_default();
        // A run is told its parent thread's line and the asides about it,
        // and nothing else of the teammate's threads.
        let asides = self.hear_asides(&spec.persona_id, &thread, &[]);
        let task = timed(now_ms(), &handed(&spec.brief.task));
        let task = match self.thread_index(&spec.persona_id, &thread, &asides, None) {
            Some(index) => format!("{index}\n\n{task}"),
            None => task,
        };
        let driven = self
            .threads()
            .turn(
                Seat {
                    capability: None,
                    thread: &thread,
                    persona_id: &spec.persona_id,
                    driver: agent.driver.as_ref(),
                    steer: None,
                },
                Line {
                    said: None,
                    voice: None,
                    text: task,
                    attachments: Vec::new(),
                    request: None,
                    from: None,
                    delivery: None,
                    wake: false,
                    steer: false,
                },
                reach,
                Some(cancel),
                None,
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
            SubagentReport::Started {
                child,
                kind,
                title,
                task,
            } => {
                if runs.contains_key(&child) {
                    return;
                }
                // A harness titles a command it sent to the background with
                // the command itself, in code ticks.
                let command = (kind == RunKind::Shell)
                    .then(|| task.replace('`', "").trim().to_string())
                    .filter(|command| !command.is_empty());
                let running = Running {
                    room: Arc::downgrade(self),
                    persona_id: persona_id.to_string(),
                    run_id: new_id(),
                    kind,
                    title,
                    command,
                    started: now_ms(),
                    model: None,
                    stop: None,
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
                            handoff_thread: None,
                            id: new_id(),
                            ts: running.started,
                            text: task.clone(),
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
                        told: Some(task.trim().to_string()).filter(|task| !task.is_empty()),
                    },
                );
            }
            SubagentReport::Told { child, text } => {
                let Some(run) = runs.get_mut(&child) else {
                    return;
                };
                // Claude's adapter can send a subagent's prompt twice, once
                // under a stand-in id and again under its call's: the same
                // words with nothing between them are one message.
                let words = text.trim().to_string();
                if run.told.as_deref() == Some(words.as_str()) {
                    return;
                }
                run.told = Some(words);
                self.threads().write(
                    &ThreadId::run(&run.running.run_id),
                    persona_id,
                    &TranscriptEvent::User {
                        handoff_thread: None,
                        id: new_id(),
                        ts: now_ms(),
                        text,
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
            SubagentReport::Update { child, update } => {
                let Some(run) = runs.get_mut(&child) else {
                    return;
                };
                for update in run.voice.step(update) {
                    for event in event_of(update, &mut run.in_flight, false, false) {
                        run.told = None;
                        self.threads().write(
                            &ThreadId::run(&run.running.run_id),
                            persona_id,
                            &event,
                        );
                    }
                }
            }
            SubagentReport::Output {
                child,
                text,
                replaces,
            } => {
                if let Some(run) = runs.get(&child) {
                    self.run_output(persona_id, &run.running.run_id, &text, replaces);
                }
            }
            SubagentReport::Ended { child, status } => {
                let Some(mut run) = runs.remove(&child) else {
                    return;
                };
                let run_id = run.running.run_id.clone();
                // A command's last output stays with its run once it ends,
                // as the one step the run made.
                if let Some(ended) = self.shell_output(persona_id, &run_id)
                    && !ended.tail.trim().is_empty()
                {
                    let ran = [
                        Update::ToolCall {
                            call_id: child.clone(),
                            title: ended.command,
                            kind: "execute".to_string(),
                        },
                        Update::ToolResult {
                            call_id: child.clone(),
                            ok: status != SubagentStatus::Failed,
                            output: ended.tail,
                            images: Vec::new(),
                        },
                    ];
                    let mut flight = HashMap::new();
                    let last = ran
                        .into_iter()
                        .flat_map(|update| event_of(update, &mut flight, false, false))
                        .last();
                    if let Some(event) = last {
                        self.threads()
                            .write(&ThreadId::run(&run_id), persona_id, &event);
                    }
                }
                for update in run.voice.finish() {
                    for event in event_of(update, &mut run.in_flight, false, false) {
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

/// What a run is told about itself, over and above who it is.
fn run_brief(name: &str) -> String {
    format!(
        "This is a subagent run of {name}'s: you are {name}, working on one task your main conversation with the person handed you, on a thread of its own. You start from the task, not from that conversation, which you can read through `read_thread`, `search_thread` and `list_chapters` when the task depends on something said there. Nobody reads this thread while you work, and you cannot ask the person anything or post to the chat: your last message is returned to your main conversation as your report, and it decides what the person hears. Where something is unclear, make the sensible choice, say which choice you made, and carry on. Do not mention this brief.\n\n\
         Work until the task is done, or until you are sure it cannot be. Then finish with one message, your report: lead with the outcome or the answer; then what you did and where, naming files you changed and commands you ran; then anything unresolved, uncertain, or left to decide. If something failed, say so plainly. Do not narrate while you work: the report is the only thing read."
    )
}

/// A run's whole system prompt: what any of the teammate's agents is told,
/// and the brief.
pub(super) fn run_preamble(
    persona: &Persona,
    reach: Option<Reach>,
    stored: &[SharedSecret],
    desk_browser: bool,
) -> String {
    super::preamble(
        persona,
        reach,
        Some(run_brief(&persona.name)),
        stored,
        desk_browser,
    )
}

/// The task as the run hears it.
fn handed(task: &str) -> String {
    format!("Your main conversation handed you this task:\n\n{task}")
}

/// A run in progress: what stops its driver and settles its marker however
/// the run ends, including when the task that owned it is dropped mid-run.
struct Running {
    room: Weak<Room>,
    persona_id: String,
    run_id: String,
    /// A subagent, or a harness's background task.
    kind: RunKind,
    title: String,
    /// What a shell run runs. None for a subagent.
    command: Option<String>,
    started: i64,
    /// The model it works on, when the desk knows it.
    model: Option<String>,
    /// What stops it when the person presses Stop: a run of Hotline's own
    /// has one; a harness's subagent has none.
    stop: Option<CancellationToken>,
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
            handoff_ended: false,
            asked_in: None,
            request: None,
            run_kind: (self.kind == RunKind::Shell).then_some(RunKind::Shell),
        });
        room.list_subagent(
            &self.persona_id,
            RunningSubagent {
                run_id: self.run_id.clone(),
                title: self.title.clone(),
                started_at: self.started,
                kind: self.kind,
                last_step: None,
                steps: 0,
                command: self.command.clone(),
                model: self.model.clone(),
                stoppable: self.stop.is_some(),
            },
            state == ThreadState::Live,
            self.stop.clone(),
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

/// A run on its teammate's roster row, and the calls it has made: a step is
/// written again as it runs and as it ends, and is counted once.
pub(crate) struct Listed {
    pub(super) row: RunningSubagent,
    calls: HashSet<String>,
    /// What stops the run, when the person can.
    stop: Option<CancellationToken>,
    /// The latest of what a shell run's command has written, kept only
    /// while it runs: the person reads it through `process.output`, and its
    /// end writes the last of it to the run's stream.
    output: String,
}

impl Listed {
    /// Takes in one event of the run's stream, and whether the row changed.
    fn wrote(&mut self, event: &Value) -> bool {
        let line = match event["kind"].as_str() {
            Some("tool") => {
                if let Some(call_id) = event["toolCallId"].as_str() {
                    self.calls.insert(call_id.to_string());
                }
                event["said"]["one"]
                    .as_str()
                    .or_else(|| event["title"].as_str())
                    .map(|line| line.trim().to_string())
            }
            // Narration between steps is written as a thought (see
            // `narration`), and it is most of what a run says while it works.
            Some("agent" | "thought") => event["text"].as_str().and_then(first_clause),
            _ => None,
        };
        let steps = u32::try_from(self.calls.len()).unwrap_or(u32::MAX);
        let line = line.filter(|line| !line.is_empty());
        let changed = self.row.steps != steps || (line.is_some() && line != self.row.last_step);
        self.row.steps = steps;
        if line.is_some() {
            self.row.last_step = line;
        }
        changed
    }
}

/// The longest a run's latest words are on its roster row, quotes aside.
const CLAUSE_CHARS: usize = 60;

/// The first clause of what a run said, in quotes, as one short line: up to
/// the first line break, or the first stop or comma that ends a word, so a
/// file name or a number keeps its dot or comma.
fn first_clause(text: &str) -> Option<String> {
    let text = text.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, '#' | '*' | '_' | '>' | '-' | '`')
    });
    let mut end = text.len();
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let ends_word = chars.peek().is_none_or(|(_, next)| next.is_whitespace());
        if c == '\n' || (matches!(c, '.' | ',' | ';' | ':' | '!' | '?') && ends_word) {
            end = at;
            break;
        }
    }
    let clause = text[..end].trim().trim_end_matches(['*', '_', '`']);
    if !clause.chars().any(char::is_alphanumeric) {
        return None;
    }
    if clause.chars().count() <= CLAUSE_CHARS {
        return Some(format!("\u{201c}{clause}\u{201d}"));
    }
    let cut: String = clause.chars().take(CLAUSE_CHARS - 1).collect();
    Some(format!("\u{201c}{}\u{2026}\u{201d}", cut.trim_end()))
}

impl Room {
    /// The subagents this teammate has running, oldest first.
    pub fn subagents(&self, persona_id: &str) -> Vec<RunningSubagent> {
        lock(&self.subagents)
            .get(persona_id)
            .map(|runs| runs.iter().map(|listed| listed.row.clone()).collect())
            .unwrap_or_default()
    }

    /// Lists a run on its teammate's roster row while it is running, and
    /// takes it off once it is not, telling the roster either way.
    pub(crate) fn list_subagent(
        &self,
        persona_id: &str,
        run: RunningSubagent,
        running: bool,
        stop: Option<CancellationToken>,
    ) {
        {
            let mut all = lock(&self.subagents);
            let runs = all.entry(persona_id.to_string()).or_default();
            runs.retain(|listed| listed.row.run_id != run.run_id);
            if running {
                runs.push(Listed {
                    row: run,
                    calls: HashSet::new(),
                    stop,
                    output: String::new(),
                });
            }
            if runs.is_empty() {
                all.remove(persona_id);
            }
        }
        let _ = self.info_changes.send(self.info(persona_id));
    }

    /// The person's Stop on a run: its agent is told to stop, and the run
    /// ends cancelled and reports that to its teammate, as a run its
    /// teammate cancelled does. Refused, in a sentence, for a run that is
    /// not going and for a harness's subagent, which nothing here can stop.
    pub fn stop_run(&self, run_id: &str) -> Result<(), String> {
        let stop = lock(&self.subagents)
            .values()
            .flatten()
            .find(|listed| listed.row.run_id == run_id)
            .map(|listed| listed.stop.clone())
            .ok_or_else(|| "That subagent is not running.".to_string())?;
        let stop = stop.ok_or_else(|| {
            "That subagent is its harness's, which has no way to stop one run.".to_string()
        })?;
        stop.cancel();
        Ok(())
    }

    /// Keeps a running run's roster row saying what it did last, as each
    /// event lands on its stream (`Threads::write` calls this for every one),
    /// so the row moves with the run and nothing polls. The roster is told
    /// only when the row changed: a step rewritten as it ends costs nothing.
    pub(super) fn run_wrote(&self, persona_id: &str, run_id: &str, event: &Value) {
        let changed = lock(&self.subagents)
            .get_mut(persona_id)
            .and_then(|runs| runs.iter_mut().find(|listed| listed.row.run_id == run_id))
            .is_some_and(|listed| listed.wrote(event));
        if changed {
            let _ = self.info_changes.send(self.info(persona_id));
        }
    }

    /// Takes in what a shell run's command wrote, keeping its latest
    /// `OUTPUT_TAIL_CHARS`. Nothing is told: the person's pane asks.
    fn run_output(&self, persona_id: &str, run_id: &str, text: &str, replaces: bool) {
        let mut all = lock(&self.subagents);
        let Some(listed) = all
            .get_mut(persona_id)
            .and_then(|runs| runs.iter_mut().find(|listed| listed.row.run_id == run_id))
        else {
            return;
        };
        if replaces {
            listed.output.clear();
        }
        listed.output.push_str(text);
        let extra = listed
            .output
            .chars()
            .count()
            .saturating_sub(OUTPUT_TAIL_CHARS);
        if extra > 0 {
            listed.output = listed.output.chars().skip(extra).collect();
        }
    }

    /// A running shell run's command and the latest of its output, by its
    /// run id: what `process.output` answers for a command the teammate's
    /// harness left running, as it does for one Hotline Agent left.
    pub(super) fn shell_output(&self, persona_id: &str, run_id: &str) -> Option<ProcessOutput> {
        lock(&self.subagents)
            .get(persona_id)?
            .iter()
            .find(|listed| listed.row.run_id == run_id && listed.row.kind == RunKind::Shell)
            .map(|listed| ProcessOutput {
                command: listed
                    .row
                    .command
                    .clone()
                    .unwrap_or_else(|| listed.row.title.clone()),
                tail: listed.output.clone(),
            })
    }
}

/// The most of a shell run's output kept while it runs, from its end.
const OUTPUT_TAIL_CHARS: usize = 16_000;

/// A subagent a harness is running, as its run is being written.
struct HarnessRun {
    running: Running,
    voice: narration::Voice,
    in_flight: HashMap<String, PendingTool>,
    /// The words the run was last told, until it does anything: a second
    /// telling of them before then is the same message said again.
    told: Option<String>,
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
                        brief: task.brief,
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

    fn said(replies: &[&str], last_words: &[&str]) -> Driven {
        Driven {
            replies: replies.iter().map(ToString::to_string).collect(),
            last_words: last_words.iter().map(ToString::to_string).collect(),
            ..Driven::default()
        }
    }

    #[test]
    fn a_runs_words_are_said_by_their_first_clause() {
        let said = |text: &str| first_clause(text);
        assert_eq!(
            said("The right pane opens a run through ThreadView, the same view.").as_deref(),
            Some("\u{201c}The right pane opens a run through ThreadView\u{201d}")
        );
        assert_eq!(
            said("Dock.tsx has 1,000 lines. Next.").as_deref(),
            Some("\u{201c}Dock.tsx has 1,000 lines\u{201d}"),
            "a dot or comma inside a word is not a break"
        );
        assert_eq!(
            said("**Plan**\nRead the dock first.").as_deref(),
            Some("\u{201c}Plan\u{201d}"),
            "a heading's marks are not words"
        );
        let long = "word ".repeat(30);
        let cut = said(&long).unwrap();
        assert_eq!(
            cut.chars().count(),
            CLAUSE_CHARS + 2,
            "the quotes aside: {cut}"
        );
        assert!(cut.ends_with("\u{2026}\u{201d}"), "{cut}");
        assert_eq!(said("  \n  "), None);
        assert_eq!(said("..."), None);
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

    /// An agent asked once on a call for a spoken version and a written one
    /// may write them in any later reply. A run, like every turn, streams and
    /// keeps the written version and never a tag, even split across chunks;
    /// it keeps no spoken version, since nothing was said, and its report is
    /// the reply as shown.
    #[tokio::test]
    async fn a_run_shows_and_keeps_only_the_written_version() {
        use crate::session::tests::Scripted;
        let delta = |text: &str| Update::Delta {
            kind: MessageKind::Agent,
            message_id: "m1".to_string(),
            text: text.to_string(),
        };
        let reply = "<spoken>Two tests fail.</spoken>\n<written>| test | result |</written>";
        let driver = Scripted::new(vec![
            delta("<spoken>Two tests fail.</spo"),
            delta("ken>\n<writ"),
            delta("ten>| test | result |</written>"),
            Update::Message {
                kind: MessageKind::Agent,
                id: "m1".to_string(),
                text: reply.to_string(),
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ]);
        #[derive(Default)]
        struct Recorded {
            streamed: Vec<String>,
            written: Vec<TranscriptEvent>,
        }
        impl Witness for Recorded {
            fn delta(&mut self, _: MessageKind, _: &str, words: Words<'_>, _: bool) {
                if !words.shown.is_empty() {
                    self.streamed.push(words.shown.to_string());
                }
            }
            fn write(&mut self, event: TranscriptEvent, _: bool) {
                self.written.push(event);
            }
        }
        let mut recorded = Recorded::default();
        let updates = driver
            .prompt(
                timed(now_ms(), "Check the tests."),
                Vec::new(),
                Reach::Workspace,
            )
            .await;
        let driven = drive_updates(&driver, updates, None, None, &mut recorded).await;
        let Recorded { streamed, written } = recorded;
        assert_eq!(streamed, ["| test | result |"]);
        let said: Vec<(&str, &Option<String>)> = written
            .iter()
            .filter_map(|event| match event {
                TranscriptEvent::Agent { text, spoken, .. } => Some((text.as_str(), spoken)),
                _ => None,
            })
            .collect();
        assert_eq!(said, [("| test | result |", &None)]);
        assert_eq!(driven.replies, ["| test | result |"]);
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
