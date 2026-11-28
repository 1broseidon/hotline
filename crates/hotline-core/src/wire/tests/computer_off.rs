//! A change that takes something from a teammate's computer stops its
//! container before the change is answered, so a person told "saved" is not
//! left with a container that still has the folder, the secret or the whole
//! computer they took away. Turned off, the computer is stopped, not removed:
//! turned on again it is the same container, home and bearer and all.
//!
//! The runtime is the scripted `docker` of `computer/fixtures.rs`, which
//! logs every command and keeps the container's state in a file; no real
//! runtime is asked anything and no image is pulled. A stop or a create is
//! held open by a file the test removes, and the stop's place in the
//! container's queue is read from its lock, never waited for with a sleep.

use super::*;
use crate::computer::Taken;
use crate::session::tests::{ComputerRoom, enrol, kept_computer_room};
use crate::test_support::until;

/// What a phone waits for any answer.
const PHONE_WAITS: Duration = Duration::from_secs(20);

const CONTAINER: &str = "hotline-computer-ada";

struct Desk {
    fixture: ComputerRoom,
    handle: Arc<dyn RoomHandle>,
}

/// Ada with her computer on, over the scripted runtime, not yet started.
async fn desk(name: &str) -> Desk {
    let fixture = kept_computer_room(name, Some("0.11.0")).await;
    let handle: Arc<dyn RoomHandle> = Arc::new(CoreHandle {
        room: fixture.room.clone(),
    });
    Desk { fixture, handle }
}

impl Desk {
    fn room(&self) -> &Arc<Room> {
        &self.fixture.room
    }

