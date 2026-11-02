//! How a request ends: at the door of a turn that refuses it, by a stop that
//! lands while its answer is on its way, and as an old record read it.
use super::questions::request;
use super::*;
use crate::contract::{DeliveryFrom, ThreadId};
use crate::session::Wired;
use crate::session::turns::{Begin, Occupant, RequestLine};

/// A session of `persona` that no longer answers: its capability ended, as a
/// chapter's swap or a revocation ends it while a line waits in its queue.
async fn replaced(room: &Arc<Room>, persona: &str) -> Arc<crate::session::Session> {
    room.start(persona).await.unwrap();
    let old = room.session(persona).unwrap();
    old.capability.revoke();
    assert!(!old.capability.is_current());
    old
}

/// A question refused at the door of its turn, because the session it was
/// handed to has been replaced, is not left running as begun: its asker is
/// told it was not answered.
#[tokio::test]
async fn a_question_refused_at_the_door_ends_instead_of_staying_begun() {
    let (room, _) = setup("door-refuses-a-question");
    let old = replaced(&room, "bob").await;
    let mut asked = saved_request("refused", Intent::Ask, Phase::Running);
    asked.request_counted = true;
    seed(&room, asked, 1, false);
    let mut wired = Wired::words("saved work");
    wired.request = Some(RequestLine {
        request: "refused".into(),
        answer: None,
    });
    assert!(matches!(old.begin(&room, wired).await, Begin::Stop));
    let refused = request(&room, "ada~bob", "refused");
    assert!(
        !(refused.phase() == Phase::Running && refused.begun()),
        "a refused question stays running and begun forever"
    );
    assert_eq!(refused.phase(), Phase::Reply);
    assert!(refused.failed());
}

/// A result refused at the door of its turn was not heard, so it is still
/// there to be heard by the session that replaces the refusing one.
#[tokio::test]
async fn a_result_refused_at_the_door_is_not_consumed() {
    let (room, _) = setup("door-refuses-a-result");
    let old = replaced(&room, "ada").await;
    let mut answered = saved_request("unheard", Intent::Ask, Phase::Reply);
    answered.reply = "the answer".into();
    answered.state = State::Reply {
        counted: true,
        sending: false,
        heard: false,
        failed: false,
    };
    answered.request_counted = true;
    seed(&room, answered, 2, false);
    let mut wired = Wired::words("the answer");
    wired.from = Some(DeliveryFrom::new(
        &ThreadId::pair("ada~bob"),
        Some("unheard".into()),
    ));
    assert!(matches!(old.begin(&room, wired).await, Begin::Stop));
    assert!(
        !request(&room, "ada~bob", "unheard").heard(),
        "the refused result is marked heard"
    );
    assert!(room.begin_exchange_result("unheard"));
}

/// A stop that lands while a request's answer is on its way to the sender
/// does not also send the sender a "stopped" notice.
#[tokio::test]
async fn a_stop_while_the_answer_is_on_its_way_does_not_also_tell_the_sender_it_stopped() {
    let (room, agents) = controlled("stop-during-delivery");
    agents.drivers["ada"].startup.forget_permits(100);
    room.allow_sender("bob", "ada").unwrap();
    let id = send(&room, "ask").await;
    agents.drivers["bob"].updates.add_permits(2);
    until("the answer is counted and on its way", || {
        room.exchange_pair("ada~bob").is_some_and(|p| {
            p.requests.iter().any(|r| {
                r.id == id
                    && r.phase() == Phase::Reply
                    && matches!(r.state, State::Reply { counted: true, .. })
            })
        })
    })
    .await;
    room.stop_exchange("ada", "bob").unwrap();
    agents.drivers["ada"].startup.add_permits(100);
    agents.drivers["ada"].updates.add_permits(10);
    until("no exchange worker is left", || {
        lock(&room.exchange_workers).is_empty()
    })
    .await;
    let tape = room.tape("ada");
    let answered = tape
        .iter()
        .any(|v| v["kind"] == "delivery" && v["cause"]["requestId"] == id.as_str());
    let stopped = tape
        .iter()
        .any(|v| v["id"] == format!("exchange-ended:{id}"));
    assert!(
        !(answered && stopped),
        "the sender heard the answer and that it was stopped"
    );
}

/// Records written before a request's end was one thing still load: a
/// stopped one that owed its thread, one its desk was told of, a result
/// heard inline, and a begun one.
#[test]
fn requests_saved_with_the_old_end_flags_still_load() {
    let base = |extra: serde_json::Value| {
        let mut record = json!({
            "id": "r", "from": "ada", "to": "bob", "message": "m",
            "intent": "ask", "phase": "stopped", "reply": "stopped", "failed": true,
        });
        for (key, value) in extra.as_object().unwrap() {
            record[key] = value.clone();
        }
        let request: Request = serde_json::from_value(record).expect("an old record loads");
        // And what is saved is read back the same.
        let again: Request =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert_eq!(again.phase(), request.phase());
        request
    };
    let owing = base(json!({"owed_to_thread": true, "reply_thread": "side"}));
    assert!(Room::owes_answer(&owing));
    let told = base(json!({"owedToThread": false, "returned": true}));
    assert!(!Room::owes_answer(&told));
    let person = base(json!({"stopped_by_person": true, "released": true}));
    assert_eq!(person.phase(), Phase::Stopped);
    assert!(!person.stands());
    let heard = base(json!({"phase": "done", "failed": false, "result_consumed": true}));
    assert!(!Room::exchange_pending(&heard));
    let unheard = base(json!({"phase": "done", "failed": false}));
    assert!(Room::exchange_pending(&unheard));
    let begun = base(json!({"phase": "running", "failed": false, "started": true}));
    assert!(begun.begun());
}

/// A request moves only as its state allows: what has ended cannot be
/// finished, and a result that was heard cannot be stopped.
#[test]
fn a_request_refuses_a_move_its_state_does_not_allow() {
    let stop = Stop {
        by_person: false,
        owed: Owed::Nobody,
        answering: false,
    };
    let mut stopped = saved_request("s", Intent::Ask, Phase::Stopped);
    assert!(
        stopped
            .advance(Event::Finished {
                waiting: false,
                failed: false
            })
            .is_err()
    );
    assert!(stopped.advance(Event::Begun).is_err());
    let mut heard = saved_request("h", Intent::Ask, Phase::Done);
    heard.state = State::Done {
        heard: true,
        failed: false,
    };
    assert!(heard.advance(Event::Ended(stop)).is_err());
    let mut queued = saved_request("q", Intent::Ask, Phase::Queued);
    assert!(queued.advance(Event::Begun).is_err(), "never admitted");
    queued.advance(Event::Admitted).unwrap();
    queued.advance(Event::Begun).unwrap();
    assert!(queued.begun());
    // A stop that lands as its answer is on its way owes nothing if it lands.
    let mut replying = saved_request("r", Intent::Ask, Phase::Reply);
    replying.advance(Event::Sending).unwrap();
    replying
        .advance(Event::Ended(Stop {
            owed: Owed::Thread,
            ..stop
        }))
        .unwrap();
    assert!(replying.answering());
    replying.advance(Event::Delivered).unwrap();
    assert_eq!(replying.owed(), Owed::Nobody);
}
