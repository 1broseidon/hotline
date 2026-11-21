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
//! offers are taken before it starts. A pair has no agent of its own: a
//! colleague's question is answered in the recipient's own threads.

use super::computer::{
    COMPUTER_DOWNLOADING, ComputerAtStart, Driving, computer_failed_note, computer_unavailable,
    with_note, without_computer,
};
use super::prompt::said;
use super::sides::OpenedBy;
use super::{Room, chapters, now_ms, workspace_root};
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
    /// The person's name as the preamble said it, so a later change is told
    /// once on the next line rather than the name on every line.
    pub told_name: Option<String>,
    armed: bool,
}

/// The streams a driver publishes while it lives, subscribed to before it
/// starts: ACP may publish a picker change between its handshake and the moment
/// the session enters the room, and the receiver keeps it.
pub(super) struct Watches {
    pub info: Option<watch::Receiver<DriverInfo>>,
    pub unprompted: Option<mpsc::UnboundedReceiver<mpsc::Receiver<Update>>>,
    pub subagents: Option<mpsc::UnboundedReceiver<SubagentReport>>,
    pub wakes: Option<mpsc::UnboundedReceiver<crate::session::jobs::ProcessReturn>>,
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
/// harness the teammate runs on now is the one that issued it. Only a child
/// has a session of its own to reopen.
fn resumable(
    resume: Resume,
    binding: Option<AgentBinding>,
    backend_id: &str,
    in_process: bool,
) -> Option<String> {
    binding
        .filter(|_| resume == Resume::Binding)
        .filter(|binding| !in_process && binding.backend_id == backend_id)
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
            mut persona,
            title,
            opener,
            lease,
            heard_from,
        } = opening;
        let policy = Policy::of(thread.kind);
        let in_process = persona.backend_id == HOTLINE_BACKEND_ID;
        let main = policy.resume == Resume::Checkpoint;
        // Before anything is made or written there: the workspace and granted
        // folders must be the directories the record names (H-64), and one
        // not bound yet is bound and recorded first, when no confined
        // teammate could have replaced it.
        self.bind_workspace(&mut persona).await?;
        // The skills the teammate may read are files in its workspace, for
        // either driver: the built-ins and whatever it is granted of the
        // offered ones, copied under Hotline's marker so a revoked grant leaves
        // nothing of Hotline's behind and nothing of the teammate's is ever
        // touched. The DM writes them; its other threads share the folder.
        if main {
            crate::skills::materialize(
                Path::new(&persona.cwd),
                workspace_root::bound_identity(&persona),
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
        // does not wait for a download. It is made with the person's folders
        // alone, never the received one.
        let driving = driving_of(&thread, &title, &lease);
        let mut computer_note = None;
        let for_computer = self.without_received(view.clone());
        let extra_mcp = match policy.computer {
            Computer::No => Vec::new(),
            Computer::Lease => match self.grant_computer(&for_computer, &driving).await {
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
            Computer::Download => match self.computer_at_start(&for_computer, &driving).await {
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
                    // Kept where the pane's Start, and the teammate's own
                    // computer_status, can say why.
                    self.computer_links.failed(&view.id, reason.clone());
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
        let history = match &tape {
            Some(events) => said(events),
            None if in_process => said(&earlier),
            None => Vec::new(),
        };
        let transcript = || chapters::serialize_chapter(&earlier);
        let reopening = resumable(
            policy.resume,
            ThreadStore::new(&self.log)
                .load(&thread)
                .and_then(|record| record.binding),
            &view.backend_id,
            in_process,
        );

        let opener = opener.and_then(|opener| self.participant(&opener.persona_id).ok());
        let opened_by = match &opener {
            Some(opener) => OpenedBy::Handoff(opener),
            None => OpenedBy::Person,
        };
        // What its desk shell is handed is read now, so a harness started
        // below and every command after it get the grants as they stand.
        self.refresh_desk_shell(&view);
        let stored = self.stored_secrets();
        // A job's agent is told only of the tools it is served.
        let offered = tools_of(self, &thread, &view.id, &lease).offered_names();
        let told_name = self.person_name();
        // Whether this desk has a browser to offer, as the session starts.
        let desk_browser = self.desk_browser().is_some();
        let build = |view: &Persona, transcript: Option<String>| {
            // The preamble lists the person's folders; each item names where
            // its files are kept.
            let mut preamble = preamble_of(
                thread.kind,
                &self.without_received(view.clone()),
                reach,
                &stored,
                desk_browser,
                &offered,
                Joining {
                    opened_by,
                    context: context.clone(),
                    transcript,
                    note: note.clone(),
                },
            );
            if let Some(name) = &told_name {
                preamble.push_str("\n\n");
                preamble.push_str(&super::person_name::name_sentence(name));
            }
            if self.lead_call(&view.id, &thread) {
                preamble.push_str("\nYou are answering the desk as team lead. Answer roster questions with list_teammates. For work naming a teammate, use send with wants work, the person's words unchanged as the message and a short title, and say whose inbox it is in. Do not stop or reassign another teammate's work.");
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
        // to say so on its tape before anything is sent. Another thread would
        // only open onto that, so it is refused instead.
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
            told_name,
            armed: true,
        })
    }
}

/// What a fresh agent is told of the conversation it joins, over and above who
/// it is.
struct Joining<'a> {
    /// The colleague the thread was opened for, when one was.
    opened_by: OpenedBy<'a>,
    /// The parent's chapter note and tail.
    context: Option<String>,
    /// The thread's own earlier lines, for a child that could not reopen it.
    transcript: Option<String>,
    /// The DM's: how the chapter before it closed, and what became of its
    /// computer.
    note: Option<String>,
}

/// What a kind tells its agent about itself, and about what it joins.
/// `offered` is the Hotline tools its agent is served.
fn preamble_of(
    kind: ThreadKind,
    persona: &Persona,
    reach: Option<Reach>,
    stored: &[SharedSecret],
    desk_browser: bool,
    offered: &[String],
    joining: Joining<'_>,
) -> String {
    let Joining {
        opened_by,
        context,
        transcript,
        note,
    } = joining;
    match kind {
        ThreadKind::Run => super::runner::run_preamble(persona, reach, stored, desk_browser),
        ThreadKind::Job => {
            super::job_thread::job_preamble(persona, reach, stored, desk_browser, offered)
        }
        // The DM's: what it needs to know of the chapter it joins, and of its
        // computer, is the note.
        ThreadKind::Dm => super::preamble(persona, reach, note, stored, desk_browser),
        _ => super::sides::work_preamble(
            persona,
            reach,
            stored,
            desk_browser,
            opened_by,
            context,
            transcript,
        ),
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
        Tools::None => tools,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::PersonaComputer;
    use crate::driver::{MessageKind, Update};
    use crate::session::lock;
    use crate::session::runner::RunSpec;
    use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
    use tokio_util::sync::CancellationToken;

    fn room_with(ada: &Persona, log: crate::log::Log, agents: Arc<Fake>) -> Arc<Room> {
        enrol(&log, ada);
        Room::with_agents_and_computers(
            log,
            Arc::new(DeskKeys),
            agents,
            crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
        )
    }

    fn reply(text: &str) -> Vec<Update> {
        vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "m1".to_string(),
                text: text.to_string(),
            },
            Update::Turn {
                stop_reason: "end_turn".to_string(),
                usage: None,
            },
        ]
    }

    fn with_computer() -> Persona {
        let mut ada = persona("ada");
        ada.session_checkpoints = vec![SessionCheckpoint {
            backend_id: ada.backend_id.clone(),
            session_id: "main".to_string(),
        }];
        ada.last_session_id = Some("main".to_string());
        ada.computer = Some(PersonaComputer {
            cpus: None,
            enabled: true,
            image: None,
            memory: None,
            pids: None,
            mounts: None,
        });
        ada
    }

    #[test]
    fn a_threads_agent_never_holds_its_teammates_session() {
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Job] {
            let view = own_view(with_computer(), Policy::of(kind));
            assert!(view.session_checkpoints.is_empty(), "{kind:?}");
            assert_eq!(view.last_session_id, None, "{kind:?}");
        }
    }

    #[test]
    fn the_dm_resumes_the_session_its_teammate_holds() {
        let view = own_view(with_computer(), Policy::of(ThreadKind::Dm));
        assert_eq!(view.session_checkpoints.len(), 1);
        assert_eq!(view.last_session_id.as_deref(), Some("main"));
    }

    #[test]
    fn the_computer_is_shared_with_every_thread_with_an_agent_of_its_own() {
        let computer = |kind| {
            own_view(with_computer(), Policy::of(kind))
                .computer
                .is_some_and(|computer| computer.enabled)
        };
        for kind in [
            ThreadKind::Dm,
            ThreadKind::Side,
            ThreadKind::Run,
            ThreadKind::Job,
        ] {
            assert!(computer(kind), "{kind:?}");
        }
        assert!(computer(ThreadKind::Call));
    }

    #[test]
    fn a_clone_is_on_the_model_and_effort_its_home_session_is_on() {
        use crate::contract::SessionConfig;
        let mut ada = persona("ada");
        ada.model_id = Some("vendor/recorded".to_string());
        ada.effort_id = Some("low".to_string());
        let mut home = crate::session::idle_info("ada");
        let view = clone_of(ada.clone(), &home, &[]);
        assert_eq!(
            view.model_id.as_deref(),
            Some("vendor/recorded"),
            "no live model, so the record's stands"
        );
        home.current_model_id = Some("vendor/live".to_string());
        home.configs = vec![SessionConfig {
            id: "effort".to_string(),
            name: "Effort".to_string(),
            category: None,
            current_id: Some("high".to_string()),
            options: Vec::new(),
            note: None,
        }];
        let view = clone_of(ada, &home, &[]);
        assert_eq!(view.model_id.as_deref(), Some("vendor/live"));
        assert_eq!(view.effort_id.as_deref(), Some("high"));
    }

    /// A clone has the home thread's permission set, and the home agent can
    /// only take from it: each ability left out is gone from the view the
    /// agent is built and its turns are reached from.
    #[test]
    fn a_clone_is_narrowed_by_what_the_home_agent_left_out() {
        use crate::contract::{McpPolicy, PolicyMode};
        let mut ada = with_computer();
        ada.reach = Some(Reach::Machine);
        ada.folders = Some(vec![crate::contract::FolderGrant {
            identity: None,
            path: "/srv/harbour".to_string(),
            writable: true,
        }]);
        ada.mcp_policy = McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        };
        let home = crate::session::idle_info("ada");
        let whole = clone_of(ada.clone(), &home, &[]);
        assert_eq!(whole.reach, Some(Reach::Machine));
        assert!(whole.computer.as_ref().is_some_and(|c| c.enabled));
        assert_eq!(whole.mcp_policy.mode, PolicyMode::All);

        let narrowed = clone_of(
            ada,
            &home,
            &[Ability::Computer, Ability::Folders, Ability::McpServers],
        );
        assert!(!narrowed.computer.as_ref().is_some_and(|c| c.enabled));
        assert_eq!(narrowed.reach, Some(Reach::Workspace));
        assert_eq!(narrowed.folders, None);
        assert_eq!(narrowed.mcp_policy.mode, PolicyMode::None);
    }

