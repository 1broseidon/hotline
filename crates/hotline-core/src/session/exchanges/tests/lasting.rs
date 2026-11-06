//! A request between two teammates ends only by an explicit end: Stop
//! exchange, or a revocation (a policy change, either teammate stopped or
//! removed). A chapter turning over and a rebuild of the main session for a
//! computer or an MCP sign-in leave it standing: its leases are each
//! teammate's lasting authority, not the session it was sent beside.
use super::*;
use crate::contract::{ChapterClose, Persona};
use crate::session::Agents;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::Semaphore;

/// Each teammate's agents in the order the room builds them, the last
/// standing for every build after it, so a main conversation and a work
/// thread never share a driver: one shared would carry the old session's
/// stop into the thread's turn.
struct Builds {
    agents: HashMap<&'static str, Vec<Arc<Fake>>>,
    built: Mutex<HashMap<String, usize>>,
}

#[async_trait::async_trait]
impl Agents for Builds {
    fn agent(
        &self,
        persona: &Persona,
        preamble: String,
        said: Vec<crate::driver::rig::Said>,
        tools: TeammateTools,
        mcp: Vec<crate::mcp::McpServer>,
    ) -> Result<Arc<dyn crate::driver::Driver>, String> {
        let agents = &self.agents[persona.id.as_str()];
        let fake = {
            let mut built = lock(&self.built);
            let count = built.entry(persona.id.clone()).or_default();
            let fake = agents[(*count).min(agents.len() - 1)].clone();
            *count += 1;
            fake
        };
        fake.agent(persona, preamble, said, tools, mcp)
    }

    async fn complete(&self, _model: &str, _system: &str, _prompt: &str) -> Result<String, String> {
        Err("not needed".into())
    }
}

fn turn(id: &str, text: &str) -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Agent,
            id: id.into(),
            text: text.into(),
        },
        Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        },
    ]
}

/// A main conversation's agent, one per build.
fn main(who: &str) -> Arc<Fake> {
    Fake::new(Scripted::new(turn(who, "Main.")))
}

/// The room, with Bob's work thread held at `gate` and built second, after
/// his main conversation: Ada's handoff is Bob's second agent.
fn gated_room(log: crate::log::Log, gate: &Arc<Semaphore>) -> (Arc<Room>, Arc<Fake>) {
    let work = Fake::new(Scripted::new(turn("built", "Built it.")).gated(gate.clone()));
    let agents = Builds {
        agents: HashMap::from([
            ("ada", vec![main("ada")]),
            ("bob", vec![main("bob"), work.clone(), main("bob")]),
        ]),
        built: Mutex::default(),
    };
    let room = Room::with_agents(log, Arc::new(DeskKeys), Arc::new(agents));
    room.allow_sender("bob", "ada").unwrap();
    (room, work)
}

fn request(room: &Room, id: &str) -> Request {
    room.exchange_pair("ada~bob")
        .and_then(|pair| pair.requests.into_iter().find(|r| r.id == id))
        .expect("the request is saved")
}

/// Ada's handoff to Bob, its turn running in his work thread and held there,
/// with both main conversations up.
async fn handoff_at_work(name: &str) -> (Arc<Room>, Arc<Semaphore>, String) {
    let log = scratch(name);
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let (room, work) = gated_room(log, &gate);
    room.start("ada").await.unwrap();
    room.start("bob").await.unwrap();
    let handoff = send(&room, "handoff").await;
    until("bob's handoff turn is running", || {
        work.prompts().len() == 1
    })
    .await;
    (room, gate, handoff)
}

/// Turns `whose` main conversation over to its next chapter the way the
/// person's line after a closed chapter does: the old session goes, and a
/// new one hears the line.
async fn next_chapter(room: &Arc<Room>, whose: &str) {
    let old = room.session(whose).unwrap();
    room.start_fresh_chapter(whose, ChapterClose::User)
        .await
        .unwrap();
    room.prompt(whose, "next", None, None).await.unwrap();
    until("the next chapter's session is up", || {
        room.session(whose)
            .is_ok_and(|now| !Arc::ptr_eq(&now, &old))
    })
    .await;
}

/// Either teammate's main conversation turning over to its next chapter
/// leaves Ada's handoff to Bob running: it ends with its result.
#[tokio::test]
async fn a_handoff_outlives_either_teammates_chapter_turning_over() {
    for whose in ["ada", "bob"] {
        let (room, gate, handoff) = handoff_at_work(&format!("lasting-chapter-{whose}")).await;

        next_chapter(&room, whose).await;
        assert_eq!(request(&room, &handoff).phase(), Phase::Running, "{whose}");
        gate.add_permits(2);
        done(&room, &handoff).await;
        let result = request(&room, &handoff);
        assert_eq!(
            (result.failed(), result.reply.as_str()),
            (false, "Built it."),
            "{whose}"
        );
    }
}

