//! The one way an agent is built for a thread.
//!
//! A work thread and a subagent run are answered by a fresh agent of their
//! teammate, and what differs between them is the kind's [`Policy`], not the
//! code that builds it. [`Room::thread_agent`] reads the row: it makes the
//! teammate's folder, writes `AGENTS.md` for a child, takes the computer away
//! or grants it (through a lease the teammate's threads share), serves the
//! kind's tools under the kind's lease, and tells the agent what the kind says
//! it should know of the conversation it joins. Then it starts the agent.
//!
//! **Resume** is one mechanism here. A thread whose record holds a session id
//! the teammate's harness issued asks the child to reopen it. If the child
//! will not, or the thread never saved one, or the agent is Hotline Agent
//! (whose context is only what it is handed), the agent is seeded from the
//! thread's own stream: its history for Hotline Agent, a fenced transcript in
//! the preamble for a child. The teammate's own session is never reopened, so
//! a thread's agent can never answer inside the main conversation.
//!
//! The main conversation is built here too, as the DM thread (`start_now` is
//! the room's part of a start: the gate, the roster, the session it publishes).
//! Its row says what is its own: the teammate's checkpoint is the session it
//! resumes, its seed is the open chapter, its folder is written here, its
//! computer does not hold the start up for a download, and the watches a driver
//! offers are taken before it starts. A pair uses the same builder, with its
//! caller as opener and its history oriented to the target answering it.

use super::computer::{
    COMPUTER_DOWNLOADING, ComputerAtStart, Driving, computer_failed_note, computer_unavailable,
    with_note, without_computer,
};
use super::prompt::said;
use super::{Room, chapters, now_ms};
use crate::contract::{
    Ability, McpPolicy, Persona, PolicyMode, Reach, SessionCheckpoint, SessionInfo, SharedSecret,
};
use crate::contract::{ThreadId, ThreadKind};
use crate::driver::{
    CapabilityEpoch, CapabilityLease, Driver, DriverInfo, HOTLINE_BACKEND_ID, SubagentReport,
    Update, acp,
};
use crate::log::StreamId;
use crate::mcp::server::TeammateTools;
use crate::thread::{AgentBinding, Computer, Lease, Opener, Policy, Resume, ThreadStore, Tools};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

/// What a thread's agent is built for.
pub(super) struct Opening {
    pub thread: ThreadId,
    /// The teammate as this thread sees it. The builder takes the rest off it:
    /// the teammate's own session, and the computer when the kind has none.
    pub persona: Persona,
    /// What the thread is called, which is what another thread is told it is
    /// busy with when it asks for the computer this one has.
    pub title: String,
    /// The teammate that handed the work over, when one did: the agent is told
    /// who asked, and what they are for.
    pub opener: Option<Opener>,
    /// The thread's authority, made by [`lease_of`] before the agent is.
    pub lease: CapabilityLease,
    /// When the lines this agent is yet to be handed begin. A thread is
    /// published before its agent is built, so what the person said in it
    /// meanwhile is already on its stream; it is the agent's first turn and
    /// not part of the history the agent is seeded with, or it would hear it
    /// twice.
    pub heard_from: Option<i64>,
}

/// An agent that was built and started for a thread. It is stopped when this
/// is dropped, unless the thread was published with it: [`ThreadAgent::keep`].
pub(super) struct ThreadAgent {
    pub driver: Arc<dyn Driver>,
    /// The teammate as the agent was started for it.
    pub view: Persona,
    /// The agent's own id for the conversation, as the driver reports it.
    pub reported: Option<String>,
    /// All the driver said of itself when it started: its picker, its
    /// capabilities, whether it restored the context it was given.
    pub started: DriverInfo,
    /// Whether the agent was handed the teammate's computer.
    pub computer: bool,
    /// What the agent holds the teammate's computer under, which is its own
    /// and no replacement's.
    pub holder: crate::computer::gate::Holder,
    /// What a driver offers to watch, taken before it started. Only the DM's.
    pub watches: Option<Watches>,
    /// The saved session id the agent reopened, when it did. Nothing else
    /// about the thread's own memory is promised by the harness.
    pub resumed: Option<String>,
    armed: bool,
}

