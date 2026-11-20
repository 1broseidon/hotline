//! A Turn by turn call to a teammate starts its thread's agent the moment it
//! is placed, so the first thing said finds it up (`Room::prepare_call`).

use super::*;
use crate::contract::{VoiceCallMode, VoiceInputMode};
use crate::thread::ThreadState;
use hotline_voice::CallRequest;

mod inbox;

/// A room on `driver` with Ada's main conversation running and calls wired
/// to it, and the call fakes' speech.
async fn calling(name: &str, driver: Scripted) -> (Arc<Room>, Arc<Fake>, Arc<crate::voice::Calls>) {
    calling_with(name, driver, None).await
}

async fn calling_with(
    name: &str,
    driver: Scripted,
    realtime: Option<Arc<dyn hotline_voice::realtime::Realtime>>,
) -> (Arc<Room>, Arc<Fake>, Arc<crate::voice::Calls>) {
    let fake = Fake::new(driver);
    let room = room(name, fake.clone());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            room.log.root(),
            room.log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let mut services =
        crate::voice::tests::with_fake(Arc::new(crate::voice::tests::Fake::default()));
    services.realtime = realtime;
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        crate::voice::room::RoomSide::new(Arc::downgrade(&room), vault),
        Some(services),
    );
    room.set_voice(&calls);
    room.start("ada").await.unwrap();
    (room, fake, calls)
}

fn handle(room: &Arc<Room>) -> Arc<dyn crate::wire::RoomHandle> {
    Arc::new(crate::wire::tests::CoreHandle { room: room.clone() })
}

/// A Turn by turn call to Ada that takes typed lines, by its id.
fn call_ada(room: &Arc<Room>, calls: &Arc<crate::voice::Calls>) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    calls
        .start_with_input(
            &id,
            Some("ada".into()),
            false,
            VoiceInputMode::Text,
            handle(room),
        )
        .unwrap();
    id
}

/// Agents built so far, the main conversation's included.
fn built(fake: &Fake) -> usize {
    lock(&fake.preambles).len()
}

/// The call's thread has its agent, up and running no turn.
fn ready(room: &Room, id: &str) -> bool {
    room.sides.get(id).is_some_and(|side| !side.working())
}

#[tokio::test]
async fn a_call_starts_its_threads_agent_before_anything_is_said_and_says_nothing_to_it() {
    let (room, fake, calls) = calling("warm-call-ready", Scripted::new(spoken_turn())).await;
    let home = built(&fake);
    let id = call_ada(&room, &calls);
    let thread = ThreadId::call(&id);
    until("the call's agent is up", || ready(&room, &id)).await;
    assert_eq!(built(&fake), home + 1, "one agent, for the call's thread");
    assert_eq!(
        fake.prompts(),
        Vec::<String>::new(),
        "nothing is said to it"
    );
    assert_eq!(
        room.working_calls("ada"),
        Vec::<String>::new(),
        "an agent that is up with nothing to do runs no turn"
    );
    let link = room.link_of(&thread).unwrap();
    assert_eq!(link.state, ThreadState::Live);
    assert_eq!(link.title, "Call");

    calls
        .text(&id, 1, "Is the harbour plan ready?", None)
        .unwrap();
    let said = heard(&fake.driver.prompts, 1).await;
    assert!(said[0].contains("Is the harbour plan ready?"), "{said:?}");
    assert_eq!(
        built(&fake),
        home + 1,
        "the first line is the agent's first turn"
    );
    calls.end(&id).unwrap();
}

#[tokio::test]
async fn a_line_said_while_the_agent_is_starting_waits_for_that_start() {
    let gate = Arc::new(Semaphore::new(1));
    let driver = Scripted::new(spoken_turn()).slow_to_start(gate.clone());
    let (room, fake, calls) = calling("warm-call-joins", driver).await;
    let home = built(&fake);
    let id = call_ada(&room, &calls);
    until("the call's agent is being built", || {
        built(&fake) == home + 1
    })
    .await;
    calls
        .text(&id, 1, "Is the harbour plan ready?", None)
        .unwrap();
    until("the line waits for the start", || {
        room.working_calls("ada") == [id.clone()]
    })
    .await;
    assert_eq!(fake.prompts(), Vec::<String>::new());
    gate.add_permits(1);
    let said = heard(&fake.driver.prompts, 1).await;
    assert!(said[0].contains("Is the harbour plan ready?"), "{said:?}");
    assert_eq!(built(&fake), home + 1, "one start, not two");
    calls.end(&id).unwrap();
}

/// What a call that nobody spoke on leaves is what it left before calls
/// started agents: its link as `call_began` wrote it, nothing in the thread
/// but the disconnection, and no agent.
#[tokio::test]
async fn hanging_up_with_nothing_said_lets_the_agent_go_and_leaves_the_thread_as_it_was() {
    let (room, fake, calls) = calling("warm-call-unused", Scripted::new(spoken_turn())).await;
    let id = call_ada(&room, &calls);
    let thread = ThreadId::call(&id);
    let before = room.link_of(&thread).unwrap();
    until("the call's agent is up", || ready(&room, &id)).await;
    let cancels = fake.cancel_count();
    calls.end(&id).unwrap();
    until("the agent is let go of", || room.sides.get(&id).is_none()).await;
    assert!(fake.cancel_count() > cancels, "its driver is stopped");
    assert_eq!(room.link_of(&thread).unwrap(), before);
    let kinds: Vec<Value> = room
        .log
        .load(&StreamId::of_thread(&thread).unwrap())
        .into_iter()
        .map(|event| event["kind"].clone())
        .filter(|kind| kind != "link")
        .collect();
    assert_eq!(kinds, [json!("notice")], "only 'Voice disconnected'");
    assert_eq!(fake.prompts(), Vec::<String>::new());
}