    /// The owner asks, and the answer comes back within what a phone waits.
    async fn ask(&self, cmd: &str, params: Value) -> Value {
        tokio::time::timeout(
            PHONE_WAITS,
            remote_control_answer(
                Seat::Owner,
                &self.handle,
                self.room().log(),
                &Parts::default(),
                json!({ "id": 1, "cmd": cmd, "params": params }),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{cmd} was not answered within a phone's wait"))
    }

    /// `persona.update` with ada's computer as she has it, `enabled` set.
    async fn turn_computer(&self, enabled: bool) -> Value {
        let mut computer = json!(self.ada().computer);
        computer["enabled"] = json!(enabled);
        self.ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "computer": computer } }),
        )
        .await
    }

    fn ada(&self) -> Persona {
        room::roster(self.room().log())
            .into_iter()
            .find(|persona| persona.id == "ada")
            .unwrap()
    }

    /// What the runtime says the container is: absent, stopped or running.
    fn container(&self) -> String {
        std::fs::read_to_string(self.fixture.root.join("state"))
            .unwrap()
            .trim()
            .to_string()
    }

    /// The bearer the runtime made the container with.
    fn bearer(&self) -> String {
        std::fs::read_to_string(self.fixture.root.join("state.envtoken")).unwrap()
    }

    fn commands(&self) -> Vec<String> {
        std::fs::read_to_string(self.fixture.root.join("argv.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The commands given after the first `seen` of them.
    fn since(&self, seen: usize) -> Vec<String> {
        self.commands().split_off(seen)
    }

    fn hold(&self, what: &str) {
        std::fs::write(self.fixture.root.join(format!("state.{what}")), "").unwrap();
    }

    fn release(&self, what: &str) {
        std::fs::remove_file(self.fixture.root.join(format!("state.{what}"))).unwrap();
    }

    fn state(&self) -> SessionState {
        self.room().info("ada").state
    }

    fn notices(&self) -> Vec<String> {
        self.room()
            .log()
            .load(&StreamId::Tape("ada".into()))
            .iter()
            .filter(|event| event["kind"] == "notice")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .collect()
    }

    /// Ada running, with her computer.
    async fn running(&self) {
        self.room().start("ada").await.unwrap();
        assert_eq!(self.container(), "running");
        assert!(self.room().computer_in_use("ada"), "she has her computer");
    }

    /// Waits for the restart a change placed to have brought ada back.
    async fn back(&self) {
        until("ada is back after the change", || {
            self.state() == SessionState::Ready
        })
        .await;
    }
}

fn is(command: &str, word: &str) -> bool {
    command == format!("{word} {CONTAINER}")
}

/// Off from the pane: stopped before the answer, home and bearer kept, and
/// the teammate's restart does not start it again. On again, the same
/// container starts: no create, no remove, the same bearer.
#[tokio::test]
async fn turning_a_computer_off_stops_it_before_the_answer_and_on_brings_the_same_one_back() {
    let desk = desk("off-stops").await;
    desk.running().await;
    let bearer = desk.bearer();
    let seen = desk.commands().len();

    let off = desk.turn_computer(false).await;
    assert_eq!(off["ok"], true, "{off}");
    // As the answer arrives, the runtime has stopped the container.
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
    let stops = desk.since(seen);
    assert!(stops.iter().any(|line| is(line, "stop")), "{stops:?}");
    assert!(
        !stops.iter().any(|line| line.starts_with("rm")),
        "stopped, not removed: {stops:?}"
    );

    // The restart brings ada back without it, and nothing starts it again.
    desk.back().await;
    assert!(!desk.room().computer_in_use("ada"));
    assert_eq!(desk.container(), "stopped");
    let after = desk.since(seen);
    assert!(!after.iter().any(|line| is(line, "start")), "{after:?}");

    // On again: the same container comes back.
    let seen = desk.commands().len();
    let on = desk.turn_computer(true).await;
    assert_eq!(on["ok"], true, "{on}");
    until("ada is back with her computer", || {
        desk.room().computer_in_use("ada")
    })
    .await;
    assert_eq!(desk.container(), "running");
    let back = desk.since(seen);
    assert!(back.iter().any(|line| is(line, "start")), "{back:?}");
    assert!(
        !back
            .iter()
            .any(|line| line.starts_with("create") || line.starts_with("rm")),
        "the same computer, not a new one: {back:?}"
    );
    assert_eq!(desk.bearer(), bearer, "the same container, with its bearer");
}

/// The phone's own switch stops it before its answer the same way, and
/// removing the teammate removes the container before that answer.
#[tokio::test]
async fn the_phones_switch_and_removing_the_teammate_take_the_container_before_the_answer() {
    let desk = desk("off-phone").await;
    desk.running().await;

    let off = desk
        .ask(
            "mobile.persona_computer",
            json!({ "id": "ada", "enabled": false }),
        )
        .await;
    assert_eq!(off["ok"], true, "{off}");
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
    desk.back().await;

    // On again, then the teammate removed while her computer runs.
    let on = desk
        .ask(
            "mobile.persona_computer",
            json!({ "id": "ada", "enabled": true }),
        )
        .await;
    assert_eq!(on["ok"], true, "{on}");
    until("ada is back with her computer", || {
        desk.room().computer_in_use("ada")
    })
    .await;
    let gone = desk.ask("persona.delete", json!({ "id": "ada" })).await;
    assert_eq!(gone["ok"], true, "{gone}");
    assert_eq!(desk.container(), "absent", "removed before the answer");
}

/// A folder granted to a computer that stays on, taken away or made
/// read-only: the container is stopped before the answer, running teammate
/// or not, and the next start makes it again with the folders she has now.
#[tokio::test]
async fn a_folder_taken_from_a_computer_that_stays_on_stops_it_before_the_answer() {
    let desk = desk("off-folder").await;
    let folder = crate::computer::fixtures::scratch("off-folder-granted");
    let path = folder.to_string_lossy().into_owned();
    let granted = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "folders": [{ "path": path, "writable": true }] } }),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    desk.running().await;
    let recorded =
        || std::fs::read_to_string(desk.fixture.root.join("state.folders")).unwrap_or_default();
    assert!(recorded().contains(&path), "made with the folder");

    // Running: stopped at the answer, then made again by the restart.
    let seen = desk.commands().len();
    let ro = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "folders": [{ "path": path, "writable": false }] } }),
        )
        .await;
    assert_eq!(ro["ok"], true, "{ro}");
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
    desk.back().await;
    until("the restart made it again, read-only", || {
        desk.room().computer_in_use("ada")
    })
    .await;
    assert!(recorded().ends_with(":ro"), "{}", recorded());
    let remade = desk.since(seen);
    assert!(
        remade
            .iter()
            .any(|line| line == &format!("rm -f {CONTAINER}"))
    );

    // At rest, with the container still up: stopped at the answer, and the
    // next start makes it without the folder.
    desk.room().stop("ada").unwrap();
    assert_eq!(desk.container(), "running", "a stopped teammate's idles");
    let taken = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "folders": [] } }),
        )
        .await;
    assert_eq!(taken["ok"], true, "{taken}");
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
    assert_eq!(desk.state(), SessionState::Stopped, "nothing restarted her");
    desk.room().start("ada").await.unwrap();
    assert!(!recorded().contains(&path), "made again without it");
}