    #[test]
    fn a_session_is_reopened_only_by_the_child_that_issued_it() {
        let binding = |backend: &str| {
            Some(AgentBinding {
                backend_id: backend.to_string(),
                session_id: "s1".to_string(),
            })
        };
        assert_eq!(
            resumable(Resume::Binding, binding("claude"), "claude", false).as_deref(),
            Some("s1")
        );
        assert_eq!(
            resumable(Resume::Binding, binding("claude"), "codex", false),
            None
        );
        assert_eq!(
            resumable(
                Resume::Binding,
                binding(HOTLINE_BACKEND_ID),
                HOTLINE_BACKEND_ID,
                true
            ),
            None,
            "Hotline Agent has no session to reopen; its history is its context"
        );
        assert_eq!(resumable(Resume::Binding, None, "claude", false), None);
    }

    #[test]
    fn the_dm_holds_the_computer_under_dm_and_the_rest_under_their_kind_and_key() {
        assert_eq!(lease_key(&ThreadId::dm("ada")), "dm");
        assert_eq!(lease_key(&ThreadId::side("s1")), "side:s1");
        assert_eq!(lease_key(&ThreadId::pair("a-b")), "pair:a-b");
    }

    #[test]
    fn only_the_dm_resumes_a_checkpoint_seeds_from_its_chapter_and_does_not_wait_for_a_download() {
        let dm = Policy::of(ThreadKind::Dm);
        assert_eq!(dm.resume, Resume::Checkpoint);
        assert!(dm.seed.chapters && !dm.seed.own_history);
        assert_eq!(dm.computer, Computer::Download);
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Job] {
            let policy = Policy::of(kind);
            assert_ne!(policy.resume, Resume::Checkpoint, "{kind:?}");
            assert!(!policy.seed.chapters, "{kind:?}");
            assert_ne!(policy.computer, Computer::Download, "{kind:?}");
        }
    }

    #[test]
    fn a_thread_leases_what_its_kind_says() {
        let parent = CapabilityEpoch::default().lease();
        let side = lease_of(ThreadKind::Side, &parent);
        let run = lease_of(ThreadKind::Run, &parent);
        let dm = lease_of(ThreadKind::Dm, &parent);
        parent.revoke();
        assert!(
            run.check().is_err(),
            "a run's lease is a child of its parent's"
        );
        assert!(dm.check().is_err(), "the DM's is the parent's own");
        assert!(
            side.check().is_ok(),
            "a side thread outlives the session it was opened beside"
        );
    }

    #[test]
    fn every_kind_with_an_agent_of_its_own_is_granted_the_computer() {
        assert!(Policy::of(ThreadKind::Call).has_computer());
        for kind in [
            ThreadKind::Dm,
            ThreadKind::Side,
            ThreadKind::Run,
            ThreadKind::Job,
        ] {
            assert!(Policy::of(kind).has_computer(), "{kind:?}");
        }
    }

    /// A pair is what two teammates said to each other, and nothing answers
    /// in it: no agent, no tools, no computer, nothing to park.
    #[test]
    fn a_pair_has_no_agent_of_its_own() {
        let pair = Policy::of(ThreadKind::Pair);
        assert_eq!(pair.tools, Tools::None);
        assert!(!pair.has_computer());
        assert_eq!(pair.idle, crate::thread::Idle::Never);
        assert_eq!(pair.resume, Resume::Never);
    }

    #[test]
    fn only_a_side_thread_is_told_the_parents_tail_and_a_run_hears_nothing_of_itself() {
        assert!(Policy::of(ThreadKind::Side).seed.parent_tail);
        assert!(Policy::of(ThreadKind::Side).seed.own_history);
        let run = Policy::of(ThreadKind::Run).seed;
        assert!(!run.parent_tail && !run.own_history);
    }

    #[tokio::test]
    async fn a_run_starts_fresh_though_its_stream_already_holds_its_task() {
        let agents = Fake::new(Scripted::new(reply("Done.")));
        let room = room_with(&persona("ada"), scratch("agent-run-fresh"), agents.clone());
        room.run(
            RunSpec {
                work_for: Default::default(),
                persona_id: "ada".to_string(),
                run_id: "r1".to_string(),
                title: "Check".to_string(),
                brief: crate::session::Brief::subagent("Check the crane.".to_string(), Vec::new()),
                capability: room.capability_lease("ada"),
            },
            CancellationToken::new(),
        )
        .await;
        assert!(
            lock(&agents.seeds).iter().all(Vec::is_empty),
            "a run is told nothing of any conversation, its own task included"
        );
        let tools = agents.tools();
        assert!(tools[0].in_run() && !tools[0].in_work());
        assert!(
            lock(&agents.preambles)[0].contains("This is a subagent run of Ada's"),
            "a run is the teammate, told it is a run"
        );
    }

    /// A confined teammate whose working directory became a link (its shell
    /// removed the emptied folder and made one) does not start there: the
    /// skills, the computer and the tools would all follow it, so nothing is
    /// written where it points and the person is told why. One whose folder
    /// is reached through a linked folder above it starts as before.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_teammate_whose_workspace_became_a_link_does_not_start_through_it() {
        let agents = Fake::new(Scripted::new(reply("Hello.")));
        let log = scratch("agent-linked-root");
        let outside = log.root().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = log.root().join("work");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let mut ada = persona("ada");
        ada.reach = None;
        ada.cwd = link.to_string_lossy().into_owned();
        let room = room_with(&ada, log, agents.clone());
        let refused = room.start("ada").await.unwrap_err();
        assert!(refused.contains("is now a link to"), "{refused}");
        assert!(
            std::fs::read_dir(&outside).unwrap().next().is_none(),
            "nothing is written where the link points"
        );
        assert!(lock(&agents.preambles).is_empty(), "no agent was built");

        let agents = Fake::new(Scripted::new(reply("Hello.")));
        let log = scratch("agent-linked-parent");
        let real = log.root().join("real");
        std::fs::create_dir_all(real.join("project")).unwrap();
        let alias = log.root().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let mut ada = persona("ada");
        ada.reach = None;
        ada.cwd = alias.join("project").to_string_lossy().into_owned();
        let room = room_with(&ada, log, agents.clone());
        room.start("ada").await.unwrap();
        assert!(real.join("project/.agents/skills").is_dir());
    }

    /// A teammate on workspace reach starts only in the directory its record
    /// names (`cwdIdentity`), before anything is made or written: one
    /// renamed away and remade, or gone, is refused and never made again. A
    /// record without one binds its folder at its first start, written to the
    /// record first, making it if it is not there yet, unless a confined
    /// teammate could have replaced it or a folder leading to it (a link
    /// there, or no link at all), which the person settles by choosing it
    /// again; an older grant that is now a link is granted again.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_teammate_starts_only_in_the_directory_its_record_names() {
        let identity = |path: &std::path::Path| crate::tools::path_identity(path).unwrap();
        let recorded = |room: &Arc<Room>| room.persona("ada").unwrap().cwd_identity;
        let skills = |path: &std::path::Path| path.join(".agents/skills").is_dir();
        let confined = |cwd: &std::path::Path| {
            let mut ada = persona("ada");
            ada.reach = None;
            ada.cwd = cwd.to_string_lossy().into_owned();
            ada
        };
        let room_of =
            |ada: &Persona, log| room_with(ada, log, Fake::new(Scripted::new(reply("Hello."))));

        // Bound: the same directory starts; another at its path does not, and
        // one that is gone is not made again.
        let log = scratch("agent-root-bound");
        let work = log.root().join("work");
        std::fs::create_dir(&work).unwrap();
        let mut ada = confined(&work);
        ada.cwd_identity = Some(identity(&work));
        let room = room_of(&ada, log);
        room.start("ada").await.unwrap();
        assert!(skills(&work));
        room.stop("ada").unwrap();
        std::fs::rename(&work, work.with_file_name("work-was")).unwrap();
        std::fs::create_dir(&work).unwrap();
        let refused = room.start("ada").await.unwrap_err();
        assert!(refused.contains("no longer the folder chosen"), "{refused}");
        assert!(!skills(&work), "nothing is written in the other directory");
        std::fs::remove_dir(&work).unwrap();
        let refused = room.start("ada").await.unwrap_err();
        assert!(refused.contains("no longer the folder chosen"), "{refused}");
        assert!(
            !work.exists(),
            "a bound folder that is gone is not made again"
        );

        // Unbound: a plain folder is bound and recorded at its first start, and
        // a new teammate's folder that is not made yet is made and bound.
        for made in [true, false] {
            let log = scratch(if made {
                "agent-root-adopt"
            } else {
                "agent-root-new"
            });
            let work = log.root().join("work");
            if made {
                std::fs::create_dir(&work).unwrap();
            }
            let room = room_of(&confined(&work), log);
            assert_eq!(recorded(&room), None);
            room.start("ada").await.unwrap();
            assert_eq!(recorded(&room), Some(identity(&work)));
            assert!(skills(&work));
        }

        // Not when Bob, confined, may write a folder leading to it: through a
        // link in his folder, or with no link at all.
        for linked in [true, false] {
            let log = scratch(if linked {
                "agent-root-planted"
            } else {
                "agent-root-inside"
            });
            let holder = log.root().join("holder");
            let elsewhere = log.root().join("elsewhere");
            std::fs::create_dir_all(&holder).unwrap();
            std::fs::create_dir_all(elsewhere.join("work")).unwrap();
            if linked {
                std::os::unix::fs::symlink(&elsewhere, holder.join("mid")).unwrap();
            } else {
                std::fs::create_dir_all(holder.join("mid/work")).unwrap();
            }
            let mut bob = confined(&log.root().join("bob"));
            bob.id = "bob".to_string();
            bob.folders = Some(vec![crate::contract::FolderGrant {
                path: holder
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                writable: true,
                identity: None,
            }]);
            enrol(&log, &bob);
            let room = room_of(&confined(&holder.join("mid/work")), log);
            let refused = room.start("ada").await.unwrap_err();
            assert!(refused.contains("Choose the folder again"), "{refused}");
            assert!(!skills(&elsewhere.join("work")) && !skills(&holder.join("mid/work")));
            assert_eq!(recorded(&room), None);
        }

        // An older grant whose path is now a link is granted again by the
        // person, never adopted where it now leads.
        let log = scratch("agent-root-linked-grant");
        let work = log.root().join("work");
        let granted = log.root().join("granted");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(log.root().join("target")).unwrap();
        std::os::unix::fs::symlink(log.root().join("target"), &granted).unwrap();
        let mut ada = confined(&work);
        ada.folders = Some(vec![crate::contract::FolderGrant {
            path: granted.to_string_lossy().into_owned(),
            writable: true,
            identity: None,
        }]);
        let room = room_of(&ada, log);
        let refused = room.start("ada").await.unwrap_err();
        assert!(refused.contains("Grant the folder again"), "{refused}");
        assert!(!skills(&work));
        assert!(
            room.persona("ada").unwrap().folders.unwrap()[0]
                .identity
                .is_none()
        );
    }

    /// Binding a workspace waits on the policy lock a grant change holds, and
    /// reads the record only then: a start that began while the person was
    /// choosing the folder again at the same path cannot finish until that
    /// change is written, and then runs in, and leaves recorded, the newer
    /// choice rather than what it would have adopted.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_waits_for_a_grant_change_and_the_newer_choice_wins() {
        let log = scratch("agent-root-race");
        let work = log.root().join("work");
        std::fs::create_dir(&work).unwrap();
        let mut ada = persona("ada");
        ada.reach = None;
        ada.cwd = work.to_string_lossy().into_owned();
        let agents = Fake::new(Scripted::new(reply("Hello.")));
        let room = room_with(&ada, log, agents.clone());

        let gate = room.policy_update_lock();
        let held = gate.lock().await;
        let starting = tokio::spawn({
            let room = room.clone();
            async move { room.start("ada").await }
        });
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        assert!(
            !starting.is_finished(),
            "a start cannot bind while a change holds the lock"
        );

        // The person chooses the same path again after it was remade.
        std::fs::rename(&work, work.with_file_name("work-was")).unwrap();
        std::fs::create_dir(&work).unwrap();
        let chosen = crate::tools::path_identity(&work).unwrap();
        let mut record = room.persona("ada").unwrap();
        record.cwd_identity = Some(chosen.clone());
        crate::room::append_persona(&room.log, &record).unwrap();
        drop(held);

        starting.await.unwrap().unwrap();
        assert_eq!(
            room.persona("ada").unwrap().cwd_identity,
            Some(chosen.clone())
        );
        assert_eq!(agents.views().pop().unwrap().cwd_identity, Some(chosen));
        assert!(work.join(".agents/skills").is_dir());
        assert!(!work.with_file_name("work-was").join(".agents").exists());
    }

    /// A permission change made while a start waits on the policy lock
    /// always wins: with a granted folder taken away, made read-only, or
    /// whole-machine reach narrowed, the waiting start does not go on with
    /// what it had: no agent is built and nothing is written, and the
    /// reattach the change orders starts it from the record.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_does_not_go_on_with_access_taken_away_while_it_waited() {
        for change in ["revoked", "read-only", "reach"] {
            let log = scratch(&format!("agent-access-{change}"));
            let work = log.root().join("work");
            let shared = log.root().join("shared");
            std::fs::create_dir_all(&work).unwrap();
            std::fs::create_dir_all(&shared).unwrap();
            let shared = shared.canonicalize().unwrap();
            let mut ada = persona("ada");
            ada.cwd = work.to_string_lossy().into_owned();
            ada.cwd_identity = crate::tools::path_identity(&work);
            ada.reach = (change == "reach").then_some(Reach::Machine);
            ada.folders = Some(vec![crate::contract::FolderGrant {
                path: shared.to_string_lossy().into_owned(),
                writable: true,
                identity: crate::tools::path_identity(&shared),
            }]);
            let agents = Fake::new(Scripted::new(reply("Hello.")));
            let room = room_with(&ada, log, agents.clone());

            let gate = room.policy_update_lock();
            let held = gate.lock().await;
            let starting = tokio::spawn({
                let room = room.clone();
                async move { room.start("ada").await }
            });
            for _ in 0..200 {
                tokio::task::yield_now().await;
            }
            assert!(
                !starting.is_finished(),
                "{change}: the start waits on the lock"
            );
            let mut record = room.persona("ada").unwrap();
            match change {
                "revoked" => record.folders = None,
                "read-only" => record.folders.as_mut().unwrap()[0].writable = false,
                _ => record.reach = None,
            }
            crate::room::append_persona(&room.log, &record).unwrap();
            drop(held);

            let refused = starting.await.unwrap().unwrap_err();
            assert!(refused.contains("access changed"), "{change}: {refused}");
            assert!(
                agents.views().is_empty(),
                "{change}: no agent ran with the old access"
            );
            assert!(
                !work.join(".agents").exists(),
                "{change}: nothing was written"
            );
        }
    }

    /// A thread's own narrowing survives the binding: a view without the
    /// teammate's folders, or with one read-only that the record makes
    /// writable, is bound as it is and never widened to the record.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_threads_own_narrower_access_survives_the_binding() {
        let log = scratch("agent-access-narrowed");
        let work = log.root().join("work");
        let shared = log.root().join("shared");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        let shared = shared.canonicalize().unwrap();
        let mut ada = persona("ada");
        ada.reach = None;
        ada.cwd = work.to_string_lossy().into_owned();
        ada.cwd_identity = crate::tools::path_identity(&work);
        ada.folders = Some(vec![crate::contract::FolderGrant {
            path: shared.to_string_lossy().into_owned(),
            writable: true,
            identity: crate::tools::path_identity(&shared),
        }]);
        let room = room_with(&ada, log, Fake::new(Scripted::new(reply("Hello."))));

        let mut without = ada.clone();
        without.folders = None;
        room.bind_workspace(&mut without).await.unwrap();
        assert_eq!(without.folders, None);

        let mut read_only = ada.clone();
        read_only.folders.as_mut().unwrap()[0].writable = false;
        room.bind_workspace(&mut read_only).await.unwrap();
        let folders = read_only.folders.unwrap();
        assert_eq!(folders.len(), 1);
        assert!(!folders[0].writable, "never widened to the record");
    }

    #[tokio::test]
    async fn a_side_thread_gets_its_own_tools_the_teammates_folder_and_a_childs_identity_file() {
        let agents = Fake::new(Scripted::new(reply("On it.")));
        agents.reporting("s-side", true);
        let log = scratch("agent-side-folder");
        let mut ada = persona("ada");
        ada.backend_id = "cursor".to_string();
        let folder = log.root().join("not-made-yet");
        ada.cwd = folder.to_string_lossy().into_owned();
        let room = room_with(&ada, log, agents.clone());
        let summary = room.start_side("ada", "Triage").await.unwrap();
        // The agent is built on a task of its own: wait for its turn to end.
        for _ in 0..500 {
            if room.sides("ada").iter().all(|side| !side.working) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let _ = summary;
        assert!(folder.is_dir(), "the builder makes the working directory");
        assert!(
            folder.join("AGENTS.md").is_file(),
            "a child is told who it is by a file, before it starts"
        );
        let tools = agents.tools();
        assert!(tools[0].in_work() && !tools[0].in_run());
    }
}
