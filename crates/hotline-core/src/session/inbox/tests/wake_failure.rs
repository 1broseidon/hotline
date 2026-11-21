//! A wake that never reached the model must leave the original item unread.

use super::pick_up::idle;
use super::*;
use crate::contract::{InboxNotice, InboxWakeFailureState};
use tokio::time::sleep;

fn quota_refusal() -> Vec<Update> {
    vec![
        Update::Message {
            kind: MessageKind::Thought,
            id: new_id(),
            text: "Usage Limit Reached. You have reached your current quota for this period. Your limit will reset in 1 hour, 16 minutes.".into(),
        },
        Update::Turn {
            stop_reason: "refusal".into(),
            usage: None,
        },
    ]
}

fn deferred(room: &Room, id: &str) -> i64 {
    let saved = item(room, "bob", id);
    assert_eq!(saved.state, InboxState::New);
    assert!(!states(&saved).contains(&InboxState::Seen));
    assert_eq!(saved.outcome, None);
    assert_eq!(saved.offered, None);
    assert_eq!(room.inbox_unread("bob"), 1);
    let due = saved.offer_again.expect("the failed wake has a retry time");
    assert!(due > now_ms(), "the retry is not due yet");
    due
}

fn wake_notice(room: &Room, original: &str, state: InboxWakeFailureState) -> InboxItem {
    let mut notices: Vec<_> = items(room, "ada")
        .into_iter()
        .filter(|item| item.notice == Some(InboxNotice::WakeFailed))
        .collect();
    assert_eq!(notices.len(), 1, "one wake failure notice: {notices:?}");
    let notice = notices.pop().unwrap();
    assert_eq!(notice.reply_to.as_deref(), Some(original));
    assert_eq!(
        notice.wake_failure.as_ref().expect("typed failure").state,
        state
    );
    notice
}

#[tokio::test(start_paused = true)]
async fn a_driver_sign_in_error_keeps_the_quick_message_unread() {
    use crate::contract::NoticeLevel;
    use crate::driver::failure::Failure;

    let failure = Failure::classify("invalid API key", Some(401), "model_request");
    let (room, _) = room_on(
        "inbox-wake-sign-in",
        Scripted::new(vec![
            Update::Notice {
                level: NoticeLevel::Error,
                text: failure.notice(),
            },
            Update::Turn {
                stop_reason: "failed".into(),
                usage: None,
            },
        ]),
    );
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": "Which runner?"}),
    )
    .await
    .unwrap();
    let id = sent["sent"][0]["id"].as_str().unwrap();
    until("the sender has a failed-wake notice", || {
        items(&room, "ada")
            .iter()
            .any(|item| item.notice == Some(InboxNotice::WakeFailed))
    })
    .await;
    deferred(&room, id);
    wake_notice(&room, id, InboxWakeFailureState::NotSignedIn);
}

#[tokio::test(start_paused = true)]
async fn stopping_a_wake_before_output_does_not_mark_it_seen() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let (room, agents) = room_on(
        "inbox-wake-stopped",
        replies().gated(gate).on_cancel(vec![Update::Turn {
            stop_reason: "cancelled".into(),
            usage: None,
        }]),
    );
    let bob = at_rest(&room, "bob").await;
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": "Which runner?"}),
    )
    .await
    .unwrap();
    let id = sent["sent"][0]["id"].as_str().unwrap();
    until("the prompt is offered to Bob", || !agents.told().is_empty()).await;
    assert_eq!(item(&room, "bob", id).state, InboxState::New);
    room.cancel("bob").unwrap();
    idle(&bob).await;
    deferred(&room, id);
    wake_notice(&room, id, InboxWakeFailureState::Stopped);
}

async fn make_retry_due(room: &Arc<Room>, id: &str) {
    // Tokio is paused, but the persisted dates use the wall clock.
    let mut saved = item(room, "bob", id);
    saved.offer_again = Some(now_ms() - 1);
    room.keep_item(&saved).unwrap();
    room.inbox_due(now_ms()).await;
}

