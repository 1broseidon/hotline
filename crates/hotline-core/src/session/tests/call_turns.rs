//! What becomes of the person's words on a Turn by turn call once the
//! teammate has spoken its answer: they are a turn of their own in the
//! thread's session, queued behind a turn still open, and the call waits for
//! that turn, never for the earlier one, so the person is acknowledged, hears
//! the presence of the work, and is answered (`Calls::respond`,
//! `Call::wait_for_turn`).

use super::*;
use crate::contract::VoiceInputMode;
use crate::voice::tests::Fake as Voice;
use hotline_contract::VoiceState;

const ACK: &str = "Comparing Dougie Hamilton next.";
const FIRST: &str = "Tavares leads on goals.";
const SECOND: &str = "Hamilton has more assists.";

/// A reply said on the call, as an agent streams it and then writes it.
fn answer(id: &str, said: &str) -> Vec<Update> {
    let text = format!("<spoken>{said}</spoken>");
    vec![piece(id, &text), whole(id, &text)]
}

/// A piece of a reply as an agent streams it.
fn piece(id: &str, text: &str) -> Update {
    Update::Delta {
        kind: MessageKind::Agent,
        message_id: id.into(),
        text: text.into(),
    }
}

/// A reply as the agent writes it once it is whole.
fn whole(id: &str, text: &str) -> Update {
    Update::Message {
        kind: MessageKind::Agent,
        id: id.into(),
        text: text.into(),
    }
}

fn end() -> Update {
    Update::Turn {
        stop_reason: "end_turn".into(),
        usage: None,
    }
}

/// A call's lines as the person hears them, read off its events, with the
/// states it says and how many clips of sound it sent.
struct Heard {
    events: tokio::sync::broadcast::Receiver<hotline_contract::VoiceEvent>,
    lines: Vec<String>,
    /// Each state the call said, and whether it said it listens.
    states: Vec<(VoiceState, bool)>,
    sounds: usize,
}

impl Heard {
    fn read(&mut self) {
        use hotline_contract::VoiceEvent;
        while let Ok(event) = self.events.try_recv() {
            match event {
                VoiceEvent::Said { text, .. } => self.lines.push(text),
                VoiceEvent::State {
                    state, listening, ..
                } => self.states.push((state, listening)),
                VoiceEvent::Clip { data, .. } if !data.is_empty() => self.sounds += 1,
                _ => {}
            }
        }
    }

    /// How many times `line` was said on the call, whole.
    fn said(&mut self, line: &str) -> usize {
        self.read();
        self.lines.iter().filter(|said| *said == line).count()
    }

    /// Whether the call last said it takes what the person says: it
    /// listens, or thinks while the teammate's turn works and says it
    /// listens, which the window shows as Listening once nothing plays.
    fn listens(&mut self) -> bool {
        self.read();
        matches!(
            self.states.last(),
            Some((VoiceState::Listening, _) | (VoiceState::Thinking, true))
        )
    }

    /// How many clips of sound the call has sent.
    fn sounds(&mut self) -> usize {
        self.read();
        self.sounds
    }
}

