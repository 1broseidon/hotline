use super::*;
use http::header::SEC_WEBSOCKET_PROTOCOL;
use tokio_tungstenite::tungstenite::{Error, handshake::client::Request};

fn request(bridge: &bridge::Bridge, path: &str, token: Option<&str>) -> Request {
    let mut request = format!("{}{path}", bridge.origin.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            format!("unused, hotline-viewer.{token}").parse().unwrap(),
        );
    }
    request
}
async fn refused(request: Request) {
    match tokio_tungstenite::connect_async(request).await {
        Err(Error::Http(response)) => assert_eq!(response.status(), 403),
        other => panic!("expected the bridge to refuse the upgrade: {other:?}"),
    }
}

#[tokio::test]
async fn viewer_tokens_are_persona_bound_single_use_and_never_owner_credentials() {
    let (port, asked) = fake_computer().await;
    let (h, room) = Harness::with_computer().await;
    *room.viewer.lock().unwrap() = Some(format!("http://127.0.0.1:{port}/#secret-bearer"));
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = client
        .pair(
            &h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload,
            "Viewer token test",
        )
        .await
        .unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client.clone())
        .await
        .unwrap();
    let other_bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let token = bridge.viewer_token("ada");
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(token, bridge.token);
    for path in [
        "/ws",
        "/computer/bob/ws",
        "/computer/ada/ws/",
        "/computer/bob/../ada/ws",
        "/computer/%61da/ws",
    ] {
        refused(request(&bridge, path, Some(&token))).await;
    }
    refused(request(&bridge, &format!("/ws?token={token}"), None)).await;
    refused(request(
        &bridge,
        &format!("/computer/ada/ws?token={token}"),
        None,
    ))
    .await;
    // The owner token still opens the wire, but cannot replace a viewer token.
    refused(request(&bridge, "/computer/ada/ws", None)).await;
    refused(request(
        &bridge,
        &format!("/computer/ada/ws?token={}", bridge.token),
        None,
    ))
    .await;
    let (mut wire, _) = tokio_tungstenite::connect_async(request(
        &bridge,
        &format!("/ws?token={}", bridge.token),
        None,
    ))
    .await
    .unwrap();
    let hello = tokio::time::timeout(Duration::from_secs(5), wire.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let hello: Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    drop(wire);
    refused(request(&other_bridge, "/computer/ada/ws", Some(&token))).await;
    refused(request(&bridge, "/computer/ada/ws", Some(&bridge.token))).await;
    refused(request(
        &bridge,
        &format!("/computer/ada/ws?token={}", bridge.token),
        Some(&token),
    ))
    .await;
    let mut ambiguous = request(&bridge, "/computer/ada/ws", Some(&token));
    ambiguous.headers_mut().append(
        SEC_WEBSOCKET_PROTOCOL,
        format!("hotline-viewer.{token}").parse().unwrap(),
    );
    refused(ambiguous).await;
    refused(request(&bridge, "/computer/ada/ws", Some("not-hex"))).await;
    assert!(
        asked.lock().unwrap().is_none(),
        "refused upgrades never reach the computer"
    );

    // Wrong routes do not consume it, but two eligible upgrades cannot both win.
    let (left, right) = tokio::join!(
        tokio_tungstenite::connect_async(request(&bridge, "/computer/ada/ws", Some(&token))),
        tokio_tungstenite::connect_async(request(&bridge, "/computer/ada/ws", Some(&token))),
    );
    let mut winners = Vec::new();
    for result in [left, right] {
        match result {
            Ok((socket, response)) => {
                assert_eq!(
                    response.headers()[SEC_WEBSOCKET_PROTOCOL],
                    format!("hotline-viewer.{token}")
                );
                winners.push(socket);
            }
            Err(Error::Http(response)) => assert_eq!(response.status(), 403),
            other => panic!("unexpected upgrade result: {other:?}"),
        }
    }
    assert_eq!(winners.len(), 1);
    let mut socket = winners.pop().unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::binary(b"\x89PNG frame".to_vec())
    );
    socket
        .send(Message::text("{\"type\":\"takeover\"}"))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::text("{\"type\":\"takeover\"}")
    );
    refused(request(&bridge, "/computer/ada/ws", Some(&token))).await;
    drop(socket);
    refused(request(&bridge, "/computer/ada/ws", Some(&token))).await;
    // Reconnecting uses a fresh capability, not the already-spent one.
    let fresh = bridge.viewer_token("ada");
    assert_ne!(fresh, token);
    let (socket, response) =
        tokio_tungstenite::connect_async(request(&bridge, "/computer/ada/ws", Some(&fresh)))
            .await
            .unwrap();
    assert_eq!(
        response.headers()[SEC_WEBSOCKET_PROTOCOL],
        format!("hotline-viewer.{fresh}")
    );
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn viewer_tokens_expire_thirty_seconds_after_minting() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = client
        .pair(
            &h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload,
            "Expired viewer",
        )
        .await
        .unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let token = bridge.viewer_token("ada");
    tokio::time::sleep(Duration::from_secs(30)).await;
    refused(request(&bridge, "/computer/ada/ws", Some(&token))).await;
    // Expiry of a viewer capability must not expire the owner's bridge.
    let (socket, _) = tokio_tungstenite::connect_async(request(
        &bridge,
        &format!("/ws?token={}", bridge.token),
        None,
    ))
    .await
    .unwrap();
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}
