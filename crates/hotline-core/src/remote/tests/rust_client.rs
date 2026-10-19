use super::*;

#[tokio::test]
async fn rust_client_pairs_pins_and_recovers_subscriptions_without_replaying_commands() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let decoded = PairingPayload::from_link(&invitation.link).unwrap();
    assert_eq!(decoded.secret, invitation.payload.secret);
    assert_eq!(decoded.expires_at, invitation.expires_at);
    assert_eq!(decoded.version, 2);
    let desk = client.pair(&decoded, "Test laptop").await.unwrap();
    assert_eq!(desk.desk_id, h.remote.status_desktop_id());
    assert!(client.pair(&decoded, "Second claim").await.is_err());
    let mut session = client.connect(&desk);
    async fn receive(session: &mut client::Session) -> Value {
        serde_json::from_str(
            &tokio::time::timeout(ANSWER, session.incoming.recv())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }
    assert_eq!(receive(&mut session).await["type"], "hello");
    assert_eq!(*session.state.borrow(), client::State::Open);
    session
        .outgoing
        .send(json!({"id":1,"sub":"room"}).to_string())
        .await
        .unwrap();
    assert_eq!(receive(&mut session).await["ok"], true);
    assert!(receive(&mut session).await["snapshot"].is_array());
    session.outgoing.send(json!({"id":2,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/client-test-skills"}}}).to_string()).await.unwrap();
    loop {
        let frame = receive(&mut session).await;
        if frame["id"] == 2 {
            assert_eq!(frame["ok"], true);
            break;
        }
    }
    h.remote.configure(false, network::ALL).await.unwrap();
    tokio::time::timeout(
        ANSWER,
        session
            .state
            .wait_for(|state| *state == client::State::Unreachable),
    )
    .await
    .unwrap()
    .unwrap();
    h.remote.configure(true, network::ALL).await.unwrap();
    loop {
        if receive(&mut session).await["type"] == "hello" {
            break;
        }
    }
    assert_eq!(receive(&mut session).await["ok"], true);
    let snapshot = receive(&mut session).await;
    assert!(
        snapshot["snapshot"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event.to_string().contains("/tmp/client-test-skills"))
    );
    session
        .outgoing
        .send(json!({"id":3,"cmd":"backends.list"}).to_string())
        .await
        .unwrap();
    loop {
        if receive(&mut session).await["id"] == 3 {
            break;
        }
    }
    assert_eq!(
        h.desk
            .log
            .load(&crate::log::StreamId::Room)
            .iter()
            .filter(|event| event.to_string().contains("/tmp/client-test-skills"))
            .count(),
        1,
        "the mutation was not replayed"
    );
    let mut tampered = desk.clone();
    tampered.desk_key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
    assert!(client.open(&tampered, None).await.is_err());
    drop(session);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_and_bridge_learn_and_clear_a_relay_from_an_authenticated_hello() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let desk = client
        .pair(&invitation.payload, "Discovery laptop")
        .await
        .unwrap();
    assert!(desk.relay.is_none());

    // This desk chose a relay after pairing. Its next authenticated hello,
    // rather than an invitation or an unsealed server frame, announces it.
    let relay = format!("https://relay.example/room/relay/{}", desk.desk_id);
    h.remote.state.lock().unwrap().relay.url = Some(relay.clone());
    let mut session = client.connect(&desk);
    tokio::time::timeout(ANSWER, session.relay.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*session.relay.borrow_and_update(), Some(relay.clone()));
    assert_eq!(
        serde_json::from_str::<Value>(&session.incoming.recv().await.unwrap()).unwrap()["type"],
        "hello"
    );

    let mut bridge = bridge::Bridge::with_client(&desk, client.clone())
        .await
        .unwrap();
    let (mut local, _) = tokio_tungstenite::connect_async(format!(
        "{}/ws?token={}",
        bridge.origin.replace("http:", "ws:"),
        bridge.token
    ))
    .await
    .unwrap();
    tokio::time::timeout(ANSWER, bridge.relay.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*bridge.relay.borrow_and_update(), Some(relay.clone()));
    assert!(matches!(local.next().await, Some(Ok(Message::Text(_)))));
    drop(local);
    drop(session);

    // An authenticated hello that no longer advertises a relay clears it.
    h.remote.state.lock().unwrap().relay.url = None;
    let mut previous = desk.clone();
    previous.relay = Some(relay);
    let mut session = client.connect(&previous);
    tokio::time::timeout(ANSWER, session.relay.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*session.relay.borrow(), None);
    let (local, _) = tokio_tungstenite::connect_async(format!(
        "{}/ws?token={}",
        bridge.origin.replace("http:", "ws:"),
        bridge.token
    ))
    .await
    .unwrap();
    tokio::time::timeout(ANSWER, bridge.relay.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*bridge.relay.borrow(), None);
    drop(local);
    drop(bridge);
    drop(session);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_and_bridge_preserve_a_known_relay_when_authenticated_endpoints_are_malformed()
{
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let mut desk = client
        .pair(&invitation.payload, "Endpoint validation")
        .await
        .unwrap();
    let known = Some(format!("https://relay.example/relay/{}", desk.desk_id));
    desk.relay = known.clone();
    // The listener still answers at the paired address, but its authenticated
    // hello has a malformed endpoint list. That is not a relay removal.
    h.remote.state.lock().unwrap().endpoints = vec!["http://invalid.example".into()];
    let mut session = client.connect(&desk);
    let hello = tokio::time::timeout(ANSWER, session.incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&hello).unwrap()["type"],
        "hello"
    );
    assert_eq!(*session.state.borrow(), client::State::Open);
    assert_eq!(*session.relay.borrow(), known);
    assert!(!session.relay.has_changed().unwrap());
    assert!(session.discovery.borrow().is_none());

    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let (mut local, _) = tokio_tungstenite::connect_async(format!(
        "{}/ws?token={}",
        bridge.origin.replace("http:", "ws:"),
        bridge.token
    ))
    .await
    .unwrap();
    let hello = tokio::time::timeout(ANSWER, local.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(hello, Message::Text(_)));
    assert_eq!(*bridge.relay.borrow(), known);
    assert!(!bridge.relay.has_changed().unwrap());
    drop(local);
    drop(bridge);
    drop(session);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_companion_cannot_change_settings_or_subscribe_to_room() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Companion).unwrap();
    let desk = client
        .pair(&invitation.payload, "Companion laptop")
        .await
        .unwrap();
    let mut socket = client.open(&desk, None).await.unwrap();
    socket.next().await.unwrap().unwrap();
    for request in [
        json!({"id":1,"sub":"room"}),
        json!({"id":2,"cmd":"settings.update","params":{"patch":{"skillsHome":"/tmp/no"}}}),
    ] {
        socket
            .send(Message::text(request.to_string()))
            .await
            .unwrap();
        let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
            panic!("expected reply");
        };
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["code"], "forbidden");
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn pairing_failures_do_not_consume_the_invitation_or_replace_a_pin() {
    struct Locked;
    impl crate::credentials::SecretStore for Locked {
        fn get(&self, _: &str) -> std::io::Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("test store locked"))
        }
        fn delete(&self, _: &str) -> std::io::Result<()> {
            Ok(())
        }
    }
    let h = Harness::new().await;
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let mut invalid = invitation.payload.clone();
    invalid.version = 99;
    assert!(client.pair(&invalid, "Laptop").await.is_err());
    invalid = invitation.payload.clone();
    invalid.expires_at = crate::remote::now() - 1;
    assert!(client.pair(&invalid, "Laptop").await.is_err());
    invalid = invitation.payload.clone();
    invalid.url = invalid.url.replace("https:", "http:");
    assert!(client.pair(&invalid, "Laptop").await.is_err());
    invalid = invitation.payload.clone();
    invalid.desk_key =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed::keypair().unwrap().1);
    assert!(client.pair(&invalid, "Laptop").await.is_err());
    assert!(
        client::Client::new(Arc::new(Locked))
            .pair(&invitation.payload, "Laptop")
            .await
            .is_err()
    );
    assert!(h.remote.pairing_result(&invitation.id).unwrap().is_none());
    let paired = client.pair(&invitation.payload, "Laptop").await.unwrap();
    assert_eq!(paired.desk_key, invitation.payload.desk_key);
    assert_eq!(h.remote.devices().len(), 1);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_reports_revoked_only_after_a_pinned_authenticated_rejection() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let desk = client
        .pair(&invitation.payload, "Revocation test")
        .await
        .unwrap();
    let mut session = client.connect(&desk);
    tokio::time::timeout(
        ANSWER,
        session.state.wait_for(|s| *s == client::State::Open),
    )
    .await
    .unwrap()
    .unwrap();
    let mut bridge = bridge::Bridge::with_client(&desk, client.clone())
        .await
        .unwrap();
    let (local, _) = tokio_tungstenite::connect_async(format!(
        "{}/ws?token={}",
        bridge.origin.replace("http:", "ws:"),
        bridge.token
    ))
    .await
    .unwrap();
    tokio::time::timeout(ANSWER, bridge.state.wait_for(|s| *s == client::State::Open))
        .await
        .unwrap()
        .unwrap();
    h.remote.revoke(&h.remote.devices()[0].id).unwrap();
    tokio::time::timeout(
        ANSWER,
        session.state.wait_for(|s| *s == client::State::Revoked),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(
        ANSWER,
        bridge.state.wait_for(|s| *s == client::State::Revoked),
    )
    .await
    .unwrap()
    .unwrap();
    drop(local);
    drop(bridge);
    // Revocation is terminal; a new client session also recognizes an offline revoke.
    tokio::time::timeout(ANSWER, async {
        while session.state.changed().await.is_ok() {}
    })
    .await
    .unwrap();
    assert_eq!(*session.state.borrow(), client::State::Revoked);
    let mut fresh = client.connect(&desk);
    tokio::time::timeout(
        ANSWER,
        fresh.state.wait_for(|s| *s == client::State::Revoked),
    )
    .await
    .unwrap()
    .unwrap();
    drop(fresh);
    drop(session);

    // Pair() saved a device identity for this pin before the handshake failed.
    // Thus connect() actually attempts Noise, rather than failing for a missing key.
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let mut wrong = invitation.payload.clone();
    wrong.desk_key =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed::keypair().unwrap().1);
    assert!(client.pair(&wrong, "Wrong pin").await.is_err());
    let mut wrong_desk = desk.clone();
    wrong_desk.desk_key = wrong.desk_key;
    let mut session = client.connect(&wrong_desk);
    tokio::time::timeout(
        ANSWER,
        session.state.wait_for(|s| *s == client::State::Unreachable),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(600),
            session.state.wait_for(|s| *s == client::State::Revoked)
        )
        .await
        .is_err()
    );
    drop(session);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_does_not_trust_a_close_or_an_unsealed_rejection() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let desk = client
        .pair(&invitation.payload, "Network failure test")
        .await
        .unwrap();
    let identity: Identity =
        serde_json::from_slice(&h.remote.identity.read().unwrap().unwrap()).unwrap();
    for forged in [
        None,
        Some(Message::Binary(sealed::DEVICE_REJECTED.to_vec().into())),
        Some(Message::text(
            json!({
                "type":"hello", "protocolVersion":2, "desktopId":desk.desk_id, "mode":"team",
                "endpoints":["https://forged.example/relay/desk"]
            })
            .to_string(),
        )),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut target = desk.clone();
        target.url = format!("https://{}", listener.local_addr().unwrap());
        let tls = server::tls(&identity).unwrap();
        let fake = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tls.accept(stream).await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert!(matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
            if let Some(forged) = forged {
                socket.send(forged).await.unwrap();
            }
            let _ = socket.close(None).await;
        });
        let mut session = client.connect(&target);
        tokio::time::timeout(
            ANSWER,
            session.state.wait_for(|s| *s == client::State::Unreachable),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(*session.relay.borrow(), None);
        assert!(!session.relay.has_changed().unwrap());
        assert!(session.discovery.borrow().is_none());
        assert!(!session.discovery.has_changed().unwrap());
        tokio::time::timeout(ANSWER, fake).await.unwrap().unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(600),
                session.state.wait_for(|s| *s == client::State::Revoked)
            )
            .await
            .is_err()
        );
        drop(session);
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}
