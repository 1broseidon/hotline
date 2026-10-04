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
//! offers are taken before it starts. A peer session (`peer_session`) is still
//! built on its own, and moves onto this when its phase lands.

use super::{
    COMPUTER_DOWNLOADING, ComputerAtStart, Driving, Room, chapters, computer_failed_note,
    computer_unavailable, now_ms, said, with_note, without_computer,
};
use crate::contract::{Persona, Reach, SessionCheckpoint, SharedSecret};
use crate::driver::{
    CapabilityEpoch, CapabilityLease, Driver, DriverInfo, HOTLINE_BACKEND_ID, SubagentReport,
    Update, acp,
};
use crate::mcp::server::TeammateTools;
use crate::thread::{
    AgentBinding, Computer, Lease, Opener, Policy, Resume, ThreadId, ThreadKind, ThreadStore, Tools,
};
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
            persona,
            title,
            opener,
            lease,
        } = opening;
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
        let context = if policy.seed.parent_tail {
            chapters::side_context(&self.tape(&view.id), now_ms())
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
        let earlier = match thread.stream() {
            Some(stream) if policy.seed.own_history => self.log.load(&stream),
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

        let opened_by = opener.and_then(|opener| self.persona(&opener.persona_id).ok());
        let stored = self.stored_secrets();
        let build = |view: &Persona, transcript: Option<String>| {
            self.agents.agent(
                view,
                preamble_of(
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
                ),
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
        });
        let mut starting = Starting(Some(driver.clone()));
        let mut info = driver.start(&checkpointed).await?;
        lease.check()?;
        if reopening.is_some() && !info.context_restored {
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
        ThreadKind::Run => super::runner::run_preamble(persona, reach),
        // The DM's: what it needs to know of the chapter it joins, and of its
        // computer, is the note.
        ThreadKind::Dm => super::preamble(persona, reach, note, stored),
        // The kinds not built here yet have their own preambles until they are.
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
        Tools::Work => tools.for_work(thread.key.clone()),
        Tools::Run => tools.for_run(),
        Tools::Peer => tools.for_peer(),
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
            secrets: None,
        });
        ada
    }

    #[test]
    fn a_threads_agent_never_holds_its_teammates_session() {
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Pair] {
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
    fn the_computer_stays_with_the_main_conversation() {
        let computer = |kind| {
            own_view(with_computer(), Policy::of(kind))
                .computer
                .is_some_and(|computer| computer.enabled)
        };
        assert!(
            computer(ThreadKind::Side),
            "a work thread shares it through the lease"
        );
        assert!(!computer(ThreadKind::Run));
        assert!(computer(ThreadKind::Dm));
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
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Pair] {
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
    fn only_conversations_and_a_pair_are_granted_the_computer() {
        for kind in [ThreadKind::Run, ThreadKind::Call] {
            assert!(!Policy::of(kind).has_computer(), "{kind:?}");
        }
        for kind in [ThreadKind::Dm, ThreadKind::Side, ThreadKind::Pair] {
            assert!(Policy::of(kind).has_computer(), "{kind:?}");
        }
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
                persona_id: "ada".to_string(),
                run_id: "r1".to_string(),
                title: "Check".to_string(),
                task: "Check the crane.".to_string(),
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
            lock(&agents.preambles)[0].starts_with("You are a subagent"),
            "a run's preamble is the worker's, not the teammate's"
        );
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
        room.start_side("ada", "Triage").await.unwrap();
        assert!(folder.is_dir(), "the builder makes the working directory");
        assert!(
            folder.join("AGENTS.md").is_file(),
            "a child is told who it is by a file, before it starts"
        );
        let tools = agents.tools();
        assert!(tools[0].in_work() && !tools[0].in_run());
    }
}
