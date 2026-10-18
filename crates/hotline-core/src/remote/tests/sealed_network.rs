use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

pub(super) fn initiator(private: &[u8; 32], public: &[u8; 32]) -> snow::HandshakeState {
    snow::Builder::new("Noise_IK_25519_ChaChaPoly_SHA256".parse().unwrap())
        .prologue(b"hotline/2")
        .unwrap()
        .local_private_key(private)
        .unwrap()
        .remote_public_key(public)
        .unwrap()
        .build_initiator()
        .unwrap()
}
pub(super) fn invitation(pairing: &SealedPairing) -> ([u8; 32], String) {
    let url = url::Url::parse(&pairing.url).unwrap();
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(params["v"], "2");
    (
        URL_SAFE_NO_PAD
            .decode(&params["k"])
            .unwrap()
            .try_into()
            .unwrap(),
        params["s"].clone(),
    )
}
pub(super) async fn exchange(
    h: &Harness,
    path: &str,
    private: &[u8; 32],
    public: &[u8; 32],
    payload: &[u8],
) -> Option<(Socket, snow::TransportState, Vec<u8>)> {
    exchange_via(h, &h.endpoint(), path, private, public, payload).await
}
pub(super) async fn exchange_via(
    h: &Harness,
    base: &str,
    path: &str,
    private: &[u8; 32],
    public: &[u8; 32],
    payload: &[u8],
) -> Option<(Socket, snow::TransportState, Vec<u8>)> {
    let mut socket = h.socket_via(base, path, "").await.ok()?;
    let mut noise = initiator(private, public);
    let mut buffer = [0u8; 4096];
    let size = noise.write_message(payload, &mut buffer).unwrap();
    socket
        .send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .ok()?;
    let result = tokio::time::timeout(ANSWER, socket.next()).await.unwrap();
    let Some(Ok(Message::Binary(bytes))) = result else {
        return None;
    };
    let size = noise.read_message(&bytes, &mut buffer).ok()?;
    Some((
        socket,
        noise.into_transport_mode().unwrap(),
        buffer[..size].to_vec(),
    ))
}
async fn rejected(h: &Harness, private: &[u8; 32], public: &[u8; 32]) {
    let (mut socket, _, answer) = exchange(h, "/v2", private, public, &[]).await.unwrap();
    assert_eq!(answer, sealed::DEVICE_REJECTED);
    closed(&mut socket).await;
}
pub(super) async fn claim(
    h: &Harness,
    pairing: &SealedPairing,
    private: &[u8; 32],
) -> Option<Value> {
    let (public, secret) = invitation(pairing);
    let payload = json!({"secret":secret, "name":"Sealed test phone"}).to_string();
    let (_, _, answer) = exchange(h, "/v2/pair", private, &public, payload.as_bytes()).await?;
    Some(serde_json::from_slice(&answer).unwrap())
}
pub(super) async fn read_sealed(socket: &mut Socket, state: &mut snow::TransportState) -> Value {
    let mut decoder = sealed::Decoder::new(channel::FRAME_MAX);
    loop {
        let frame = tokio::time::timeout(ANSWER, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Binary(bytes) = frame else {
            panic!("no unsealed banner or answer");
        };
        if let Some(text) = decoder.decode(state, &bytes).unwrap() {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
pub(super) async fn send_sealed(
    socket: &mut Socket,
    state: &mut snow::TransportState,
    value: Value,
) {
    for bytes in sealed::encode(state, &value.to_string()).unwrap() {
        socket.send(Message::Binary(bytes.into())).await.unwrap();
    }
}
pub(super) async fn closed(socket: &mut Socket) {
    let frame = tokio::time::timeout(ANSWER, socket.next()).await.unwrap();
    assert!(!matches!(
        frame,
        Some(Ok(Message::Text(_) | Message::Binary(_)))
    ));
}

#[tokio::test]
async fn explicit_pairing_is_atomic_and_only_owners_have_desk_authority() {
    let h = Harness::new().await;
    assert!(h.socket_at("/v2/pair", "").await.is_err());
    let (first, _) = sealed::keypair().unwrap();
    let (second, _) = sealed::keypair().unwrap();
    let (_, public) = h.remote.noise_keys().unwrap();
    // Knowing the desk key earns only an authenticated refusal, not a grant.
    rejected(&h, &first, &public).await;
    assert!(h.remote.devices().is_empty());
    let (_, wrong_desk) = sealed::keypair().unwrap();
    assert!(
        exchange(&h, "/v2", &first, &wrong_desk, &[])
            .await
            .is_none()
    );
    for role in [DeviceRole::Owner, DeviceRole::Companion] {
        let pairing = h.remote.pairing_v2(role).unwrap();
        assert!(h.remote.pairing_result(&pairing.id).unwrap().is_none());
        let (a, b) = tokio::join!(claim(&h, &pairing, &first), claim(&h, &pairing, &second));
        assert_ne!(
            a.is_some(),
            b.is_some(),
            "one invitation can create only one grant"
        );
        let private = if a.is_some() { &first } else { &second };
        assert_eq!(
            a.or(b).unwrap()["role"],
            serde_json::to_value(role).unwrap()
        );
        let device = h.remote.pairing_result(&pairing.id).unwrap().unwrap();
        assert_eq!(device.role, role);
        assert!(device.public_key.is_some());
        assert!(claim(&h, &pairing, private).await.is_none());
        let (mut socket, mut state, payload) =
            exchange(&h, "/v2", private, &public, &[]).await.unwrap();
        assert!(payload.is_empty());
        assert_eq!(
            read_sealed(&mut socket, &mut state).await["protocolVersion"],
            2
        );
        send_sealed(
            &mut socket,
            &mut state,
            json!({"id":1,"cmd":"backends.list"}),
        )
        .await;
        assert_eq!(read_sealed(&mut socket, &mut state).await["ok"], true);
        send_sealed(
            &mut socket,
            &mut state,
            json!({"id":2,"cmd":"settings.update","params":{"patch":{}}}),
        )
        .await;
        let reply = read_sealed(&mut socket, &mut state).await;
        assert_eq!(reply["ok"], role == DeviceRole::Owner);
        if role == DeviceRole::Companion {
            assert_eq!(reply["code"], "forbidden");
        }
        send_sealed(&mut socket, &mut state, json!({"id":3,"sub":"room"})).await;
        let reply = read_sealed(&mut socket, &mut state).await;
        assert_eq!(reply["ok"], role == DeviceRole::Owner);
        if role == DeviceRole::Companion {
            assert_eq!(reply["code"], "forbidden");
        }
        if role == DeviceRole::Owner {
            assert!(read_sealed(&mut socket, &mut state).await["snapshot"].is_array());
        }
        h.remote.revoke(&device.id).unwrap();
        closed(&mut socket).await;
        rejected(&h, private, &public).await;
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn revocation_and_disable_drop_blocked_commands_and_release_subscriptions() {
    for role in [DeviceRole::Owner, DeviceRole::Companion] {
        for disable in [false, true] {
            let (h, room) = Harness::with_computer().await;
            let pairing = h.remote.pairing_v2(role).unwrap();
            let (private, _) = sealed::keypair().unwrap();
            let (public, _) = invitation(&pairing);
            claim(&h, &pairing, &private).await.unwrap();
            let device = h.remote.pairing_result(&pairing.id).unwrap().unwrap();
            let (mut socket, mut state, _) =
                exchange(&h, "/v2", &private, &public, &[]).await.unwrap();
            assert_eq!(
                read_sealed(&mut socket, &mut state).await["protocolVersion"],
                2
            );
            send_sealed(&mut socket, &mut state, json!({"id":0,"cmd":"mobile.persona_create","params":{"requestId":Uuid::new_v4(),"name":"Ada"}})).await;
            let created = read_sealed(&mut socket, &mut state).await;
            assert_eq!(created["ok"], true, "{created}");
            let persona = created["result"]["id"].as_str().unwrap();
            for (id, target) in [(1, json!({"view":"roster"})), (2, json!({"tape":persona}))] {
                send_sealed(&mut socket, &mut state, json!({"id":id,"sub":target})).await;
                assert_eq!(
                    read_sealed(&mut socket, &mut state).await,
                    json!({"id":id,"ok":true})
                );
                let snapshot = read_sealed(&mut socket, &mut state).await;
                assert_eq!(snapshot["sub"], id);
                assert!(snapshot["snapshot"].is_array());
            }
            assert_eq!(room.infos.receiver_count(), 1);
            assert_eq!(room.deltas.receiver_count(), 1);
            let pending = Arc::new(BlockedStop::default());
            *room.pending_stop.lock().unwrap() = Some(pending.clone());
            send_sealed(
                &mut socket,
                &mut state,
                json!({"id":3,"cmd":"computer.stop","params":{"personaId":persona}}),
            )
            .await;
            tokio::time::timeout(ANSWER, pending.entered.cancelled())
                .await
                .unwrap();
            // No timer or release in the handler: only dropping its future can end it.
            tokio::time::timeout(ANSWER, async {
                if disable {
                    h.remote.configure(false, network::ALL).await.unwrap();
                } else {
                    h.remote.revoke(&device.id).unwrap();
                }
                closed(&mut socket).await;
                pending.dropped.cancelled().await;
                loop {
                    if room.infos.receiver_count() == 0
                        && room.deltas.receiver_count() == 0
                        && h.remote.admission.counts() == (0, 0)
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }).await.expect("revocation must release command, socket, subscriptions and connection permits without waiting for the computer");
            if !disable {
                rejected(&h, &private, &public).await;
                h.remote.configure(false, network::ALL).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn cancelled_expired_and_wrong_secret_pairings_never_grant() {
    let h = Harness::new().await;
    let (private, _) = sealed::keypair().unwrap();
    let first = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (public, _) = invitation(&first);
    assert!(
        exchange(
            &h,
            "/v2/pair",
            &private,
            &public,
            br#"{"secret":"wrong","name":"x"}"#
        )
        .await
        .is_none()
    );
    assert!(h.remote.devices().is_empty());
    h.remote.cancel_pairing(&first.id).unwrap();
    assert!(claim(&h, &first, &private).await.is_none());
    let second = h.remote.pairing_v2(DeviceRole::Companion).unwrap();
    h.remote.cancel_pairing(&first.id).unwrap();
    assert!(
        h.remote.pairing_open(),
        "a stale cancel must not close a new invitation"
    );
    // Expiry is checked under the same lock as consumption, not only at HTTP upgrade.
    h.remote
        .state
        .lock()
        .unwrap()
        .sealed_pairing
        .as_mut()
        .unwrap()
        .expires_at = now() - 1;
    assert!(h.remote.pairing_result(&second.id).is_err());
    assert!(claim(&h, &second, &private).await.is_none());
    assert!(h.remote.devices().is_empty());
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn noise_handshakes_are_binary_bounded_and_expire_without_a_banner() {
    let h = Harness::new().await;
    for message in [
        Message::Text("not Noise".into()),
        Message::Binary(vec![0; 4097].into()),
    ] {
        let mut socket = h.socket_at("/v2", "").await.unwrap();
        let _ = socket.send(message).await;
        closed(&mut socket).await;
    }
    let mut socket = h.socket_at("/v2", "").await.unwrap();
    let result = tokio::time::timeout(ANSWER, socket.next()).await.unwrap();
    assert!(!matches!(
        result,
        Some(Ok(Message::Text(_) | Message::Binary(_)))
    ));
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn sealed_wire_rejects_plaintext_tampering_and_replay() {
    let h = Harness::new().await;
    let (private, _) = sealed::keypair().unwrap();
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (public, _) = invitation(&pairing);
    claim(&h, &pairing, &private).await.unwrap();
    for attack in ["text", "tamper", "replay", "oversize"] {
        let (mut socket, mut state, _) = exchange(&h, "/v2", &private, &public, &[]).await.unwrap();
        read_sealed(&mut socket, &mut state).await;
        let command = json!({"id":1,"cmd":"backends.list"}).to_string();
        let mut frame = sealed::encode(&mut state, &command).unwrap().remove(0);
        match attack {
            "text" => socket.send(Message::Text(command.into())).await.unwrap(),
            "tamper" => {
                frame[0] ^= 1;
                socket.send(Message::Binary(frame.into())).await.unwrap();
            }
            "replay" => {
                socket
                    .send(Message::Binary(frame.clone().into()))
                    .await
                    .unwrap();
                assert_eq!(read_sealed(&mut socket, &mut state).await["ok"], true);
                socket.send(Message::Binary(frame.into())).await.unwrap();
            }
            "oversize" => {
                let _ = socket.send(Message::Binary(vec![0u8; 65536].into())).await;
            }
            _ => unreachable!(),
        }
        closed(&mut socket).await;
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn tls_rotation_preserves_noise_identity_and_grants_across_restart() {
    let h = Harness::new().await;
    let (private, _) = sealed::keypair().unwrap();
    let pairing = h.remote.pairing_v2(DeviceRole::Companion).unwrap();
    claim(&h, &pairing, &private).await.unwrap();
    let keys = h.remote.noise_keys().unwrap();
    let before = h.remote.identity.read().unwrap().unwrap();
    h.remote.configure(false, network::ALL).await.unwrap();
    // Corrupt TLS, not Noise. A new certificate is not a new desk identity.
    h.remote.identity.write(b"broken TLS").unwrap();
    let reopened = Remote::open_with_store(
        h.root.path(),
        h.desk.log.clone(),
        h.desk.clone(),
        h.store.clone(),
    )
    .unwrap();
    reopened.configure(true, network::ALL).await.unwrap();
    assert_ne!(reopened.identity.read().unwrap().unwrap(), before);
    assert_eq!(reopened.noise_keys().unwrap(), keys);
    assert_eq!(reopened.devices().len(), 1);
    assert!(reopened.devices().iter().all(|d| d.public_key.is_some()));
    assert!(
        reopened
            .authenticate_v2(&sealed::keypair().unwrap().1)
            .is_none()
    );
    let public = reopened
        .devices()
        .into_iter()
        .find_map(|d| d.public_key)
        .unwrap();
    assert!(
        reopened
            .authenticate_v2(&URL_SAFE_NO_PAD.decode(public).unwrap())
            .is_some()
    );
    reopened.configure(false, network::ALL).await.unwrap();
    // An unreadable Noise key is not silently replaced either.
    reopened.noise_identity.write(b"broken Noise").unwrap();
    assert!(reopened.configure(true, network::ALL).await.is_err());
    assert!(!reopened.status().enabled);
}

#[tokio::test]
async fn sealed_computer_door_refuses_a_stopped_computer() {
    let (h, room) = Harness::with_computer().await;
    let phone = h.pair_companion().await;
    let (_, public) = h.remote.noise_keys().unwrap();
    *room.viewer.lock().unwrap() = None;
    let (mut socket, _, answer) = exchange(
        &h,
        "/v2/computer/ada/ws",
        &phone.private,
        &public,
        br#"{"purpose":"computer","personaId":"ada"}"#,
    )
    .await
    .unwrap();
    assert!(answer.is_empty());
    // Device authentication completes before the desk resolves the
    // viewer. A stopped computer sends no application frame.
    closed(&mut socket).await;
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn sealed_computer_handshake_binds_purpose_and_target_before_upstream() {
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = upstream.local_addr().unwrap().port();
    let (h, room) = Harness::with_computer().await;
    // The fake room reports a running viewer for both Ada and Bob, so a
    // rewritten target cannot be refused merely because Bob is missing.
    *room.viewer.lock().unwrap() = Some(format!("http://127.0.0.1:{port}/#secret-bearer"));
    let (private, _) = sealed::keypair().unwrap();
    let pairing = h.remote.pairing_v2(DeviceRole::Companion).unwrap();
    let (public, _) = invitation(&pairing);
    claim(&h, &pairing, &private).await.unwrap();

    let binding = br#"{"purpose":"computer","personaId":"ada"}"#;
    let refused: &[(&str, &[u8])] = &[
        // A proxy forwards Ada's unchanged Noise message to Bob's HTTP path.
        ("/v2/computer/bob/ws", binding),
        ("/v2/computer/ada/ws", b""),
        (
            "/v2/computer/ada/ws",
            br#"{"purpose":"wire","personaId":"ada"}"#,
        ),
        (
            "/v2/computer/ada/ws",
            br#"{"purpose":"computer","personaId":"Ada"}"#,
        ),
        (
            "/v2/computer/ada/ws",
            br#"{"purpose":"computer","personaId":""}"#,
        ),
        (
            "/v2/computer/ada/ws",
            br#"{"purpose":"computer","personaId":"ada","extra":true}"#,
        ),
        ("/v2/computer/ada/ws", br#"{"purpose":"computer"}"#),
        ("/v2/computer/ada/ws", br#"{"personaId":"ada"}"#),
        ("/v2/computer/ada/ws", b"not JSON"),
        ("/v2/computer/ada/ws", b"\xff"),
        // Redirecting a viewer to the ordinary wire must fail as well.
        ("/v2", binding),
    ];
    for (path, payload) in refused {
        assert!(
            exchange(&h, path, &private, &public, payload)
                .await
                .is_none(),
            "accepted {path} with {payload:?}"
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err(),
        "a refused binding reached the upstream computer"
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn sealed_computer_envelopes_hide_the_bearer_and_revoke_with_the_grant() {
    use base64::engine::general_purpose::STANDARD;
    let (port, asked) = fake_computer().await;
    let (h, room) = Harness::with_computer().await;
    *room.viewer.lock().unwrap() = Some(format!("http://127.0.0.1:{port}/#secret-bearer"));
    let (private, _) = sealed::keypair().unwrap();
    let pairing = h.remote.pairing_v2(DeviceRole::Companion).unwrap();
    let (public, _) = invitation(&pairing);
    let binding = br#"{"purpose":"computer","personaId":"ada"}"#;
    // Unknown keys cannot discover a running viewer or see its first frame,
    // even with a valid target binding.
    let (mut refused, _, answer) = exchange(&h, "/v2/computer/ada/ws", &private, &public, binding)
        .await
        .unwrap();
    assert_eq!(answer, sealed::DEVICE_REJECTED);
    closed(&mut refused).await;
    assert!(asked.lock().unwrap().is_none());
    claim(&h, &pairing, &private).await.unwrap();
    let (mut socket, mut state, _) =
        exchange(&h, "/v2/computer/ada/ws", &private, &public, binding)
            .await
            .unwrap();
    assert_eq!(
        read_sealed(&mut socket, &mut state).await,
        json!({"type":"binary", "data":STANDARD.encode(b"\x89PNG frame")})
    );
    send_sealed(
        &mut socket,
        &mut state,
        json!({"type":"text", "data":"{\"t\":\"control\",\"take\":true}"}),
    )
    .await;
    assert_eq!(
        read_sealed(&mut socket, &mut state).await,
        json!({"type":"text", "data":"{\"t\":\"control\",\"take\":true}"})
    );
    assert_eq!(
        asked.lock().unwrap().as_deref(),
        Some("token=secret-bearer")
    );
    let device = h.remote.pairing_result(&pairing.id).unwrap().unwrap();
    h.remote.revoke(&device.id).unwrap();
    closed(&mut socket).await;
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[test]
fn missing_roles_migrate_to_owner_but_never_create_an_empty_room_grant() {
    let old: RemoteDevice =
        serde_json::from_value(json!({"id":"old", "name":"Phone", "pairedAt":0})).unwrap();
    assert_eq!(old.role, DeviceRole::Owner);
    assert!(old.public_key.is_none());
    assert!(Saved::default().grants.is_empty());
}