/// The streams a driver publishes while it lives, subscribed to before it
/// starts: ACP may publish a picker change between its handshake and the moment
/// the session enters the room, and the receiver keeps it.
pub(super) struct Watches {
    pub info: Option<watch::Receiver<DriverInfo>>,
    pub unprompted: Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>>,
    pub subagents: Option<mpsc::UnboundedReceiver<SubagentReport>>,
    pub wakes: Option<mpsc::UnboundedReceiver<String>>,
}

impl ThreadAgent {
    /// The thread has taken the agent over, and stops it itself from now on.
    pub fn keep(&mut self) {
        self.armed = false;
    }
}

impl Drop for ThreadAgent {
    fn drop(&mut self) {
        if self.armed {
            self.driver.invalidate();
        }
    }
}

/// Stops a driver that was started for a thread that never went live, which
/// includes a start that is dropped halfway.
struct Starting(Option<Arc<dyn Driver>>);

impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(driver) = self.0.take() {
            driver.invalidate();
        }
    }
}

/// A thread's authority, derived from its parent's as its kind says. A kind
/// can only narrow: even an independent lease is made only after the parent's
/// has been checked by whoever asks.
pub(super) fn lease_of(kind: ThreadKind, parent: &CapabilityLease) -> CapabilityLease {
    match Policy::of(kind).lease {
        Lease::Same => parent.clone(),
        Lease::Scoped => parent.scoped(),
        Lease::Independent => CapabilityEpoch::default().lease(),
    }
}

/// What a thread holds the teammate's computer under, and what another thread
/// is told it is busy with. The DM's key is `dm`, the one the turn loop lets go
/// of when the turn ends.
fn driving_of(thread: &ThreadId, title: &str, lease: &CapabilityLease) -> Driving {
    match thread.kind {
        ThreadKind::Dm => Driving::new(lease_key(thread), "the main conversation", lease),
        ThreadKind::Pair => Driving::new(lease_key(thread), title, lease),
        _ if title.is_empty() => Driving::new(lease_key(thread), "an untitled thread", lease),
        _ => Driving::new(lease_key(thread), format!("the thread \"{title}\""), lease),
    }
}

/// What a thread holds the teammate's computer under (`computer/gate.rs`).
pub(super) fn lease_key(thread: &ThreadId) -> String {
    match thread.kind {
        ThreadKind::Dm => "dm".to_string(),
        kind => format!("{}:{}", kind.name(), thread.key),
    }
}

/// The teammate as a thread's agent is started for it: with no session of the
/// teammate's own to reopen, which would answer inside the main conversation,
/// and with the computer only if the kind has one.
fn own_view(mut persona: Persona, policy: Policy) -> Persona {
    if policy.resume != Resume::Checkpoint {
        persona.session_checkpoints = Vec::new();
        persona.last_session_id = None;
    }
    if policy.has_computer() {
        persona
    } else {
        without_computer(persona)
    }
}

/// The teammate as a clone of its home agent is started for it, a subagent or
/// a job (ADR 0008 §0): on the model and effort the home session is using
/// right now, which its record may not say, and with its whole permission set
/// but what the home agent left out for the task. Everything here can only
/// take away: the view is the record, narrowed, and the lease it runs under
/// is the home thread's.
pub(super) fn clone_of(mut persona: Persona, home: &SessionInfo, without: &[Ability]) -> Persona {
    if let Some(model) = home.current_model_id.clone() {
        persona.model_id = Some(model);
    }
    if let Some(effort) = home
        .configs
        .iter()
        .find(|config| config.id == "effort")
        .and_then(|config| config.current_id.clone())
    {
        persona.effort_id = Some(effort);
    }
    for ability in without {
        match ability {
            Ability::Computer => persona = without_computer(persona),
            Ability::Folders => {
                persona.folders = None;
                if persona.reach.is_some() {
                    persona.reach = Some(Reach::Workspace);
                }
            }
            Ability::McpServers => {
                persona.mcp_policy = McpPolicy {
                    mode: PolicyMode::None,
                    server_ids: Vec::new(),
                };
            }
        }
    }
    persona
}

