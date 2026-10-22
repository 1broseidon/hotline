use super::*;

fn concurrent_door(room: Quiet) -> (tempfile::TempDir, Log, u16) {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let door = Door::bind(
        log.clone(),
        "desk-token".into(),
        Arc::new(room),
        Parts::default(),
    )
    .unwrap();
    let port = door.port();
    tokio::spawn(door.run());
    (root, log, port)
}

/// A provider holding an ordered mutation cannot hold the reader. Reads and
/// catalog queries answer, while subsequent writes and subscriptions wait.
#[tokio::test]
async fn a_pending_global_command_does_not_block_independent_commands() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_login = Some(pending.clone());
    let (_root, log, port) = concurrent_door(room);
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"credential.login","params":{"providerId":"openai"}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), pending.entered.cancelled())
        .await
        .expect("login entered");
    ask(
        &mut socket,
        json!({"id":2,"cmd":"settings.update","params":{"patch":{"theme":"dark"}}}),
    )
    .await;
    ask(&mut socket, json!({"id":3,"sub":"room"})).await;
    ask(&mut socket, json!({"id":4,"cmd":"ping"})).await;
    ask(&mut socket, json!({"id":5,"cmd":"providers.list"})).await;
    let response = tokio::time::timeout(Duration::from_secs(5), heard(&mut socket))
        .await
        .expect("an independent read bypasses the pending mutation");
    assert_eq!(response["id"], 5, "{response}");
    assert_eq!(response["ok"], true);
    assert!(!pending.completed.is_cancelled());
    assert_ne!(room::settings(&log).get("theme"), Some(&json!("dark")));
    pending.release.cancel();
    let mut replies = Vec::new();
    let mut snapshot_seen = false;
    while replies.len() < 4 || !snapshot_seen {
        let frame = tokio::time::timeout(Duration::from_secs(5), heard(&mut socket))
            .await
            .expect("ordered replies and snapshot arrive");
        if let Some(id) = frame["id"].as_i64() {
            assert_eq!(frame["ok"], id != 1, "{frame}");
            replies.push(id);
        }
        if frame["sub"] == 3 {
            assert!(
                frame["snapshot"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["id"] == "theme" && event["value"] == "dark")
            );
            snapshot_seen = true;
        }
    }
    assert_eq!(replies, [1, 2, 3, 4]);
}

/// An opt-in measurement uses the same socket and handler with a fixed
/// provider delay, printing latency rather than asserting a timing ceiling.
#[tokio::test]
#[ignore = "measurement with a one-second provider delay"]
async fn global_command_latency_measurement() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_login = Some(pending.clone());
    let (_root, _log, port) = concurrent_door(room);
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"credential.login","params":{"providerId":"openai"}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), pending.entered.cancelled())
        .await
        .expect("login entered");
    let release = pending.release.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        release.cancel();
    });
    let began = std::time::Instant::now();
    ask(&mut socket, json!({"id":2,"cmd":"providers.list"})).await;
    assert_eq!(answered(&mut socket, 2).await["ok"], true);
    eprintln!("providers.list behind a 1s login: {:?}", began.elapsed());
    timer.await.unwrap();
}

/// In-memory WebSockets let the silence deadline run on Tokio's paused
/// clock while exercising the same seated reader as the network transport.
#[tokio::test]
async fn a_lean_seat_expires_while_a_global_command_is_pending() {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let root = tempfile::tempdir().unwrap();
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_login = Some(pending.clone());
    let agents = crate::wire::Agents {
        log: Log::open(root.path()),
        room: Arc::new(room),
        parts: Parts::default(),
    };
    let (client, server) = tokio::io::duplex(8192);
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let mut client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let seat = tokio::spawn(super::super::socket::seated(server, Seat::Owner, agents));
    client
        .send(Message::text(
            json!({"id":1,"cmd":"client.hello","params":{"capabilities":["lean"]}}).to_string(),
        ))
        .await
        .unwrap();
    let hello = client.next().await.unwrap().unwrap();
    assert!(hello.into_text().unwrap().contains("capabilities"));
    client
        .send(Message::text(
            json!({"id":2,"cmd":"credential.login","params":{"providerId":"openai"}}).to_string(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), pending.entered.cancelled())
        .await
        .expect("login entered");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(90)).await;
    tokio::time::timeout(Duration::from_secs(5), seat)
        .await
        .expect("the silent seat ends")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), pending.dropped.cancelled())
        .await
        .expect("the remote command is cancelled");
    assert!(!pending.completed.is_cancelled());
}

