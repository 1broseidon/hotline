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
        account_id: None,
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
    use crate::contract::{FolderGrant, McpPolicy, PolicyMode};
    let mut ada = with_computer();
    ada.reach = Some(Reach::Machine);
    ada.folders = Some(vec![FolderGrant {
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
    let binding = |backend: &str, account: Option<&str>| {
        Some(AgentBinding {
            backend_id: backend.to_string(),
            session_id: "s1".to_string(),
            account_id: account.map(str::to_string),
        })
    };
    assert_eq!(
        resumable(
            Resume::Binding,
            binding("claude", None),
            "claude",
            None,
            false
        )
        .as_deref(),
        Some("s1")
    );
    assert_eq!(
        resumable(
            Resume::Binding,
            binding("claude", None),
            "codex",
            None,
            false
        ),
        None
    );
    assert_eq!(
        resumable(
            Resume::Binding,
            binding(HOTLINE_BACKEND_ID, None),
            HOTLINE_BACKEND_ID,
            None,
            true
        ),
        None,
        "Hotline Agent has no session to reopen; its history is its context"
    );
    assert_eq!(
        resumable(Resume::Binding, None, "claude", None, false),
        None
    );
    assert_eq!(
        resumable(
            Resume::Binding,
            binding("claude", Some("a")),
            "claude",
            Some("a"),
            false
        )
        .as_deref(),
        Some("s1")
    );
    assert_eq!(
        resumable(
            Resume::Binding,
            binding("claude", Some("a")),
            "claude",
            Some("b"),
            false
        ),
        None
    );
    assert_eq!(
        resumable(
            Resume::Binding,
            binding("claude", None),
            "claude",
            Some("b"),
            false
        ),
        None
    );
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
fn every_kind_with_an_agent_of_its_own_is_granted_the_computer() {
    assert!(Policy::of(ThreadKind::Call).has_computer());
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Side,
        ThreadKind::Pair,
        ThreadKind::Run,
        ThreadKind::Job,
    ] {
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

#[tokio::test]
async fn a_pair_seeds_each_driver_in_the_answerers_direction_without_the_home_session() {
    use crate::driver::rig::Said;
    use serde_json::json;

    for backend in [HOTLINE_BACKEND_ID, "cursor"] {
        for target in ["ada", "bob"] {
            let agents = Fake::new(Scripted::new(reply("Ready.")));
            let log = scratch("agent-pair-history");
            for id in ["ada", "bob"] {
                let mut teammate = persona(id);
                teammate.backend_id = backend.to_string();
                teammate.session_checkpoints = vec![SessionCheckpoint {
                    backend_id: backend.to_string(),
                    session_id: "home-session".to_string(),
                    account_id: None,
                }];
                teammate.last_session_id = Some("home-session".to_string());
                teammate.cwd = log.root().join(id).to_string_lossy().into_owned();
                enrol(&log, &teammate);
            }
            for event in [
                json!({"kind":"user", "id":"a1", "ts":1, "text":"Ada's line"}),
                json!({"kind":"agent", "id":"b1", "ts":2, "text":"Bob's line"}),
            ] {
                log.append(&StreamId::Pair("ada~bob".to_string()), &event)
                    .unwrap();
            }
            let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
            let caller = if target == "ada" { "bob" } else { "ada" };
            let _agent = room
                .thread_agent(Opening {
                    thread: ThreadId::pair("ada~bob"),
                    persona: room.persona(target).unwrap(),
                    title: "A colleague's request".to_string(),
                    opener: Some(Opener {
                        persona_id: caller.to_string(),
                        name: caller.to_string(),
                    }),
                    lease: room.capability_lease(target),
                    heard_from: None,
                })
                .await
                .unwrap();
            let expected = if target == "ada" {
                vec![
                    Said::Agent("Ada's line".to_string()),
                    Said::User("Bob's line".to_string()),
                ]
            } else {
                vec![
                    Said::User("Ada's line".to_string()),
                    Said::Agent("Bob's line".to_string()),
                ]
            };
            assert_eq!(
                crate::session::tests::words(lock(&agents.seeds)[0].clone()),
                expected
            );
            let view = &agents.views()[0];
            assert!(
                view.session_checkpoints.is_empty(),
                "a pair never reopens the home session"
            );
            assert!(view.last_session_id.is_none());
            let preamble = &lock(&agents.preambles)[0];
            assert!(preamble.contains("replying privately to your teammate"));
            assert!(preamble.contains(&room.persona(caller).unwrap().name));
            let tools = agents.tools();
            assert!(!tools[0].in_work() && !tools[0].in_run());
        }
    }
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
