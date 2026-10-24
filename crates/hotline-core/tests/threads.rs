//! Threads on the wire, through the real core and a disposable model endpoint:
//! the same conversation, read by a client that declared `threads2` and by one
//! that did not.
use crate::common;

use axum::{Router, body::Bytes, routing::post};
use common::Client;
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

const TOKEN: &str = "disposable-threads-test";
const PATIENCE: Duration = Duration::from_secs(20);

/// A model that answers every request with the same words, in one chunk.
fn answer_events() -> Vec<Value> {
    vec![
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{"role":"assistant","content":"On it."},"finish_reason":null}]}),
        json!({"id":"chat_test","object":"chat.completion.chunk","created":1,"model":"vendor/coder","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ]
}

/// A new thread opened over the wire is the same conversation to a client of
/// either shape: the older one is sent the markers and per-kind deltas it
/// always had, the one that said `threads2` is sent links and `ThreadDelta`s,
/// and every `thread.*` verb moves the one thread both are reading.
#[tokio::test(flavor = "multi_thread")]
async fn a_work_thread_is_one_conversation_to_old_and_threads2_clients() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|_body: Bytes| async {
            let body = answer_events()
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            (
                [("Content-Type", "text/event-stream")],
                format!("{body}data: [DONE]\n\n"),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let desk = common::open_desk(root.path()).unwrap();
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());

    let mut now = Client::connect(port, TOKEN).await;
    let mut old = Client::connect(port, TOKEN).await;
    let saved = now
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name":"chat","baseUrl":base,"api":"chat_completions","models":["vendor/coder"]
            }}),
        )
        .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let made = now
        .call(
            "persona.create",
            json!({"draft": {
                "name":"Threads tester","goal":"Answer briefly","cwd":workspace.to_string_lossy()
            }}),
        )
        .await;
    let persona = made["result"]["id"].as_str().unwrap().to_owned();

    // The hello names what this core can do, and the socket that declares it
    // is read differently from then on.
    let hello = now
        .call(
            "client.hello",
            json!({"capabilities":["threads2","somethingNew"]}),
        )
        .await;
    assert_eq!(hello["ok"], true, "{hello}");
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("threads2")),
        "{hello}"
    );

    let now_tape = now.subscribe(json!({"tape":persona})).await;
    let old_tape = old.subscribe(json!({"tape":persona})).await;

    // Open a work thread, and read it by id on one socket and by side id on
    // the other.
    let opened = now
        .call(
            "thread.open",
            json!({"personaId":persona,"text":"Look into the crane"}),
        )
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let summary = &opened["result"];
    assert_eq!(summary["thread"]["kind"], "side");
    assert_eq!(summary["personaId"], persona.as_str());
    assert_eq!(summary["title"], "Look into the crane");
    let key = summary["thread"]["key"].as_str().unwrap().to_owned();
    let thread = json!({"kind":"side","key":key});
    let now_side = now.subscribe(json!({"threadId":thread})).await;
    let old_side = old.subscribe(json!({"side":key})).await;

    // The tape's link, in the shape each client reads. A subscription is
    // answered before it listens, so on a fast machine the link was written
    // in between and arrives in the tape's snapshot instead of live.
    let link = live_or_snapshot(&mut now, now_tape, |event| event["kind"] == "link").await;
    assert_eq!(link["thread"], key.as_str());
    assert_eq!(link["threadKind"], "side");
    let marker = live_or_snapshot(&mut old, old_tape, |event| event["kind"] == "side").await;
    assert_eq!(marker["sideId"], key.as_str());
    assert!(
        old.inbox
            .iter()
            .all(|frame| frame["event"]["kind"] != "link"),
        "an old client is never sent a link"
    );

    // The first turn ends, then another is asked for, with both reading. The
    // model answers at once, so on a fast machine the turn has ended before
    // the side is subscribed to, and arrives in its snapshot instead.
    now.next_where(PATIENCE, |frame| {
        let ended = |event: &Value| event["kind"] == "turn";
        frame["sub"] == now_side
            && (ended(&frame["event"])
                || frame["snapshot"]
                    .as_array()
                    .is_some_and(|events| events.iter().any(ended)))
    })
    .await;
    let said = now
        .call(
            "thread.prompt",
            json!({"thread":thread,"text":"And the hoist?"}),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    let delta = now
        .next_where(PATIENCE, |frame| {
            frame["sub"] == now_side && frame["ephemeral"]["type"] == "thread_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["thread"], thread);
    assert_eq!(delta["ephemeral"]["kind"], "text");
    assert_eq!(delta["ephemeral"]["text"], "On it.");
    let delta = old
        .next_where(PATIENCE, |frame| {
            frame["sub"] == old_side && frame["ephemeral"]["type"] == "side_agent_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["sideId"], key.as_str());
    assert_eq!(delta["ephemeral"]["text"], "On it.");
    assert!(
        old.inbox
            .iter()
            .all(|frame| frame["ephemeral"]["type"] != "thread_delta"),
        "an old client is never sent a ThreadDelta"
    );
    now.next_where(PATIENCE, |frame| {
        frame["sub"] == now_side
            && frame["event"]["kind"] == "agent"
            && frame["event"]["text"] == "On it."
    })
    .await;

    // The same list, by either name; the work thread is live and was asked
    // about by its first line.
    let listed = now.call("thread.list", json!({"personaId":persona})).await["result"].clone();
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["thread"] == thread)
        .expect("the work thread is listed")
        .clone();
    assert_eq!(row["state"], "live");
    assert_eq!(row["preview"], "On it.");
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["thread"] == json!({"kind":"dm","key":persona})),
        "the DM is listed with it: {listed}"
    );
    let side_list = old.call("side.list", json!({"personaId":persona})).await["result"].clone();
    assert_eq!(side_list[0]["sideId"], key.as_str());
    assert_eq!(side_list[0]["status"], "live");

    // Park, speak into it (which wakes it), close, continue.
    let parked = now.call("thread.park", json!({"thread":thread})).await;
    assert_eq!(parked["ok"], true, "{parked}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "parked");
    assert_eq!(
        old.call("side.list", json!({"personaId":persona})).await["result"][0]["status"],
        "parked"
    );
    let woken = old
        .call("side.prompt", json!({"sideId":key,"text":"Still there?"}))
        .await;
    assert_eq!(woken["ok"], true, "{woken}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "live");
    now.next_where(PATIENCE, |frame| {
        frame["sub"] == now_side
            && frame["event"]["kind"] == "user"
            && frame["event"]["text"] == "Still there?"
    })
    .await;
    let closed = now.call("thread.close", json!({"thread":thread})).await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(state_of(&mut now, &persona, &thread).await, "closed");
    let refused = now
        .call("thread.prompt", json!({"thread":thread,"text":"hello?"}))
        .await;
    assert_eq!(
        refused["ok"], false,
        "a closed thread is spoken in only once continued"
    );
    let back = now.call("thread.continue", json!({"thread":thread})).await;
    assert_eq!(back["ok"], true, "{back}");
    assert_eq!(back["result"]["state"], "live");
    let cancelled = now.call("thread.cancel", json!({"thread":thread})).await;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let interrupted = now
        .call("thread.cancel", json!({"thread":thread,"interrupt":true}))
        .await;
    assert_eq!(interrupted["ok"], true, "{interrupted}");

    // The main conversation is a thread too: the same verbs, the same words.
    assert_eq!(
        now.call("session.start", json!({"personaId":persona}))
            .await["ok"],
        true
    );
    let said = now
        .call(
            "thread.prompt",
            json!({"thread":{"kind":"dm","key":persona},"text":"Morning"}),
        )
        .await;
    assert_eq!(said["ok"], true, "{said}");
    let delta = now
        .next_where(PATIENCE, |frame| {
            frame["sub"] == now_tape && frame["ephemeral"]["type"] == "thread_delta"
        })
        .await;
    assert_eq!(
        delta["ephemeral"]["thread"],
        json!({"kind":"dm","key":persona})
    );
    let delta = old
        .next_where(PATIENCE, |frame| {
            frame["sub"] == old_tape && frame["ephemeral"]["type"] == "agent_delta"
        })
        .await;
    assert_eq!(delta["ephemeral"]["personaId"], persona.as_str());
    for (cmd, params) in [
        (
            "session.cancel",
            json!({"personaId":persona,"interrupt":true}),
        ),
        (
            "thread.cancel",
            json!({"thread":{"kind":"dm","key":persona},"interrupt":true}),
        ),
    ] {
        let interrupted = now.call(cmd, params).await;
        assert_eq!(interrupted["ok"], true, "{cmd}: {interrupted}");
    }

    now.call("session.stop", json!({"personaId":persona})).await;
    drop(now);
    drop(old);
    door_task.abort();
    server.abort();
}

/// A thread's state in the list, which is where a client reads it.
async fn state_of(client: &mut Client, persona: &str, thread: &Value) -> String {
    let listed = client
        .call("thread.list", json!({"personaId":persona}))
        .await["result"]
        .clone();
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["thread"] == *thread)
        .expect("the thread is listed")["state"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[path = "threads/together.rs"]
mod together;

/// The event on subscription `sub` that `wanted` picks, live or in a
/// snapshot it was sent. A snapshot found goes back to the inbox less the
/// event taken, for the next wait to read.
async fn live_or_snapshot(client: &mut Client, sub: i64, wanted: impl Fn(&Value) -> bool) -> Value {
    let mut frame = client
        .next_where(PATIENCE, |frame| {
            frame["sub"] == sub
                && (wanted(&frame["event"])
                    || frame["snapshot"]
                        .as_array()
                        .is_some_and(|events| events.iter().any(&wanted)))
        })
        .await;
    let Some(events) = frame["snapshot"].as_array_mut() else {
        return frame["event"].clone();
    };
    let at = events.iter().position(&wanted).unwrap();
    let event = events.remove(at);
    client.inbox.push_front(frame);
    event
}