#[tokio::test(start_paused = true)]
async fn a_quota_refusal_leaves_a_quick_question_unread_until_a_successful_retry() {
    let script = Scripted::turns(vec![
        quota_refusal(),
        quota_refusal(),
        vec![Update::Turn {
            stop_reason: "end_turn".into(),
            usage: None,
        }],
    ]);
    let (room, agents) = room_on("inbox-quota-refusal-retry", script);
    // The fake shares one script: Ada keeps the notice without consuming Bob's turns.
    room.log
        .append(
            &StreamId::Room,
            &json!({"kind": "quiet_limit", "id": "quiet-limit:ada", "teammate": "ada", "held": "stopped"}),
        )
        .unwrap();
    let bob = at_rest(&room, "bob").await;
    let before = now_ms();
    let sent = tool(
        &room,
        "ada",
        "send",
        json!({"to": ["bob"], "message": "Which runner?"}),
    )
    .await
    .unwrap();
    let id = sent["sent"][0]["id"].as_str().unwrap();

    until("Bob got the quick question", || !agents.told().is_empty()).await;
    idle(&bob).await;
    assert!(agents.told()[0].contains("message: \"Which runner?\""));
    let due = deferred(&room, id);
    let notice = wake_notice(&room, id, InboxWakeFailureState::OutOfQuota);
    let reset_at = notice.wake_failure.unwrap().reset_at.expect("quota reset");
    let delay_ms = (60 + 16) * 60 * 1_000;
    assert!((before + delay_ms..=now_ms() + delay_ms).contains(&reset_at));
    assert_eq!(due, reset_at);
    assert_eq!(lock(&bob.pick_up).armed(), None, "no immediate pick-up");
    room.inbox_due(now_ms()).await;
    sleep(heard::gather_delay() * 3).await;
    assert_eq!(agents.prompts().len(), 1, "no turn before the retry is due");

    make_retry_due(&room, id).await;
    until("Bob's second wake", || agents.prompts().len() >= 2).await;
    idle(&bob).await;
    deferred(&room, id);
    assert_eq!(
        wake_notice(&room, id, InboxWakeFailureState::OutOfQuota).id,
        notice.id
    );

    make_retry_due(&room, id).await;
    until("the successful wake marks the question seen", || {
        item(&room, "bob", id).state == InboxState::Seen
    })
    .await;
    idle(&bob).await;
    let heard = agents.told();
    assert_eq!(heard.len(), 3);
    assert!(
        heard
            .iter()
            .all(|turn| turn.contains("message: \"Which runner?\""))
    );
    assert_eq!(room.inbox_unread("bob"), 0);
    assert_eq!(item(&room, "bob", id).offer_again, None);
    assert_eq!(
        wake_notice(&room, id, InboxWakeFailureState::OutOfQuota).id,
        notice.id
    );
}

#[tokio::test(start_paused = true)]
async fn a_cold_start_failure_retries_only_when_due_and_posts_the_persons_notice_once() {
    let (room, agents) = room_on(
        "inbox-cold-start-wake-failure",
        replies().failing_to_start("offline"),
    );
    let id = {
        let _turn = room.open_posting(&ThreadId::dm("ada"), None);
        send(
            &room,
            "ada",
            "bob",
            "Check the crane",
            json!({"wants": "work"}),
        )
        .await
    };
    assert!(item(&room, "bob", &id).for_person);
    let bob_starts = || {
        agents
            .views()
            .iter()
            .filter(|view| view.id == "bob")
            .count()
    };
    let chat_posts = || {
        room.tape("ada")
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .collect::<Vec<_>>()
    };

    until("the cold gather retired", || {
        lock(&room.inbox_gathers).is_empty()
    })
    .await;
    assert_eq!(bob_starts(), 1);
    deferred(&room, &id);
    let notice = wake_notice(&room, &id, InboxWakeFailureState::CouldNotStart);
    assert_eq!(notice.wake_failure.unwrap().reset_at, None);
    let posts = chat_posts();
    assert_eq!(posts.len(), 1, "one durable chat post: {posts:?}");
    let text = posts[0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Hotline:") && text.contains("Bob") && text.contains("still waiting"),
        "{text}"
    );
    assert!(
        agents.prompts().is_empty(),
        "startup never reached a prompt"
    );
    room.inbox_due(now_ms()).await;
    sleep(heard::gather_delay() * 3).await;
    assert_eq!(
        bob_starts(),
        1,
        "no second startup while the item is deferred"
    );

    make_retry_due(&room, &id).await;
    until("the due cold start failed again", || {
        item(&room, "bob", &id)
            .offer_again
            .is_some_and(|at| at > now_ms())
    })
    .await;
    assert_eq!(bob_starts(), 2, "the clock retries the cold start once");
    deferred(&room, &id);
    assert_eq!(agents.prompts(), [] as [String; 0]);
    assert_eq!(
        wake_notice(&room, &id, InboxWakeFailureState::CouldNotStart).id,
        notice.id
    );
    assert_eq!(chat_posts(), posts, "the second failure adds no chat post");

    agents.allow_starts();
    make_retry_due(&room, &id).await;
    until("the recovered receiver gets its item", || {
        !agents.prompts().is_empty()
    })
    .await;
    idle(&room.session("bob").unwrap()).await;
    assert_eq!(item(&room, "bob", &id).state, InboxState::Noticed);
    tool(&room, "bob", "read_inbox", json!({"ids": [&id]}))
        .await
        .unwrap();
    assert_eq!(item(&room, "bob", &id).state, InboxState::Seen);
}
