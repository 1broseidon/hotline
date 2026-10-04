//! The one way an agent is built for a thread.
//!
//! A side thread and a subagent run are answered by a fresh agent of their
//! teammate, and what differs between them is the kind's [`Policy`], not the
//! code that builds it. [`Room::thread_agent`] reads the row: it makes the
//! teammate's folder, writes `AGENTS.md` for a child, takes the computer away
//! or grants it, serves the kind's tools under the kind's lease, and tells the
//! agent what the kind says it should know of the conversation it joins. Then
//! it starts the agent.
//!
//! **Resume** is one mechanism here. A thread whose record holds a session id
//! the teammate's harness issued asks the child to reopen it. If the child
//! will not, or the thread never saved one, or the agent is Hotline Agent
//! (whose context is only what it is handed), the agent is seeded from the
//! thread's own stream: its history for Hotline Agent, a fenced transcript in
//! the preamble for a child. The teammate's own session is never reopened, so
//! a thread's agent can never answer inside the main conversation.
//!
//! The main conversation (`start_now`) and a peer session (`peer_session`) are
//! still built on their own; each moves onto this when its phase lands, which is
//! why the policy has a row for them.

use super::{Room, chapters, now_ms, said, without_computer};
use crate::contract::{Persona, Reach, SessionCheckpoint};
use crate::driver::{CapabilityEpoch, CapabilityLease, Driver, HOTLINE_BACKEND_ID, acp};
use crate::mcp::server::TeammateTools;
use crate::thread::{AgentBinding, Lease, Policy, ThreadId, ThreadKind, ThreadStore, Tools};
use std::sync::Arc;

/// What a thread's agent is built for.
pub(super) struct Opening {
    pub thread: ThreadId,
    /// The teammate as this thread sees it. The builder takes the rest off it:
    /// the teammate's own session, and the computer when the kind has none.
    pub persona: Persona,
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
    /// The saved session id the agent reopened, when it did. Nothing else
    /// about the thread's own memory is promised by the harness.
    pub resumed: Option<String>,
    armed: bool,
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

/// The teammate as a thread's agent is started for it: with no session of the
/// teammate's own to reopen, which would answer inside the main conversation,
/// and with the computer only if the kind has one.
fn own_view(mut persona: Persona, policy: Policy) -> Persona {
    persona.session_checkpoints = Vec::new();
    persona.last_session_id = None;
    if policy.computer {
        persona
    } else {
        without_computer(persona)
    }
}

/// The session a child is asked to reopen: the thread's saved one, when the
/// harness the teammate runs on now is the one that issued it. Only a child
/// has a session of its own to reopen.
fn resumable(binding: Option<AgentBinding>, backend_id: &str, in_process: bool) -> Option<String> {
    binding
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
            lease,
        } = opening;
        let policy = Policy::of(thread.kind);
        let in_process = persona.backend_id == HOTLINE_BACKEND_ID;
        std::fs::create_dir_all(&persona.cwd).map_err(|error| {
            format!(
                "{}'s working directory {} could not be made: {error}",
                persona.name, persona.cwd
            )
        })?;
        let view = own_view(persona, policy);
        // An ACP session takes no system prompt, so who it is has to be on
        // disk before the child is started.
        if !in_process {
            acp::materialize_agents_md_with_capability(&view, Some(lease.clone())).map_err(
                |error| format!("{}'s AGENTS.md could not be written: {error}", view.name),
            )?;
        }
        let extra_mcp = if policy.computer {
            self.grant_computer(&view).await?
        } else {
            Vec::new()
        };
        lease.check()?;

        let reach = in_process.then(|| view.reach.unwrap_or_default());
        let context = if policy.seed.parent_tail {
            chapters::side_context(&self.tape(&view.id), now_ms())
        } else {
            None
        };
        let earlier = match thread.stream() {
            Some(stream) if policy.seed.own_history => self.log.load(&stream),
            _ => Vec::new(),
        };
        let has_history = !earlier.is_empty();
        let history = if in_process {
            said(&earlier)
        } else {
            Vec::new()
        };
        let transcript = || chapters::serialize_chapter(&earlier);
        let reopening = resumable(
            ThreadStore::new(&self.log)
                .load(&thread)
                .and_then(|record| record.binding),
            &view.backend_id,
            in_process,
        );

        let build = |view: &Persona, transcript: Option<String>| {
            self.agents.agent(
                view,
                preamble_of(
                    thread.kind,
                    &view.clone(),
                    reach,
                    context.clone(),
                    transcript,
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
            reported: info.session_id,
            resumed,
            armed: true,
        })
    }
}

/// What a kind tells its agent about itself, and about what it joins.
fn preamble_of(
    kind: ThreadKind,
    persona: &Persona,
    reach: Option<Reach>,
    context: Option<String>,
    transcript: Option<String>,
) -> String {
    match kind {
        ThreadKind::Run => super::runner::run_preamble(persona, reach),
        // The kinds not built here yet have their own preambles until they are.
        _ => super::sides::side_preamble(persona, reach, context, transcript),
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
        Tools::Side => tools.for_side(thread.key.clone()),
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
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Dm] {
            let view = own_view(with_computer(), Policy::of(kind));
            assert!(view.session_checkpoints.is_empty(), "{kind:?}");
            assert_eq!(view.last_session_id, None, "{kind:?}");
        }
    }

    #[test]
    fn the_computer_stays_with_the_main_conversation() {
        let computer = |kind| {
            own_view(with_computer(), Policy::of(kind))
                .computer
                .is_some_and(|computer| computer.enabled)
        };
        assert!(!computer(ThreadKind::Side));
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
            resumable(binding("claude"), "claude", false).as_deref(),
            Some("s1")
        );
        assert_eq!(resumable(binding("claude"), "codex", false), None);
        assert_eq!(
            resumable(binding(HOTLINE_BACKEND_ID), HOTLINE_BACKEND_ID, true),
            None,
            "Hotline Agent has no session to reopen; its history is its context"
        );
        assert_eq!(resumable(None, "claude", false), None);
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
    fn only_the_main_conversation_and_a_pair_are_granted_the_computer() {
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Call] {
            assert!(!Policy::of(kind).computer, "{kind:?}");
        }
        assert!(Policy::of(ThreadKind::Dm).computer);
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
        assert!(tools[0].in_run() && !tools[0].in_side());
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
        assert!(tools[0].in_side() && !tools[0].in_run());
    }
}