/// A secret granted to the computer taken away: the container, which holds
/// it in memory and in its jobs' environment, is stopped before the answer,
/// running teammate or not, and the computer that comes back is handed the
/// set granted now.
#[tokio::test]
async fn a_secret_taken_from_a_computer_that_stays_on_stops_it_before_the_answer() {
    let desk = desk("off-secret").await;
    desk.room()
        .vault_for_tests()
        .set_shared_secret("GITHUB_TOKEN", "ghp_notarealtoken0001")
        .unwrap();
    let grant = |desk: &Desk| {
        let mut ada = desk.ada();
        ada.secrets = Some(vec!["GITHUB_TOKEN".into()]);
        ada.secret_grants = Some(vec![crate::contract::SecretGrant {
            name: "GITHUB_TOKEN".into(),
            readable: true,
            on: None,
        }]);
        enrol(desk.room().log(), &ada);
    };
    grant(&desk);
    desk.running().await;
    let handed = || {
        desk.fixture
            .taken
            .sets()
            .last()
            .cloned()
            .unwrap_or_default()
    };
    assert!(handed().contains_key("GITHUB_TOKEN"));

    // Running.
    let taken = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "secrets": [] } }),
        )
        .await;
    assert_eq!(taken["ok"], true, "{taken}");
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
    desk.back().await;
    until("the restart brought the computer back", || {
        desk.room().computer_in_use("ada")
    })
    .await;
    assert!(
        handed().is_empty(),
        "handed what is granted now: {:?}",
        handed()
    );

    // At rest, its container up: the same.
    grant(&desk);
    desk.room().secrets_changed().await;
    assert!(handed().contains_key("GITHUB_TOKEN"));
    desk.room().stop("ada").unwrap();
    assert_eq!(desk.container(), "running");
    let taken = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "secrets": [] } }),
        )
        .await;
    assert_eq!(taken["ok"], true, "{taken}");
    assert_eq!(desk.container(), "stopped", "stopped before the answer");
}

/// The sweep counts no teammate whose computer is off as using it: a
/// record turned off with no stop behind it (a desk that went down between
/// the two) has its container stopped at the next sweep, though the
/// teammate's session runs; and so has a container an older build left
/// running for a teammate whose computer is off.
#[tokio::test]
async fn the_sweep_stops_a_turned_off_computer_even_while_its_teammate_runs() {
    let desk = desk("off-sweep").await;
    desk.running().await;
    desk.room().sweep_computers().await;
    assert_eq!(desk.container(), "running", "in use while on");

    let mut ada = desk.ada();
    ada.computer.as_mut().unwrap().enabled = false;
    enrol(desk.room().log(), &ada);
    assert_eq!(desk.state(), SessionState::Ready, "her session still runs");
    desk.room().sweep_computers().await;
    assert_eq!(
        desk.container(),
        "stopped",
        "a running session is no use of it"
    );

    // A desk opened afresh over one an older build left running.
    std::fs::write(desk.fixture.root.join("state"), "running").unwrap();
    let reopened = crate::session::tests::reopened(&desk.fixture);
    reopened.sweep_computers().await;
    assert_eq!(desk.container(), "stopped", "adopted and stopped at once");
}

