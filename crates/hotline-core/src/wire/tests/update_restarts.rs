//! A change that restarts a teammate is answered once it is saved, not once
//! the teammate is back. A computer turned on is made and started inside the
//! restart, which can take longer than a phone waits, and a phone that gave up
//! told the person a saved change was not confirmed. The answer still stands
//! on the change: the record is durable and what it took away is revoked
//! before it is sent. The restart is the room's own work after it, and a
//! restart that fails says why on the teammate.
//!
//! The restart is held back by the agent's own start (a semaphore the test
//! hands out permits from) or by the teammate's start gate held here, never
//! by a sleep.

use super::*;
use crate::computer::Computer;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona};
use crate::test_support::until;
use tokio::sync::Semaphore;

/// How long an answer may take here: far under the phone's twenty seconds,
/// and never reached by an answer that does not wait on the held restart.
const ANSWER: Duration = Duration::from_secs(5);

struct Desk {
    _root: tempfile::TempDir,
    root: std::path::PathBuf,
    log: Log,
    room: Arc<Room>,
    handle: Arc<dyn RoomHandle>,
    agents: Arc<Fake>,
    starts: Arc<Semaphore>,
}

/// Ada running on a scripted agent whose every start takes a permit from
/// `starts`, on a desk whose computers have no runtime: a computer turned on
/// is refused at once, and nothing is pulled or made.
async fn running_ada() -> Desk {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path().join("desk"));
    enrol(&log, &persona("ada"));
    let starts = Arc::new(Semaphore::new(1));
    let agents = Fake::new(Scripted::turns(vec![Vec::new()]).slow_to_start(starts.clone()));
    let room = Room::with_agents_and_computers(
        log.clone(),
        Arc::new(DeskKeys),
        agents.clone(),
        Computer::with_path(root.path().join("no-runtime")),
    );
    room.start("ada").await.unwrap();
    let handle: Arc<dyn RoomHandle> = Arc::new(CoreHandle { room: room.clone() });
    Desk {
        root: root.path().to_path_buf(),
        _root: root,
        log,
        room,
        handle,
        agents,
        starts,
    }
}

impl Desk {
    /// The authority Ada's running session was started on.
    fn session_lease(&self) -> hotline_tools::capability::CapabilityLease {
        let lease = self
            .agents
            .tools()
            .last()
            .and_then(crate::mcp::server::TeammateTools::capability)
            .expect("the session was handed its tools under a lease");
        assert!(lease.is_current(), "the running session's lease is current");
        lease
    }