/// The session a child is asked to reopen: the thread's saved one, when the
/// harness the teammate runs on now, under the account it runs on now, is the
/// one that issued it. Only a child has a session of its own to reopen.
fn resumable(
    resume: Resume,
    binding: Option<AgentBinding>,
    backend_id: &str,
    account_id: Option<&str>,
    in_process: bool,
) -> Option<String> {
    binding
        .filter(|_| resume == Resume::Binding)
        .filter(|binding| {
            !in_process
                && binding.backend_id == backend_id
                && binding.account_id.as_deref() == account_id
        })
        .map(|binding| binding.session_id)
}

impl Room {
    /// A thread's agent, built from its kind's policy and started. See the
    /// module.
    ///
    /// The thread's own stream is what a restarted agent is seeded from, so it
    /// is read here, once, whatever the kind.
    pub(super) async fn thread_agent(
        self: &Arc<Self>,
        opening: Opening,
    ) -> Result<ThreadAgent, String> {
        let Opening {
            thread,
            persona,
            title,
            opener,
            lease,
            heard_from,
        } = opening;
        let persona = self.on_running_account(persona);
        let policy = Policy::of(thread.kind);
        let in_process = persona.backend_id == HOTLINE_BACKEND_ID;
        let main = policy.resume == Resume::Checkpoint;
        std::fs::create_dir_all(&persona.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                persona.name, persona.cwd
            )
        })?;
        // The skills the teammate may read are files in its workspace, for
        // either driver: the built-ins and whatever it is granted of the
        // offered ones, copied under Hotline's marker so a revoked grant leaves
        // nothing of Hotline's behind and nothing of the teammate's is ever
        // touched. The DM writes them; its other threads share the folder.
        if main {
            crate::skills::materialize(
                Path::new(&persona.cwd),
                &crate::skills::Offering::from_settings(
                    self.log.root(),
                    &crate::room::settings(&self.log),
                ),
                &persona.skill_policy,
            )
            .map_err(|error| format!("{}'s skills could not be written: {error}", persona.name))?;
            lease.check()?;
        }
        let mut view = own_view(persona, policy);
        // An ACP session takes no system prompt, so who it is has to be on
        // disk before the child is started.
        if !in_process {
            acp::materialize_agents_md_with_capability(&view, Some(lease.clone())).map_err(
                |error| format!("{}'s AGENTS.md could not be written: {error}", view.name),
            )?;
        }
        // The computer is the teammate's, shared through a lease. A thread that
        // cannot be given it goes on without, and the DM, which is told so,
        // does not wait for a download.
        let driving = driving_of(&thread, &title, &lease);
        let mut computer_note = None;
        let extra_mcp = match policy.computer {
            Computer::No => Vec::new(),
            Computer::Lease => match self.grant_computer(&view, &driving).await {
                Ok(servers) => servers,
                Err(reason) => {
                    eprintln!("{}'s computer is not in this thread: {reason}", view.name);
                    view = without_computer(view);
                    Vec::new()
                }
            },
            // Wake the computer before the grant. The grant itself is appended
            // regardless of mcpPolicy. The computer never keeps the teammate
            // from answering: an image still downloading, or a computer that
            // cannot come up at all (Docker not running, an image that will
            // not pull), starts the teammate without it. The agent is told
            // which, and the tape says so too: never a silent absence.
            Computer::Download => match self.computer_at_start(&view, &driving).await {
                ComputerAtStart::NotWanted => Vec::new(),
                ComputerAtStart::Attached(extra_mcp) => extra_mcp,
                ComputerAtStart::Downloading => {
                    view = without_computer(view);
                    computer_note = Some(COMPUTER_DOWNLOADING.to_string());
                    Vec::new()
                }
                ComputerAtStart::Unavailable(reason) => {
                    lease.check()?;
                    eprintln!("{}", computer_unavailable(&view.name, &reason));
                    computer_note = Some(computer_failed_note(&reason));
                    view = without_computer(view);
                    Vec::new()
                }
            },
        };
        let has_computer = !extra_mcp.is_empty();
        lease.check()?;

        // Reach is Hotline Agent's one policy, and only Hotline Agent's: a
        // child brings its own tools and Hotline enforces nothing over them, so
        // telling one that a path outside its directory would be refused is a
        // promise nobody here can keep.
        let reach = in_process.then(|| view.reach.unwrap_or_default());
        // The parent's tape can be many megabytes. It is read and parsed off the
        // async workers, so a thread starting beside a long conversation does
        // not hold up the others the room is running.
        let context = if policy.seed.parent_tail {
            let room = self.clone();
            let persona_id = view.id.clone();
            tokio::task::spawn_blocking(move || {
                chapters::side_context(&room.tape(&persona_id), now_ms())
            })
            .await
            .map_err(|error| format!("The conversation could not be read: {error}"))?
        } else {
            None
        };
        // The DM's context is one chapter: it hears what was said in the
        // chapter it is joining, and the wake block tells it about the one that
        // closed before it, which is the whole of what a fresh context knows of
        // a conversation that has been going on for months.
        let tape = policy.seed.chapters.then(|| self.tape(&view.id));
        let note = tape.as_ref().and_then(|events| {
            with_note(computer_note.take(), chapters::wake_block(events, now_ms()))
        });
        let earlier = match StreamId::of_thread(&thread) {
            Some(stream) if policy.seed.own_history => {
                let mut earlier = self.log.load(&stream);
                if let Some(from) = heard_from {
                    earlier.retain(|event| event["ts"].as_i64().is_none_or(|ts| ts < from));
                }
                earlier
            }
            _ => Vec::new(),
        };
        let has_history = !earlier.is_empty();
        let pair = thread.kind == ThreadKind::Pair;
        let flip = pair
            && crate::paths::thread_participants(&thread.key)
                .is_some_and(|(_, agent_side)| agent_side != view.id);
        let history = match &tape {
            Some(events) => said(events),
            None if pair => super::peers::said_in(&earlier, flip),
            None if in_process => said(&earlier),
            None => Vec::new(),
        };
        let transcript = || chapters::serialize_chapter(&earlier);
        let account_id = crate::accounts::account_of(&self.log, &view);
        let reopening = resumable(
            policy.resume,
            ThreadStore::new(&self.log)
                .load(&thread)
                .and_then(|record| record.binding),
            &view.backend_id,
            account_id.as_deref(),
            in_process,
        );

        let opened_by = opener.and_then(|opener| self.participant(&opener.persona_id).ok());
        if pair && opened_by.is_none() {
            return Err("The teammate asking in this pair is no longer available.".to_string());
        }
        let stored = self.stored_secrets();
        let build = |view: &Persona, transcript: Option<String>| {
            let mut preamble = preamble_of(
                thread.kind,
                view,
                reach,
                &stored,
                Joining {
                    opener: opened_by.as_ref(),
                    context: context.clone(),
                    transcript,
                    note: note.clone(),
                },
            );
            if self.lead_call(&view.id, &thread) {
                preamble.push_str("\nYou are answering the desk as team lead. Answer roster questions with list_teammates. For work naming a teammate, use message_teammate, intent handoff, with the person's words unchanged and say who has it. Do not stop or reassign another teammate's work.");
            }
            self.agents.agent(
                view,
                preamble,
                history.clone(),
                tools_of(self, &thread, &view.id, &lease),
                extra_mcp.clone(),
            )
        };
        let mut checkpointed = view.clone();
        if let Some(session_id) = &reopening {
            checkpointed.session_checkpoints = vec![SessionCheckpoint {
                backend_id: view.backend_id.clone(),
                session_id: session_id.clone(),
                account_id,
            }];
        }
        let mut driver = build(
            &checkpointed,
            (has_history && reopening.is_none()).then(transcript),
        )?;
        // Subscribe before startup: the receiver keeps what the driver
        // publishes between its handshake and the moment this joins the room.
        let watches = main.then(|| Watches {
            info: driver.subscribe_info(),
            unprompted: driver.subscribe_unprompted(),
            subagents: driver.subscribe_subagents(),
            wakes: driver.subscribe_wakes(),
        });
        let mut starting = Starting(Some(driver.clone()));
        let mut info = driver.start(&checkpointed).await?;
        lease.check()?;
        // Hotline Agent with no provider comes up for the main conversation,
        // to say so on its tape before anything is sent. A side or peer
        // thread would only open onto that, so it is refused instead.
        if in_process && !main && driver.startup_failure().is_some() {
            return Err(crate::driver::failure::NO_PROVIDER.to_string());
        }
        // A harness that came up failed (it never answered, or is signed out)
        // was not asked to reopen anything, and starting it over would only
        // wait out the same failure a second time.
        if reopening.is_some() && !info.context_restored && driver.startup_failure().is_none() {
            // The harness would not reopen it. The thread is not lost: the
            // agent is started over from what the thread said.
            starting.0 = None;
            driver.invalidate();
            driver = build(&view, Some(transcript()))?;
            starting.0 = Some(driver.clone());
            info = driver.start(&view).await?;
            lease.check()?;
        }
        starting.0 = None;
        let resumed = reopening.filter(|_| info.context_restored);
        Ok(ThreadAgent {
            driver,
            view,
            reported: info.session_id.clone(),
            started: info,
            computer: has_computer,
            holder: driving.holder().clone(),
            watches,
            resumed,
            armed: true,
        })
    }
}