/// A stop the runtime refuses is told, in the answer and on the
/// conversation, and is not forgotten: the container is not used until it
/// stops, and the sweep tries again.
#[tokio::test]
async fn a_stop_that_fails_is_told_and_tried_again_before_use_and_at_the_sweep() {
    let desk = desk("off-fails").await;
    let folder = crate::computer::fixtures::scratch("off-fails-granted");
    let path = folder.to_string_lossy().into_owned();
    let granted = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "folders": [{ "path": path, "writable": true }] } }),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    desk.running().await;
    desk.room().stop("ada").unwrap();

    desk.hold("stopfail");
    let taken = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "folders": [] } }),
        )
        .await;
    assert_eq!(taken["ok"], false, "{taken}");
    let said = taken["error"].as_str().unwrap_or_default().to_string();
    assert!(
        said.starts_with("Saved, but Ada's computer did not stop"),
        "{said}"
    );
    assert!(said.contains("would not stop the container"), "{said}");
    assert!(
        desk.notices()
            .iter()
            .any(|text| text.starts_with("Ada's computer did not stop")),
        "{:?}",
        desk.notices()
    );
    // Saved all the same.
    assert!(desk.ada().folders.is_none());
    assert_eq!(desk.container(), "running");

    // Nothing uses it while it will not stop.
    let seen = desk.commands().len();
    let refused = desk.room().computer_up("ada", |_| {}).await.unwrap_err();
    assert!(
        refused.contains("would not stop, so it is not used"),
        "{refused}"
    );
    assert!(!desk.since(seen).iter().any(|line| is(line, "start")));

    // The sweep tries again, and once it stops the next start makes it anew.
    desk.release("stopfail");
    desk.room().sweep_computers().await;
    assert_eq!(desk.container(), "stopped");
    desk.room().computer_up("ada", |_| {}).await.unwrap();
    let remade = desk.since(seen);
    assert!(
        remade
            .iter()
            .any(|line| line == &format!("rm -f {CONTAINER}"))
    );
}

/// A stop that is still going when the answer's wait runs out is not let
/// go: the change is answered as saved (a phone must not be told a saved
/// change was not done), the stop goes on, and a failure after the answer
/// is said on the conversation.
#[tokio::test]
async fn a_stop_past_the_answers_wait_goes_on_and_a_late_failure_is_told() {
    let desk = desk("off-slow").await;
    desk.running().await;
    let mut ada = desk.ada();
    ada.computer.as_mut().unwrap().enabled = false;
    enrol(desk.room().log(), &ada);

    desk.hold("stopgate");
    desk.room()
        .computer_taken_away_within("ada", Taken::Stop, Duration::from_millis(50))
        .await
        .expect("a stop still going is answered as saved");
    assert_eq!(desk.container(), "running", "the stop is still held");
    desk.release("stopgate");
    until("the stop went on after the answer", || {
        desk.container() == "stopped"
    })
    .await;

    // Again, and this time the runtime refuses once the answer has gone.
    std::fs::write(desk.fixture.root.join("state"), "running").unwrap();
    desk.hold("stopgate");
    desk.hold("stopfail");
    desk.room()
        .computer_taken_away_within("ada", Taken::Stop, Duration::from_millis(50))
        .await
        .expect("answered as saved");
    desk.release("stopgate");
    until("the late failure is said on the conversation", || {
        desk.notices()
            .iter()
            .any(|text| text.starts_with("Ada's computer did not stop"))
    })
    .await;
    // And from then on a change to her computer says it, until it stops.
    let again = desk.turn_computer(false).await;
    assert_eq!(again["ok"], false, "{again}");
}