    /// The owner's phone asks, and the answer comes back within [`ANSWER`].
    async fn owner_asks(&self, id: u64, cmd: &str, params: Value) -> Value {
        tokio::time::timeout(
            ANSWER,
            remote_control_answer(
                Seat::Owner,
                &self.handle,
                &self.log,
                &Parts::default(),
                json!({ "id": id, "cmd": cmd, "params": params }),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{cmd} waited on the restart to answer"))
    }

    /// Ada's record as a desk opened afresh on the same directory reads it.
    fn ada_on_disk(&self) -> Persona {
        room::roster(&Log::open(self.root.join("desk")))
            .into_iter()
            .find(|persona| persona.id == "ada")
            .unwrap()
    }

    fn state(&self) -> SessionState {
        self.room.info("ada").state
    }
}

#[tokio::test]
async fn a_computer_turned_on_or_off_is_answered_saved_before_the_teammate_is_back() {
    let desk = running_ada().await;

    // On, from the phone's Computer page: a whole-record patch.
    let before = desk.session_lease();
    let on = desk
        .owner_asks(
            1,
            "persona.update",
            json!({"id": "ada", "patch": {"computer": {"enabled": true}}}),
        )
        .await;
    // A start asked for the moment the answer arrives, as a composer
    // sending to a teammate it sees stopped does. It waits for the restart
    // and is not refused: the epoch the change closed was opened again
    // before the answer.
    let asked = tokio::spawn({
        let room = desk.room.clone();
        async move { room.start("ada").await }
    });
    assert_eq!(on["ok"], true, "{on}");
    assert_eq!(on["result"]["computer"]["enabled"], true, "{on}");
    // Everything the answer stands on stands as it arrives.
    assert!(
        !before.is_current(),
        "the old session's authority ended before the answer"
    );
    assert!(desk.ada_on_disk().computer.is_some_and(|c| c.enabled));
    // And the restart has not finished: its start waits on a permit.
    until("the restart is starting ada", || {
        desk.state() == SessionState::Starting
    })
    .await;
    assert!(
        desk.room.start_gate("ada").try_lock().is_err(),
        "the restart holds ada's start gate"
    );
    desk.starts.add_permits(1);
    let started = tokio::time::timeout(ANSWER, asked)
        .await
        .expect("the start after the answer finishes with the restart")
        .unwrap();
    assert_eq!(started.map(|info| info.state), Ok(SessionState::Ready));

    // Off, through the owner's narrow command, the same way.
    let before = desk.session_lease();
    let off = desk
        .owner_asks(
            2,
            "mobile.persona_computer",
            json!({"id": "ada", "enabled": false}),
        )
        .await;
    assert_eq!(off["ok"], true, "{off}");
    assert_eq!(off["result"]["computer"]["enabled"], false, "{off}");
    assert!(!before.is_current());
    assert!(!desk.ada_on_disk().computer.is_some_and(|c| c.enabled));
    until("the restart is starting ada", || {
        desk.state() == SessionState::Starting
    })
    .await;
    desk.starts.add_permits(1);
    until("ada is back", || desk.state() == SessionState::Ready).await;
    assert!(desk.session_lease().is_current());
}

#[tokio::test]
async fn a_restart_that_fails_after_the_answer_says_why_on_the_teammate() {
    let desk = running_ada().await;
    desk.starts.add_permits(8);
    desk.agents.fail_starts("The harness is not installed.");
    // Held here, as a message being admitted holds it: the restart queues.
    let gate = desk.room.start_gate("ada").lock_owned().await;
    let before = desk.session_lease();

    let on = desk
        .owner_asks(
            1,
            "persona.update",
            json!({"id": "ada", "patch": {"computer": {"enabled": true}}}),
        )
        .await;
    assert_eq!(on["ok"], true, "{on}");
    assert!(!before.is_current());
    assert!(desk.ada_on_disk().computer.is_some_and(|c| c.enabled));
    assert_eq!(desk.state(), SessionState::Stopped, "revoked, not yet back");

    drop(gate);
    until("the failed restart says why", || {
        let info = desk.room.info("ada");
        info.state == SessionState::Stopped
            && info.error.as_deref() == Some("The harness is not installed.")
    })
    .await;
    // Nothing is left quarantined: once the harness is back, ada starts.
    desk.agents.allow_starts();
    assert_eq!(
        desk.room.start("ada").await.map(|info| info.state),
        Ok(SessionState::Ready)
    );
}

#[tokio::test]
async fn a_second_change_during_a_restart_brings_the_teammate_back_on_both() {
    let desk = running_ada().await;

    let on = desk
        .owner_asks(
            1,
            "persona.update",
            json!({"id": "ada", "patch": {"computer": {"enabled": true}}}),
        )
        .await;
    assert_eq!(on["ok"], true, "{on}");
    until("the first restart is starting ada", || {
        desk.state() == SessionState::Starting
    })
    .await;
    // A second save while the first restart is still bringing ada up, as
    // the pane allows now that the first answer does not wait for it. It
    // revokes the start under way.
    let narrowed = desk
        .owner_asks(
            2,
            "persona.update",
            json!({"id": "ada", "patch": {"reach": "workspace"}}),
        )
        .await;
    assert_eq!(narrowed["ok"], true, "{narrowed}");

    // The first start gets its permit and finds itself revoked; the second
    // restart starts ada again rather than finding nothing running.
    desk.starts.add_permits(2);
    until("ada is back on both changes", || {
        desk.state() == SessionState::Ready
    })
    .await;
    let info = desk.room.info("ada");
    assert_eq!(info.error, None, "a superseded start is no failure to show");
    let ada = desk.ada_on_disk();
    assert!(ada.computer.is_some_and(|c| c.enabled));
    assert_eq!(ada.reach, Some(Reach::Workspace));
    assert!(desk.session_lease().is_current());
}

#[tokio::test]
async fn a_stop_during_a_restart_is_not_undone_by_the_next_change() {
    let desk = running_ada().await;
    let on = desk
        .owner_asks(
            1,
            "persona.update",
            json!({"id": "ada", "patch": {"computer": {"enabled": true}}}),
        )
        .await;
    assert_eq!(on["ok"], true, "{on}");
    until("the restart is starting ada", || {
        desk.state() == SessionState::Starting
    })
    .await;
    desk.room.stop("ada").unwrap();
    let narrowed = desk
        .owner_asks(
            2,
            "persona.update",
            json!({"id": "ada", "patch": {"reach": "workspace"}}),
        )
        .await;
    assert_eq!(narrowed["ok"], true, "{narrowed}");
    desk.starts.add_permits(2);
    // The second restart has run once its gate is free, and left ada down.
    drop(desk.room.start_gate("ada").lock_owned().await);
    let info = desk.room.info("ada");
    assert_eq!(info.state, SessionState::Stopped);
    assert_eq!(info.error, None, "the person's stop is no failure");
    assert!(!desk.room.has_session("ada"));
}