/// What a chapter's turnover leaves standing, every explicit end still ends:
/// Stop exchange, either teammate stopped, a policy change on either, and the
/// room-wide one a sign-out or a server removed makes.
#[tokio::test]
async fn each_explicit_end_still_ends_a_request_a_chapter_left_standing() {
    for end in [
        "stop-exchange",
        "ada-stopped",
        "bob-stopped",
        "ada-reattached",
        "bob-reattached",
        "room-invalidated",
    ] {
        let (room, gate, handoff) = handoff_at_work(&format!("lasting-ends-{end}")).await;
        next_chapter(&room, "bob").await;
        match end {
            "stop-exchange" => room.stop_exchange("ada", "bob").unwrap(),
            "ada-stopped" => room.stop("ada").unwrap(),
            "bob-stopped" => room.stop("bob").unwrap(),
            "ada-reattached" => room.reattach("ada").await.unwrap(),
            "bob-reattached" => room.reattach("bob").await.unwrap(),
            _ => {
                room.invalidate_all().unwrap();
                room.reattach_all().await.unwrap();
            }
        }
        until("the request is stopped", || {
            request(&room, &handoff).phase() == Phase::Stopped
        })
        .await;
        assert!(request(&room, &handoff).failed(), "{end}");
        gate.add_permits(10);
    }
}

/// What changes Bob's main conversation and not what he may do rebuilds it
/// the way a chapter's swap does: his computer joining once it has
/// downloaded, a computer made again between turns, or a sign-in to an MCP
/// server. The handoff his thread is working on goes on to its result.
#[tokio::test]
async fn a_request_outlives_its_recipients_session_rebuilt_for_a_computer_or_a_sign_in() {
    use crate::contract::PersonaComputer;
    for rebuilt_by in ["computer-joins", "computer-made-again", "mcp-sign-in"] {
        let log = scratch(&format!("lasting-rebuilt-{rebuilt_by}"));
        enrol(&log, &persona("ada"));
        let mut bob = persona("bob");
        bob.computer = Some(PersonaComputer {
            cpus: None,
            enabled: true,
            image: None,
            memory: None,
            pids: None,
            mounts: None,
        });
        enrol(&log, &bob);
        let gate = Arc::new(Semaphore::new(0));
        let work = Fake::new(Scripted::new(turn("built", "Built it.")).gated(gate.clone()));
        // No runtime to be found: each start goes on without the computer.
        let nowhere = tempfile::tempdir().unwrap();
        let room = Room::with_agents_and_computers(
            log,
            Arc::new(DeskKeys),
            Arc::new(Builds {
                agents: HashMap::from([
                    ("ada", vec![main("ada")]),
                    ("bob", vec![main("bob"), work.clone(), main("bob")]),
                ]),
                built: Mutex::default(),
            }),
            crate::computer::Computer::with_path(nowhere.path()),
        );
        room.allow_sender("bob", "ada").unwrap();
        room.start("bob").await.unwrap();
        let handoff = send(&room, "handoff").await;
        until("bob's handoff turn is running", || {
            work.prompts().len() == 1
        })
        .await;
        let old = room.session("bob").unwrap();

        match rebuilt_by {
            "computer-joins" => {
                // The start that found no runtime left its failure; this
                // download is a later one that landed.
                room.computer_links.end_start("bob");
                room.computer_links.downloaded("bob");
                room.attach_computer_when_idle("bob");
            }
            "computer-made-again" => room.computer_restart("bob").await.unwrap(),
            _ => room.signed_in_to_mcp().await,
        }
        until("bob's main conversation is rebuilt", || {
            room.session("bob")
                .is_ok_and(|now| !Arc::ptr_eq(&now, &old))
                && room.info("bob").state == crate::contract::SessionState::Ready
        })
        .await;
        gate.add_permits(2);
        done(&room, &handoff).await;
        let result = request(&room, &handoff);
        assert_eq!(
            (result.failed(), result.reply.as_str()),
            (false, "Built it."),
            "{rebuilt_by}"
        );
    }
}

/// An MCP sign-in while Bob's main conversation is mid-turn rebuilds that
/// session only once its turn is over: the turn is not cut, and the session
/// after it is a new one.
#[tokio::test]
async fn an_mcp_sign_in_rebuilds_a_main_conversation_only_once_its_turn_is_over() {
    let log = scratch("lasting-sign-in-mid-turn");
    enrol(&log, &persona("bob"));
    let gate = Arc::new(Semaphore::new(0));
    let busy = Fake::new(
        Scripted::new(turn("blue", "Blue."))
            .on_cancel(vec![Update::Turn {
                stop_reason: "cancelled".into(),
                usage: None,
            }])
            .gated(gate.clone()),
    );
    let room = Room::with_agents(
        log,
        Arc::new(DeskKeys),
        Arc::new(Builds {
            agents: HashMap::from([("bob", vec![busy.clone(), main("bob")])]),
            built: Mutex::default(),
        }),
    );
    room.start("bob").await.unwrap();
    room.prompt("bob", "which colour?", None, None)
        .await
        .unwrap();
    until("bob's turn is running", || room.mid_turn("bob")).await;
    let old = room.session("bob").unwrap();

    room.signed_in_to_mcp().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        Arc::ptr_eq(&old, &room.session("bob").unwrap()),
        "the session mid-turn is kept"
    );
    assert_eq!(busy.cancel_count(), 0, "its turn is not cut");
    gate.add_permits(10);
    until("bob's main conversation is rebuilt after the turn", || {
        room.session("bob")
            .is_ok_and(|now| !Arc::ptr_eq(&now, &old))
            && room.info("bob").state == crate::contract::SessionState::Ready
    })
    .await;
    assert!(
        room.tape("bob").iter().any(|v| v["text"] == "Blue."),
        "the turn said its piece"
    );
}