#[tokio::test]
async fn a_desk_disconnect_finishes_an_accepted_global_write() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_login = Some(pending.clone());
    let (_root, _log, port) = concurrent_door(room);
    let mut socket = desk(port).await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"credential.login","params":{"providerId":"openai"}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), pending.entered.cancelled())
        .await
        .expect("login entered");
    socket.close(None).await.unwrap();
    pending.release.cancel();
    tokio::time::timeout(Duration::from_secs(5), pending.completed.cancelled())
        .await
        .expect("accepted desk work completed after disconnect");
}

#[tokio::test]
async fn teammate_commands_keep_their_order_while_other_lanes_answer() {
    let pending = Arc::new(PendingStop::default());
    let mut room = Quiet::new();
    room.pending_stop = Some(pending.clone());
    let (_root, _log, port) = concurrent_door(room);
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 0, "Ada").await;
    ask(
        &mut socket,
        json!({"id":1,"cmd":"computer.stop","params":{"personaId":ada["id"]}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), pending.entered.cancelled())
        .await
        .expect("computer stop entered");
    ask(
        &mut socket,
        json!({"id":2,"cmd":"session.stop","params":{"personaId":ada["id"]}}),
    )
    .await;
    ask(
        &mut socket,
        json!({"id":3,"cmd":"session.stop","params":{"personaId":"bob"}}),
    )
    .await;
    let other = tokio::time::timeout(Duration::from_secs(5), heard(&mut socket))
        .await
        .expect("the other lane answers");
    assert_eq!(other["id"], 3);
    assert_eq!(other["ok"], true);
    pending.release.cancel();
    for id in [1, 2] {
        let reply = tokio::time::timeout(Duration::from_secs(5), heard(&mut socket))
            .await
            .expect("the teammate lane answers in order");
        assert_eq!(reply["id"], id);
        assert_eq!(reply["ok"], true);
    }
}

#[test]
fn every_account_command_but_a_running_sign_ins_is_independent() {
    let account = "11111111-1111-4111-8111-111111111111";
    let frame = |cmd: &str, params: Value| json!({"id":1,"cmd":cmd,"params":params});
    for (cmd, params) in [
        ("accounts.list", json!({})),
        (
            "accounts.create",
            json!({"backendId":"claude-acp","name":"Work","startWith":"empty"}),
        ),
        (
            "accounts.rename",
            json!({"accountId":account,"name":"Home"}),
        ),
        ("accounts.remove", json!({"accountId":account})),
        ("accounts.status", json!({"accountId":account})),
        ("accounts.setup.list", json!({"accountId":account})),
        (
            "accounts.setup.copy",
            json!({"accountId":account,"categories":["skills"]}),
        ),
        ("accounts.login.start", json!({"accountId":account})),
    ] {
        assert!(super::socket::independent(&frame(cmd, params)), "{cmd}");
    }
    for (cmd, params) in [
        (
            "accounts.login.poll",
            json!({"accountId":account,"id":"a1"}),
        ),
        (
            "accounts.login.input",
            json!({"accountId":account,"id":"a1","input":"x"}),
        ),
        (
            "accounts.login.cancel",
            json!({"accountId":account,"id":"a1"}),
        ),
    ] {
        let frame = frame(cmd, params);
        assert!(super::socket::read_command(&frame).is_ok(), "{cmd}");
        assert!(!super::socket::independent(&frame), "{cmd}");
    }
}
