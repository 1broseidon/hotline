use super::*;

#[tokio::test]
async fn a_command_is_answered_by_its_id_and_a_command_nobody_has_is_refused() {
    let (_root, _log, port) = door("framing");
    let mut socket = desk(port).await;

    let created = create(&mut socket, 1, "Ada").await;
    assert_eq!(created["name"], "Ada");

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "persona.fly", "params": {} }),
    )
    .await;
    let refused = heard(&mut socket).await;
    assert_eq!(refused["id"], 2);
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("cannot read that command"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wire_answers_a_core_owned_collaboration_card_before_peer_start() {
    let (_root, log, port, room) = core_door("collaboration-wire");
    let mut socket = desk(port).await;
    ask(&mut socket, json!({ "id": 1, "sub": { "tape": "ada" } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 1, "ok": true }));
    let snapshot = heard(&mut socket).await;
    assert_eq!(snapshot["snapshot"], json!([]));

    let sent = crate::mcp::server::TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({ "to": "bob", "message": "read Bob's private file" }),
        )
        .await
        .unwrap();
    let id = serde_json::from_str::<Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    let card = heard_where(&mut socket, |frame| {
        frame["sub"] == 1 && frame["event"]["kind"] == "permission"
    })
    .await;
    let request_id = card["event"]["requestId"].as_str().unwrap();
    assert!(request_id.starts_with("collab:"), "{card}");
    assert_eq!(
        card["event"]["title"],
        "Allow Ada to ask Bob to work?\n\nAda's questions reach Bob's main conversation with you, and Bob answers from everything said there, including what you told it. Bob can also take handoffs in a work thread of its own, and uses its own workspace and enabled tools to fulfill Ada's requests."
    );

    ask(
        &mut socket,
        json!({
            "id": 2,
            "cmd": "session.answer_permission",
            "params": { "personaId": "ada", "requestId": request_id, "optionId": "deny" }
        }),
    )
    .await;
    let answer = answered(&mut socket, 2).await;
    assert_eq!(answer["ok"], true, "{answer}");
    // The refusal is the ask's saved answer; this room has no agent to hear it.
    let refused = || {
        log.load(&StreamId::Room)
            .into_iter()
            .filter(|event| event["kind"] == "exchange_pair")
            .flat_map(|pair| pair["requests"].as_array().cloned().unwrap_or_default())
            .rfind(|request| request["id"] == id.as_str() && request["failed"] == true)
    };
    crate::test_support::until("the ask is refused", || refused().is_some()).await;
    let refused = refused().unwrap();
    assert!(
        refused["reply"].as_str().unwrap().contains("denied"),
        "{refused}"
    );
    assert_eq!(
        log.load(&StreamId::Tape("bob".into())),
        [] as [Value; 0],
        "nothing reached Bob"
    );
    assert_eq!(
        log.load(&StreamId::Pair("ada~bob".to_string())),
        [] as [Value; 0]
    );
}

#[tokio::test]
async fn the_desk_token_opens_the_door_and_nothing_else_does() {
    let (_root, _log, port) = door("token");

    let wrong = connect_async(format!("ws://127.0.0.1:{port}/ws?token=not-it"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(wrong), Some(StatusCode::UNAUTHORIZED));

    let elsewhere = connect_async(format!("ws://127.0.0.1:{port}/pair?token=desk-token"))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(elsewhere), Some(StatusCode::NOT_FOUND));
}

fn status_of(error: Error) -> Option<StatusCode> {
    match error {
        Error::Http(response) => Some(response.status()),
        _ => None,
    }
}

#[test]
fn a_hung_up_client_and_an_interrupted_accept_are_waited_out() {
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::ConnectionAborted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::Interrupted)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from(std::io::ErrorKind::NotConnected)),
        None
    );
}

#[cfg(unix)]
#[test]
fn too_many_open_files_pauses_and_a_gone_listener_does_not() {
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ECONNABORTED)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EINTR)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EMFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::ENFILE)),
        Some(AcceptAgain::AfterPause)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(libc::EBADF)),
        None
    );
}

#[cfg(windows)]
#[test]
fn winsock_resource_pressure_pauses_without_losing_the_listener() {
    use windows_sys::Win32::Networking::WinSock::{
        WSAECONNRESET, WSAEMFILE, WSAENOBUFS, WSAENOTSOCK,
    };
    for code in [WSAEMFILE, WSAENOBUFS] {
        assert_eq!(
            accept_again(&std::io::Error::from_raw_os_error(code)),
            Some(AcceptAgain::AfterPause)
        );
    }
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAECONNRESET)),
        Some(AcceptAgain::Now)
    );
    assert_eq!(
        accept_again(&std::io::Error::from_raw_os_error(WSAENOTSOCK)),
        None
    );
}

/// A stream subscription that ends without an unsubscribe used to keep its
/// id forever, so the next client that reused the number was told it was
/// already open for a task that would never deliver.
#[tokio::test]
async fn a_subscription_id_is_free_once_its_task_has_ended() {
    let (_root, log, port) = door("sub-ended");
    let mut socket = desk(port).await;

    ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 7, "ok": true }));
    assert_eq!(
        heard(&mut socket).await,
        json!({ "sub": 7, "snapshot": [] })
    );

    log.close_broadcasts();

    let reused = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ask(&mut socket, json!({ "id": 7, "sub": "room" })).await;
            let answer = answered(&mut socket, 7).await;
            if answer["ok"] == true {
                return answer;
            }
            let error = answer["error"].as_str().unwrap_or("");
            assert!(
                error.contains("already open"),
                "waiting for the ended task to free the id, got {answer}"
            );
        }
    })
    .await
    .expect("the ended subscription never freed its id");
    assert_eq!(reused["ok"], true, "{reused}");
}