/// While a change's stop has failed and nothing has done it since, a second
/// change to the computer that finds nothing new to take (a second "off")
/// answers the same error rather than a clean "saved"; a change to
/// anything else does not; once a later try stops it, the next is clean.
#[tokio::test]
async fn a_second_change_while_a_stop_has_failed_answers_the_same_error() {
    let desk = desk("off-still-up").await;
    desk.running().await;
    desk.hold("stopfail");
    let off = desk.turn_computer(false).await;
    assert_eq!(off["ok"], false, "{off}");
    let first = off["error"].as_str().unwrap().to_string();
    assert!(
        first.starts_with("Saved, but Ada's computer did not stop"),
        "{first}"
    );
    desk.back().await;
    assert_eq!(desk.container(), "running");

    let again = desk.turn_computer(false).await;
    assert_eq!(again["ok"], false, "{again}");
    assert_eq!(
        again["error"],
        first.as_str(),
        "the same error, nothing new taken"
    );
    let phone = desk
        .ask(
            "mobile.persona_computer",
            json!({ "id": "ada", "enabled": false }),
        )
        .await;
    assert_eq!(phone["error"], first.as_str(), "{phone}");
    let named = desk
        .ask(
            "persona.update",
            json!({ "id": "ada", "patch": { "name": "Ada" } }),
        )
        .await;
    assert_eq!(named["ok"], true, "a name is not the computer's: {named}");

    desk.release("stopfail");
    desk.room().sweep_computers().await;
    assert_eq!(desk.container(), "stopped");
    let clean = desk.turn_computer(false).await;
    assert_eq!(clean["ok"], true, "{clean}");
}

/// Nothing racing a change brings the old container back. A start already
/// making the computer when it is turned off finishes first and is stopped
/// after it, before the answer; the restart the change placed starts no
/// computer; and a caller that read ada before the change starts nothing.
#[tokio::test]
async fn nothing_racing_a_change_brings_the_old_container_back() {
    let desk = desk("off-race").await;
    desk.hold("creategate");
    let starting = tokio::spawn({
        let room = desk.room().clone();
        async move { room.start("ada").await }
    });
    until("the start is making the computer", || {
        desk.commands()
            .iter()
            .any(|line| line.starts_with("create"))
    })
    .await;

    let off = tokio::spawn({
        let handle = desk.handle.clone();
        let log = desk.room().log().clone();
        async move {
            let mut computer = json!(
                room::roster(&log)
                    .into_iter()
                    .find(|persona| persona.id == "ada")
                    .unwrap()
                    .computer
            );
            computer["enabled"] = json!(false);
            remote_control_answer(
                Seat::Owner,
                &handle,
                &log,
                &Parts::default(),
                json!({ "id": 1, "cmd": "persona.update",
                        "params": { "id": "ada", "patch": { "computer": computer } } }),
            )
            .await
        }
    });
    // The stop has its place behind the start that holds the container.
    until("the stop is queued behind the start", || {
        desk.room().computers().queued("ada") == 2
    })
    .await;
    desk.release("creategate");
    let off = tokio::time::timeout(PHONE_WAITS, off)
        .await
        .expect("answered within a phone's wait")
        .unwrap();
    assert_eq!(off["ok"], true, "{off}");
    assert_eq!(
        desk.container(),
        "stopped",
        "what the start made is stopped"
    );
    let commands = desk.commands();
    let started = commands.iter().position(|line| is(line, "start")).unwrap();
    let stopped = commands.iter().rposition(|line| is(line, "stop")).unwrap();
    assert!(started < stopped, "{commands:?}");

    let _ = starting.await.unwrap();
    until("ada has settled", || {
        matches!(desk.state(), SessionState::Ready | SessionState::Stopped)
    })
    .await;
    // A caller holding ada as she was before the change goes through the
    // record as it is now, and is refused before the runtime is asked.
    let seen = desk.commands().len();
    let refused = desk.room().computer_up("ada", |_| {}).await.unwrap_err();
    assert_eq!(refused, "Ada's computer is turned off.");
    assert_eq!(desk.since(seen), Vec::<String>::new());
    assert_eq!(desk.container(), "stopped");
    let after = desk.since(stopped + 1);
    assert!(!after.iter().any(|line| is(line, "start")), "{after:?}");
}
