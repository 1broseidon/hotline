//! A desk with a relay is reached through a served desk that carries only
//! sealed records: the visitor still pins the desk, never the relay.
use super::*;

async fn served() -> (tempfile::TempDir, Arc<Remote>, String) {
    let root = tempfile::tempdir().unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let listen = reserved.local_addr().unwrap();
    drop(reserved);
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(Desk::open_with_store(root.path(), store.clone()).unwrap());
    let options = ServeOptions {
        listen,
        public_url: format!("https://{listen}/room"),
        tls_cert: None,
        tls_key: None,
    };
    let remote =
        Remote::open_served_with_store(root.path(), desk.log.clone(), desk, store, options.clone())
            .unwrap();
    remote.restore().await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !remote.status().enabled {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (root, remote, options.public_url)
}

/// The desk pairs with the served desk as `role`, as the shell would, and
/// keeps it in the room's registry.
async fn pair_desk(h: &Harness, relay: &Remote, role: DeviceRole) -> client::PairedDesk {
    let invitation = relay.pairing_v2(role).unwrap();
    let paired = client::Client::new(h.store.clone())
        .pair(&invitation.payload, "Relayed laptop")
        .await
        .unwrap();
    fs::write(
        h.root.path().join("desks.json"),
        serde_json::to_vec(&vec![paired.clone()]).unwrap(),
    )
    .unwrap();
    paired
}

async fn standing(h: &Harness) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(url) = h.remote.status().relay.and_then(|relay| relay.url) {
                return url;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the desk never stood in on its relay")
}

async fn hello(session: &mut client::Session) -> Value {
    serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(15), session.incoming.recv())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

async fn control_notice(control: &mut client::Connection) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Some(Ok(Message::Text(text))) = control.next().await else {
                panic!("the relay closed its control socket");
            };
            let notice: Value = serde_json::from_str(&text).unwrap();
            if notice["type"] != "ping" {
                return notice;
            }
        }
    })
    .await
    .expect("the relay never sent a control notice")
}

/// Visitors trust TLS here only to reach the disposable relay; the paired
/// client separately proves the relay's Noise identity when claiming a visit.
async fn relay_socket(relay: &Remote, public_url: &str, path: &str) -> Socket {
    let identity: Identity =
        serde_json::from_slice(&relay.identity.read().unwrap().unwrap()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(
            identity.certificate,
        ))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let url = url::Url::parse(public_url).unwrap();
    let name = rustls::pki_types::ServerName::try_from(
        url.host_str().unwrap().trim_matches(['[', ']']).to_owned(),
    )
    .unwrap();
    let stream = TcpStream::connect(url.socket_addrs(|| None).unwrap()[0])
        .await
        .unwrap();
    let tls = TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .unwrap();
    let request = format!("{}{path}", public_url.replace("https:", "wss:"));
    tokio_tungstenite::client_async(request, tls)
        .await
        .unwrap()
        .0
}

async fn visit(
    relay: &Remote,
    public_url: &str,
    desk_id: &str,
    control: &mut client::Connection,
) -> (Socket, String) {
    let socket = relay_socket(relay, public_url, &format!("/relay/{desk_id}/v2")).await;
    let notice = control_notice(control).await;
    assert_eq!(notice["type"], "visit");
    assert_eq!(notice["path"], "/v2");
    (socket, notice["id"].as_str().unwrap().to_owned())
}