/// Ada on `turns`, one update per permit of `gate`, a Turn by turn call to
/// her that takes typed lines, what the call says and its voice, which has a
/// line ready for any acknowledgement asked of it.
async fn call_on(
    name: &str,
    turns: Vec<Vec<Update>>,
    gate: &Arc<Semaphore>,
) -> (
    Arc<Room>,
    Arc<Fake>,
    Arc<crate::voice::Calls>,
    Heard,
    String,
) {
    let fake = Fake::new(Scripted::turns(turns).gated(gate.clone()));
    let room = room(name, fake.clone());
    let vault = Arc::new(
        crate::vault::Vault::open_with_store(
            room.log.root(),
            room.log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let voice = Arc::new(Voice::default());
    *lock(&voice.ack) = Ok(ACK.into());
    let calls = crate::voice::Calls::new(
        room.log.clone(),
        crate::voice::room::RoomSide::new(Arc::downgrade(&room), vault),
        Some(crate::voice::tests::with_fake(voice)),
    );
    room.set_voice(&calls);
    room.start("ada").await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    calls
        .start_with_input(
            &id,
            Some("ada".into()),
            false,
            VoiceInputMode::Text,
            Arc::new(crate::wire::tests::CoreHandle { room: room.clone() }),
        )
        .unwrap();
    let (_, events) = calls.subscribe(&id).unwrap();
    until("the call's agent is up", || {
        room.sides.get(&id).is_some_and(|side| !side.working())
    })
    .await;
    let heard = Heard {
        events,
        lines: Vec::new(),
        states: Vec::new(),
        sounds: 0,
    };
    (room, fake, calls, heard, id)
}

/// Acknowledge while the teammate thinks, from now on.
fn acknowledging(room: &Room) {
    room.log
        .append(
            &StreamId::Room,
            &crate::room::room_event(
                "setting",
                json!({"id": "voice", "value": {"acknowledge": true}}),
            ),
        )
        .unwrap();
}

/// Whether the call is waiting on the teammate, and for which of the
/// person's turns.
fn waiting(calls: &crate::voice::Calls, id: &str) -> (VoiceState, Option<u32>) {
    calls
        .test_change(id, |call| Ok((call.state, call.answering)))
        .unwrap()
}

/// The person's words have been handed to the teammate's session.
async fn handed(calls: &crate::voice::Calls, id: &str) {
    until("the words are handed over", || {
        calls
            .test_change(id, |call| Ok(!call.utterance_pending))
            .unwrap()
    })
    .await;
}

/// The thread's next turn has begun, so the one before it is over, and the
/// call has had time to hear so.
async fn next_turn_began(room: &Room, id: &str, turns: usize, fake: &Fake) {
    until("the next turn has begun", || {
        lock(&fake.driver.prompts).len() >= turns
            && room.sides.get(id).is_some_and(|side| side.working())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// The teammate has spoken its answer and its turn stays open (a written
/// half, a last tool call, its own bookkeeping) when the person speaks
/// again. Their words wait behind that turn as its next one; they are
/// acknowledged at once; and the end of the earlier turn does not end the
/// call's wait for theirs: it still thinks, so its presence sound and its
/// line on the work cover the new turn, until that turn answers.
#[tokio::test]
async fn words_said_while_the_answered_turn_is_still_open_wait_for_it_and_are_answered() {
    let gate = Arc::new(Semaphore::new(0));
    let mut first = answer("a1", FIRST);
    first.push(end());
    let mut second = answer("a2", SECOND);
    second.push(end());
    let (room, fake, calls, mut heard, id) =
        call_on("call-words-behind-open-turn", vec![first, second], &gate).await;

    calls
        .text(&id, 1, "Compare Tavares and Marner.", None)
        .unwrap();
    gate.add_permits(2);
    until("the answer is said", || heard.said(FIRST) == 1).await;
    assert_eq!(waiting(&calls, &id).1, Some(1), "the turn is still open");

    acknowledging(&room);
    calls
        .text(&id, 2, "Do the same comparison for Dougie Hamilton.", None)
        .unwrap();
    handed(&calls, &id).await;
    until("the person hears their words were taken", || {
        heard.said(ACK) == 1
    })
    .await;
    assert_eq!(
        lock(&fake.driver.prompts).len(),
        1,
        "the words wait behind the open turn, never joining it"
    );

    gate.add_permits(1);
    next_turn_began(&room, &id, 2, &fake).await;
    let prompts = lock(&fake.driver.prompts).clone();
    assert!(prompts[1].contains("Dougie Hamilton"), "{prompts:?}");
    assert_eq!(
        waiting(&calls, &id),
        (VoiceState::Thinking, Some(2)),
        "the earlier turn's end is not the end of the wait for these words"
    );

    gate.add_permits(3);
    until("the new turn is answered", || heard.said(SECOND) == 1).await;
    until("and the call listens once it is over", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    calls.end(&id).unwrap();
}

/// The person speaks just as the teammate's turn ends: the session's turn is
/// over while the call has not yet heard so. The words start a new turn at
/// once, are acknowledged although the call still counted the teammate as
/// busy, and the late news of the earlier turn's end does not end the wait.
#[tokio::test]
async fn words_said_as_the_answered_turn_ends_start_a_turn_the_call_waits_for() {
    let gate = Arc::new(Semaphore::new(0));
    let mut first = answer("a1", FIRST);
    first.push(end());
    let mut second = answer("a2", SECOND);
    second.push(end());
    let (room, fake, calls, mut heard, id) =
        call_on("call-words-as-turn-ends", vec![first, second], &gate).await;

    calls
        .text(&id, 1, "Compare Tavares and Marner.", None)
        .unwrap();
    gate.add_permits(2);
    until("the answer is said", || heard.said(FIRST) == 1).await;

    // The words reach the call while the earlier turn is open, and reach the
    // session only once it is over.
    acknowledging(&room);
    let admission = room.sides.waking.lock().await;
    calls
        .text(&id, 2, "Do the same comparison for Dougie Hamilton.", None)
        .unwrap();
    gate.add_permits(1);
    until("the earlier turn is over in the session", || {
        room.sides.get(&id).is_some_and(|side| !side.working())
    })
    .await;
    assert_eq!(
        waiting(&calls, &id).1,
        Some(1),
        "the call has not heard yet"
    );
    drop(admission);

    next_turn_began(&room, &id, 2, &fake).await;
    assert_eq!(
        waiting(&calls, &id),
        (VoiceState::Thinking, Some(2)),
        "the late end of the earlier turn is not the end of this one"
    );
    until("the person hears their words were taken", || {
        heard.said(ACK) == 1
    })
    .await;
    gate.add_permits(3);
    until("the new turn is answered", || heard.said(SECOND) == 1).await;
    until("the call listens once it is over", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    calls.end(&id).unwrap();
}

/// A turn that ends having said nothing for the person's latest words, as a
/// failed or silent turn does, says so on the call: the person is never left
/// with a call that only went back to listening.
#[tokio::test]
async fn a_turn_that_ends_saying_nothing_says_so() {
    let gate = Arc::new(Semaphore::new(0));
    let (_room, _fake, calls, mut heard, id) =
        call_on("call-turn-says-nothing", vec![vec![end()]], &gate).await;
    calls.text(&id, 1, "Anything new?", None).unwrap();
    gate.add_permits(1);
    until("the person hears no answer came", || {
        heard.said(hotline_voice::NOTHING_SAID) == 1
    })
    .await;
    until("the call listens", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    calls.end(&id).unwrap();
}

/// The person's Stop in the thread drops the words waiting behind the turn it
/// stops: the call stops waiting for them, and says nothing came of them.
#[tokio::test]
async fn words_dropped_by_a_stop_in_the_thread_end_the_wait_for_them() {
    let gate = Arc::new(Semaphore::new(0));
    let mut first = answer("a1", FIRST);
    first.push(end());
    let (room, fake, calls, mut heard, id) =
        call_on("call-words-stopped", vec![first], &gate).await;
    calls
        .text(&id, 1, "Compare Tavares and Marner.", None)
        .unwrap();
    gate.add_permits(2);
    until("the answer is said", || heard.said(FIRST) == 1).await;
    calls
        .text(&id, 2, "Do the same comparison for Dougie Hamilton.", None)
        .unwrap();
    handed(&calls, &id).await;
    room.cancel_side(&id).unwrap();
    until("the call stops waiting", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    assert_eq!(lock(&fake.driver.prompts).len(), 1, "the words never ran");
    until("the person hears nothing came of them", || {
        heard.said(hotline_voice::NOTHING_SAID) == 1
    })
    .await;
    calls.end(&id).unwrap();
}

/// The teammate has said its answer while its reply and turn are still open:
/// it goes on writing the written half. The call stops speaking as soon as
/// nothing more of the answer is in flight, and says it listens, so the
/// window takes the person's words once the answer has played, not once the
/// reply closes. Those words wait behind the open turn, are acknowledged and
/// answered, and the rest of the reply is not said over them.
#[tokio::test]
async fn the_call_listens_once_the_answer_is_said_though_its_reply_is_still_open() {
    let gate = Arc::new(Semaphore::new(0));
    let spoken = format!("<spoken>{FIRST}</spoken>");
    let written = "\n<written>Tavares has 41 goals to Marner's 27.</written>";
    let first = vec![
        piece("a1", &spoken),
        piece("a1", written),
        whole("a1", &format!("{spoken}{written}")),
        end(),
    ];
    let mut second = answer("a2", SECOND);
    second.push(end());
    let (room, fake, calls, mut heard, id) =
        call_on("call-listens-after-answer", vec![first, second], &gate).await;

    calls
        .text(&id, 1, "Compare Tavares and Marner.", None)
        .unwrap();
    gate.add_permits(1);
    until("the answer is said", || heard.said(FIRST) == 1).await;
    until("the call listens", || heard.listens()).await;
    assert_eq!(
        waiting(&calls, &id),
        (VoiceState::Thinking, Some(1)),
        "the turn is still open"
    );
    assert!(
        calls
            .test_change(&id, |call| Ok(call.streams.contains_key("a1")))
            .unwrap(),
        "and so is the reply"
    );

    acknowledging(&room);
    calls
        .text(&id, 2, "Do the same comparison for Dougie Hamilton.", None)
        .unwrap();
    handed(&calls, &id).await;
    until("the person hears their words were taken", || {
        heard.said(ACK) == 1
    })
    .await;
    assert_eq!(
        lock(&fake.driver.prompts).len(),
        1,
        "the words wait behind the open turn"
    );

    gate.add_permits(3);
    next_turn_began(&room, &id, 2, &fake).await;
    assert_eq!(waiting(&calls, &id), (VoiceState::Thinking, Some(2)));
    gate.add_permits(3);
    until("the new turn is answered", || heard.said(SECOND) == 1).await;
    until("the call listens once it is over", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    assert_eq!(
        heard.said(FIRST),
        1,
        "nothing of the first reply came again"
    );
    calls.end(&id).unwrap();
}

/// More of the same answer that comes after the call went back to listening,
/// with the person silent, is said as before: the call speaks again, sends
/// its sound, and listens once that is sent too.
#[tokio::test]
async fn more_of_the_answer_after_the_call_listens_is_still_said() {
    let gate = Arc::new(Semaphore::new(0));
    let both = format!("{FIRST} {SECOND}");
    let first = vec![
        piece("a1", &format!("{FIRST} ")),
        piece("a1", &format!("{SECOND} ")),
        whole("a1", &both),
        end(),
    ];
    let (_room, _fake, calls, mut heard, id) =
        call_on("call-more-after-listening", vec![first], &gate).await;

    calls
        .text(&id, 1, "Compare Tavares and Hamilton.", None)
        .unwrap();
    gate.add_permits(1);
    until("the answer's first sentence is said", || {
        heard.said(FIRST) == 1
    })
    .await;
    until("the call listens", || heard.listens()).await;
    let sounds = heard.sounds();
    let listened = heard.states.len();

    gate.add_permits(1);
    until("more of the answer is said", || heard.said(&both) == 1).await;
    until("and sent to be heard", || heard.sounds() > sounds).await;
    assert!(
        heard.states[listened..].contains(&(VoiceState::Speaking, false)),
        "the call speaks again: {:?}",
        heard.states
    );
    until("the call listens again", || heard.listens()).await;

    gate.add_permits(2);
    until("the call listens once the turn is over", || {
        waiting(&calls, &id) == (VoiceState::Listening, None)
    })
    .await;
    calls.end(&id).unwrap();
}

/// More of the answer that comes once the person has spoken again is never
/// said over them: their words stop the reply, as speaking over it does, and
/// they are the turn the call waits for.
#[tokio::test]
async fn more_of_the_answer_after_the_person_spoke_is_not_said_over_them() {
    let gate = Arc::new(Semaphore::new(0));
    const LATE: &str = "Marner trails him by fourteen.";
    let first = vec![
        piece("a1", &format!("{FIRST} ")),
        piece("a1", &format!("{LATE} ")),
        whole("a1", &format!("{FIRST} {LATE}")),
        end(),
    ];
    let mut second = answer("a2", SECOND);
    second.push(end());
    let (room, fake, calls, mut heard, id) =
        call_on("call-more-after-person", vec![first, second], &gate).await;

    calls
        .text(&id, 1, "Compare Tavares and Marner.", None)
        .unwrap();
    gate.add_permits(1);
    until("the answer's first sentence is said", || {
        heard.said(FIRST) == 1
    })
    .await;
    until("the call listens", || heard.listens()).await;
    calls
        .text(&id, 2, "Do the same comparison for Dougie Hamilton.", None)
        .unwrap();
    handed(&calls, &id).await;
    let sounds = heard.sounds();

    gate.add_permits(3);
    next_turn_began(&room, &id, 2, &fake).await;
    assert!(
        !heard.lines.iter().any(|line| line.contains(LATE)),
        "{:?}",
        heard.lines
    );
    assert_eq!(heard.sounds(), sounds, "no sound of the rest of the reply");
    assert_eq!(waiting(&calls, &id), (VoiceState::Thinking, Some(2)));
    gate.add_permits(3);
    until("the person's words are answered", || {
        heard.said(SECOND) == 1
    })
    .await;
    calls.end(&id).unwrap();
}