/// What a fresh agent is told of the conversation it joins, over and above who
/// it is.
struct Joining<'a> {
    /// The teammate that handed the work over, when one did.
    opener: Option<&'a Persona>,
    /// The parent's chapter note and tail.
    context: Option<String>,
    /// The thread's own earlier lines, for a child that could not reopen it.
    transcript: Option<String>,
    /// The DM's: how the chapter before it closed, and what became of its
    /// computer.
    note: Option<String>,
}

/// What a kind tells its agent about itself, and about what it joins.
fn preamble_of(
    kind: ThreadKind,
    persona: &Persona,
    reach: Option<Reach>,
    stored: &[SharedSecret],
    joining: Joining<'_>,
) -> String {
    let Joining {
        opener,
        context,
        transcript,
        note,
    } = joining;
    match kind {
        ThreadKind::Run => super::runner::run_preamble(persona, reach, stored),
        ThreadKind::Job => super::job_thread::job_preamble(persona, reach, stored),
        // The DM's: what it needs to know of the chapter it joins, and of its
        // computer, is the note.
        ThreadKind::Dm => super::preamble(persona, reach, note, stored),
        ThreadKind::Pair => super::peers::peer_preamble(
            opener.expect("a pair's caller was resolved before building its agent"),
            persona,
            reach,
            stored,
        ),
        _ => super::sides::work_preamble(persona, reach, stored, opener, context, transcript),
    }
}

/// The tools a kind's agent is served, under the thread's lease.
fn tools_of(
    room: &Arc<Room>,
    thread: &ThreadId,
    persona_id: &str,
    lease: &CapabilityLease,
) -> TeammateTools {
    let tools = TeammateTools::new(room, persona_id).with_capability(lease.clone());
    match Policy::of(thread.kind).tools {
        Tools::Teammate => tools.with_subagents(),
        Tools::Work if thread.kind == ThreadKind::Call => tools.for_call(thread.clone()),
        Tools::Work => tools.for_work(thread.key.clone()),
        Tools::Run => tools.for_run(),
        Tools::Job => tools.for_job(thread.key.clone()),
        Tools::Peer => tools.for_peer(thread.clone()),
        Tools::None => tools,
    }
}

#[cfg(test)]
mod tests;