#[tokio::test]
async fn hanging_up_while_the_agent_is_still_starting_lets_it_go_once_it_is_up() {
    let gate = Arc::new(Semaphore::new(1));
    let driver = Scripted::new(spoken_turn()).slow_to_start(gate.clone());
    let (room, fake, calls) = calling("warm-call-unused-starting", driver).await;
    let home = built(&fake);
    let id = call_ada(&room, &calls);
    let thread = ThreadId::call(&id);
    let before = room.link_of(&thread).unwrap();
    until("the call's agent is being built", || {
        built(&fake) == home + 1
    })
    .await;
    calls.end(&id).unwrap();
    until("the thread is put back", || room.sides.get(&id).is_none()).await;
    let cancels = fake.cancel_count();
    gate.add_permits(1);
    until("the agent that came up is stopped", || {
        fake.cancel_count() > cancels
    })
    .await;
    assert!(room.sides.get(&id).is_none());
    assert_eq!(room.link_of(&thread).unwrap(), before);
    assert_eq!(fake.prompts(), Vec::<String>::new());
}

/// A start that fails ahead of the call says nothing: the first line starts
/// the agent again and meets the failure where it always has.
#[tokio::test]
async fn a_start_that_fails_before_anything_is_said_leaves_no_trace_until_the_first_line() {
    let (room, fake, calls) = calling("warm-call-fails", Scripted::new(spoken_turn())).await;
    fake.fail_starts("the harness is signed out");
    let home = built(&fake);
    let id = call_ada(&room, &calls);
    let thread = ThreadId::call(&id);
    let before = room.link_of(&thread).unwrap();
    until("the start was tried", || built(&fake) == home + 1).await;
    until("the thread is put back", || room.sides.get(&id).is_none()).await;
    assert_eq!(room.link_of(&thread).unwrap(), before);
    let stream = || room.log.load(&StreamId::of_thread(&thread).unwrap());
    assert!(!stream().iter().any(|event| event["kind"] == "notice"));

    calls
        .text(&id, 1, "Is the harbour plan ready?", None)
        .unwrap();
    until("the first line meets the failure", || {
        stream().iter().any(|event| {
            event["kind"] == "notice"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("the harness is signed out"))
        })
    })
    .await;
    assert_eq!(built(&fake), home + 2);
    calls.end(&id).unwrap();
}

/// A call to the desk is the lead's to answer, and a live call talks to its
/// front: neither starts the teammate's agent ahead of anything said.
#[tokio::test]
async fn a_desk_call_and_a_live_call_start_no_agent_ahead_of_the_first_line() {
    let (session, _asked, _provider) = hotline_voice::realtime::RealtimeSession::scripted(64);
    let front: Arc<dyn hotline_voice::realtime::Realtime> =
        Arc::new(Front(Mutex::new(Some(session))));
    let (room, fake, calls) = calling_with(
        "warm-call-desk-live",
        Scripted::new(spoken_turn()),
        Some(front),
    )
    .await;
    room.log
        .append(
            &StreamId::Room,
            &crate::room::room_event("setting", json!({"id": "teamLead", "value": "ada"})),
        )
        .unwrap();
    let home = built(&fake);

    // One call at a time: a newer call ends the one before it.
    let desk = uuid::Uuid::new_v4().to_string();
    calls
        .start_call(
            &desk,
            CallRequest {
                target: None,
                thread: None,
                stream_audio: false,
                input_mode: VoiceInputMode::Text,
                mode: VoiceCallMode::Pipeline,
                mic_rate: None,
            },
            handle(&room),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(built(&fake), home);
    assert!(room.sides.get(&desk).is_none());
    let live = uuid::Uuid::new_v4().to_string();
    calls
        .start_call(
            &live,
            CallRequest {
                target: Some("ada".into()),
                thread: None,
                stream_audio: true,
                input_mode: VoiceInputMode::Audio,
                mode: VoiceCallMode::Front,
                mic_rate: Some(24_000),
            },
            handle(&room),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(built(&fake), home);
    assert!(room.sides.get(&live).is_none());
    calls.end(&live).unwrap();
}

/// A front that plays the one scripted session it was given.
struct Front(Mutex<Option<hotline_voice::realtime::RealtimeSession>>);

#[async_trait]
impl hotline_voice::realtime::Realtime for Front {
    fn id(&self) -> hotline_voice::speech::SpeechId {
        hotline_voice::speech::SpeechId {
            provider_id: "test".into(),
            model_id: "front".into(),
            voice: None,
        }
    }
    fn is_subscription(&self) -> bool {
        true
    }
    async fn open(
        &self,
        _: hotline_voice::realtime::RealtimeConfig,
    ) -> Result<hotline_voice::realtime::RealtimeSession, hotline_voice::speech::SpeechError> {
        lock(&self.0)
            .take()
            .ok_or(hotline_voice::speech::SpeechError::Cancelled)
    }
}