async fn closed<S>(socket: &mut WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let next = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("the cancelled relay wire remained open");
    assert!(matches!(
        next,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
}

async fn control_closed(control: &mut client::Connection) {
    let next = tokio::time::timeout(Duration::from_secs(3), control.next())
        .await
        .expect("the old relay generation remained open");
    assert!(matches!(
        next,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
}

async fn callback_refused(client: &client::Client, desk: &client::PairedDesk, capability: &str) {
    assert!(
        tokio::time::timeout(
            Duration::from_secs(3),
            client.accept_relay(desk, capability)
        )
        .await
        .expect("a refused relay callback did not close")
        .is_err()
    );
}

async fn hub_counts(relay: &Remote, expected: (usize, usize)) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while relay.relay.counts() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("relay registrations or pending visits did not retire");
}

#[tokio::test]
async fn a_phone_pairs_and_talks_to_a_desk_through_its_relay() {
    let (_relay_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let status = h
        .remote
        .relay_through(Some(server.desk_id.clone()))
        .unwrap();
    assert_eq!(status.relay.as_ref().unwrap().desk_id, server.desk_id);
    let url = standing(&h).await;
    assert_eq!(
        url,
        format!("{public_url}/relay/{}", h.remote.status_desktop_id())
    );
    assert!(h.remote.status().endpoints.contains(&url));

    // Pairing and the wire both cross the relay, against the desk's own key.
    let phone = client::Client::new(Arc::new(MemoryStore::default()));
    h.remote.state.lock().unwrap().endpoints = vec!["https://127.0.0.1:9".into()];
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload;
    assert_eq!(invitation.relay.as_deref(), Some(url.as_str()));
    let desk = phone.pair(&invitation, "Relayed phone").await.unwrap();
    assert_eq!(desk.desk_id, h.remote.status_desktop_id());
    assert_eq!(desk.url, invitation.url);
    assert_eq!(desk.relay, invitation.relay);
    let mut session = phone.connect(&desk);
    let first = hello(&mut session).await;
    assert_eq!(first["type"], "hello");
    assert!(first["endpoints"].as_array().unwrap().contains(&json!(url)));

    // A desk paired at its own address falls back to the relay it named.
    drop(phone.open(&desk, None).await.unwrap());
    session
        .outgoing
        .send(json!({"id":1,"cmd":"backends.list"}).to_string())
        .await
        .unwrap();
    loop {
        if hello(&mut session).await["id"] == 1 {
            break;
        }
    }

    // A relay that answers with any other key is refused before a frame.
    let mut forged = desk.clone();
    forged.desk_key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
    assert!(phone.open(&forged, None).await.is_err());

    // Turning Remote off takes the desk off the relay, and visitors with it.
    h.remote.configure(false, network::ALL).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while relay.relay.counts().0 != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(phone.open(&desk, None).await.is_err());
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn only_an_owners_device_stands_in_and_only_for_a_desk_it_names() {
    let (_relay_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Companion).await;
    h.remote
        .relay_through(Some(server.desk_id.clone()))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while h.remote.status().relay.unwrap().error.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(h.remote.status().relay.unwrap().url.is_none());
    assert_eq!(relay.relay.counts(), (0, 0));

    // Nobody stands in for an unknown desk; the door says so plainly.
    let http = reqwest::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
    let missing = http
        .get(format!("{public_url}/relay/{}/v2", Uuid::new_v4()))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    for path in ["/relay/accept/nope", "/relay/desk/ws", "/relay/desk"] {
        let status = http
            .get(format!("{public_url}{path}"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 404, "{path}");
    }
    assert!(h.remote.relay_through(Some("never-paired".into())).is_err());
    assert!(relay.relay_through(None).is_err());
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn a_relay_callback_proves_the_hosting_device_before_switching_to_sealed_records() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;

    for role in [DeviceRole::Owner, DeviceRole::Companion] {
        let other = client::Client::new(Arc::new(MemoryStore::default()));
        let invitation = relay.pairing_v2(role).unwrap().payload;
        let other_desk = other.pair(&invitation, "Another device").await.unwrap();
        callback_refused(&other, &other_desk, &capability).await;
        assert_eq!(relay.relay.counts(), (1, 1));
    }

    // A known capability is insufficient without a granted static key and
    // the relay's pinned responder key. Neither refusal spends the visit.
    let (_, relay_key) = relay.noise_keys().unwrap();
    for expected_key in [relay_key, [9u8; 32]] {
        let (private, _) = sealed::keypair().unwrap();
        let mut noise = sealed_network::initiator(&private, &expected_key);
        let payload = json!({"purpose":"relay-accept", "capability":capability}).to_string();
        let mut buffer = [0; 4096];
        let size = noise
            .write_message(payload.as_bytes(), &mut buffer)
            .unwrap();
        let mut socket = relay_socket(&relay, &public_url, "/v2/relay/accept").await;
        socket
            .send(Message::Binary(buffer[..size].to_vec().into()))
            .await
            .unwrap();
        closed(&mut socket).await;
        assert_eq!(relay.relay.counts(), (1, 1));
    }

    let mut callback = owner.accept_relay(&server, &capability).await.unwrap();
    hub_counts(&relay, (1, 0)).await;
    let record = vec![1, 7, 3, 9];
    visitor
        .send(Message::Binary(record.clone().into()))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), callback.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Binary(record.into())
    );
    callback
        .send(Message::Binary(vec![4, 2, 8].into()))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), visitor.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Binary(vec![4, 2, 8].into())
    );
    callback_refused(&owner, &server, &capability).await;
    drop(callback);
    closed(&mut visitor).await;
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn replacing_a_relay_generation_cancels_its_joined_and_waiting_visitors() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut old = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut old).await["type"], "ready");
    let (mut joined, capability) = visit(&relay, &public_url, &desk_id, &mut old).await;
    let mut callback = owner.accept_relay(&server, &capability).await.unwrap();
    let (mut waiting, stale_capability) = visit(&relay, &public_url, &desk_id, &mut old).await;

    let mut current = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut current).await["type"], "ready");
    control_closed(&mut old).await;
    closed(&mut joined).await;
    closed(&mut callback).await;
    closed(&mut waiting).await;
    hub_counts(&relay, (1, 0)).await;
    callback_refused(&owner, &server, &stale_capability).await;

    // Cleanup from the replaced host must not remove the current registration.
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut current).await;
    drop(owner.accept_relay(&server, &capability).await.unwrap());
    closed(&mut visitor).await;
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn an_unclaimed_relay_capability_expires_and_cannot_be_recovered() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;

    // Delay-free expiry leaves the pending entry in place, so the callback
    // must check its deadline rather than relying on the cleanup timer.
    relay.relay.expire_capability(&capability);
    assert_eq!(relay.relay.counts(), (1, 1));
    callback_refused(&owner, &server, &capability).await;
    closed(&mut visitor).await;
    hub_counts(&relay, (1, 0)).await;
    let (mut next, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;
    drop(owner.accept_relay(&server, &capability).await.unwrap());
    closed(&mut next).await;
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn restarting_remote_invalidates_a_pending_relay_callback() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;

    relay.configure(false, &relay.status().host).await.unwrap();
    control_closed(&mut control).await;
    closed(&mut visitor).await;
    hub_counts(&relay, (0, 0)).await;
    relay.configure(true, &relay.status().host).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !relay.status().enabled {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    callback_refused(&owner, &server, &capability).await;

    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;
    drop(owner.accept_relay(&server, &capability).await.unwrap());
    closed(&mut visitor).await;
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn restarting_the_desktop_cancels_a_claimed_callback_before_its_inner_handshake() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (private, _) = sealed::keypair().unwrap();
    sealed_network::claim(&h, &pairing, &private).await.unwrap();
    let (_, desk_key) = h.remote.noise_keys().unwrap();
    h.remote.relay_through(Some(server.desk_id)).unwrap();
    standing(&h).await;
    let path = format!("/relay/{}/v2", h.remote.status_desktop_id());
    let mut old = relay_socket(&relay, &public_url, &path).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while relay.relay.counts() != (1, 0)
            || relay.relay.active_visits() != 1
            || h.remote.relayed.available_permits() != 31
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the desktop did not claim the waiting callback");

    // The visitor has sent no inner Noise bytes. Its claimed callback must
    // retain the old desktop generation even if Remote immediately restarts.
    h.remote.configure(false, network::ALL).await.unwrap();
    h.remote.configure(true, network::ALL).await.unwrap();
    standing(&h).await;
    closed(&mut old).await;
    let mut noise = sealed_network::initiator(&private, &desk_key);
    let mut buffer = [0; 4096];
    let size = noise.write_message(&[], &mut buffer).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(3),
            old.send(Message::Binary(buffer[..size].to_vec().into()))
        )
        .await
        .unwrap()
        .is_err(),
        "a callback from before Remote restarted accepted an inner handshake"
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while relay.relay.active_visits() != 0 || h.remote.relayed.available_permits() != 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the old callback retained a relay or desktop visit budget");

    // The saved device grant still works through a callback from the new
    // generation, so restart invalidates the old visit rather than the device.
    let mut fresh = relay_socket(&relay, &public_url, &path).await;
    let mut noise = sealed_network::initiator(&private, &desk_key);
    let size = noise.write_message(&[], &mut buffer).unwrap();
    fresh
        .send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let Some(Ok(Message::Binary(answer))) =
        tokio::time::timeout(Duration::from_secs(3), fresh.next())
            .await
            .unwrap()
    else {
        panic!("a new callback did not finish its inner handshake");
    };
    assert_eq!(noise.read_message(&answer, &mut buffer).unwrap(), 0);
    let mut transport = noise.into_transport_mode().unwrap();
    assert_eq!(
        sealed_network::read_sealed(&mut fresh, &mut transport).await["type"],
        "hello"
    );
    h.remote.relay_through(None).unwrap();
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn deselecting_the_relay_closes_a_live_wire_even_while_remote_stays_on() {
    let (_root, relay, _public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    h.remote.relay_through(Some(server.desk_id)).unwrap();
    let url = standing(&h).await;
    h.remote.state.lock().unwrap().endpoints = vec!["https://127.0.0.1:9".into()];
    let invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload;
    let phone = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = phone
        .pair(&invitation, "Phone through relay")
        .await
        .unwrap();
    let mut wire = phone.open(&desk, None).await.unwrap();
    let Some(Ok(Message::Text(text))) = wire.next().await else {
        panic!("the relayed desk did not send a hello");
    };
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["type"],
        "hello"
    );
    assert_eq!(desk.relay.as_deref(), Some(url.as_str()));

    h.remote.relay_through(None).unwrap();
    assert!(h.remote.status().enabled);
    assert!(h.remote.status().relay.is_none());
    control_closed(&mut wire).await;
    hub_counts(&relay, (0, 0)).await;
    assert!(phone.open(&desk, None).await.is_err());
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn revoking_a_relay_host_releases_a_joined_visit_while_its_peer_is_not_reading() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let device_id = relay.devices()[0].id.clone();
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let (mut visitor, capability) = visit(&relay, &public_url, &desk_id, &mut control).await;
    let callback = owner.accept_relay(&server, &capability).await.unwrap();
    assert_eq!(relay.relay.active_visits(), 1);

    // Keep the callback unread until both TLS/socket buffers fill. This makes
    // the relay's next forwarded send block independently of the host control.
    let sent = Arc::new(AtomicUsize::new(0));
    let progress = sent.clone();
    let writer = tokio::spawn(async move {
        let record = vec![0u8; 65_535];
        while visitor
            .send(Message::Binary(record.clone().into()))
            .await
            .is_ok()
        {
            progress.fetch_add(1, Ordering::SeqCst);
        }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let before = sent.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            if before > 0 && sent.load(Ordering::SeqCst) == before {
                assert!(
                    !writer.is_finished(),
                    "the writer closed before backpressure"
                );
                break;
            }
        }
    })
    .await
    .expect("the unread callback never exerted backpressure");
    assert_eq!(relay.relay.active_visits(), 1);

    relay.revoke(&device_id).unwrap();
    tokio::time::timeout(Duration::from_secs(12), async {
        while relay.relay.active_visits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("revocation left a joined visit budget blocked on its peer");
    hub_counts(&relay, (0, 0)).await;
    control_closed(&mut control).await;
    writer.abort();
    let _ = writer.await;
    drop(callback);
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn selecting_a_new_relay_keeps_the_desktops_existing_session_budget() {
    let (_root, relay, _public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    h.remote
        .relay_through(Some(server.desk_id.clone()))
        .unwrap();
    standing(&h).await;
    let mut visits: Vec<_> = (0..32)
        .map(|_| h.remote.relayed.clone().try_acquire_owned().unwrap())
        .collect();
    assert!(h.remote.relayed.clone().try_acquire_owned().is_err());

    h.remote.relay_through(None).unwrap();
    assert!(h.remote.relayed.clone().try_acquire_owned().is_err());
    h.remote.relay_through(Some(server.desk_id)).unwrap();
    standing(&h).await;
    assert!(h.remote.relayed.clone().try_acquire_owned().is_err());
    drop(visits.pop());
    drop(h.remote.relayed.clone().try_acquire_owned().unwrap());
    h.remote.relay_through(None).unwrap();
    relay.configure(false, &relay.status().host).await.unwrap();
}

#[tokio::test]
async fn a_relay_visitors_rate_budget_returns_http_429_after_ten_arrivals() {
    let (_root, relay, public_url) = served().await;
    let h = Harness::new().await;
    let server = pair_desk(&h, &relay, DeviceRole::Owner).await;
    let owner = client::Client::new(h.store.clone());
    let desk_id = h.remote.status_desktop_id();
    let mut control = owner.host_relay(&server, &desk_id).await.unwrap();
    assert_eq!(control_notice(&mut control).await["type"], "ready");
    let mut visitors = Vec::new();
    for _ in 0..10 {
        visitors.push(visit(&relay, &public_url, &desk_id, &mut control).await.0);
    }
    assert_eq!(relay.relay.counts(), (1, 10));
    let response = reqwest::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
        .get(format!("{public_url}/relay/{desk_id}/v2"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"],
        "desk_busy"
    );
    assert_eq!(relay.relay.counts(), (1, 10));
    relay.configure(false, &relay.status().host).await.unwrap();
    drop(visitors);
}
