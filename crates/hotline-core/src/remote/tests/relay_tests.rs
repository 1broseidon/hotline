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
    let mut invitation = h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload;
    invitation.url = url.clone();
    let desk = phone.pair(&invitation, "Relayed phone").await.unwrap();
    assert_eq!(desk.desk_id, h.remote.status_desktop_id());
    assert_eq!(desk.url, url);
    let mut session = phone.connect(&desk);
    let first = hello(&mut session).await;
    assert_eq!(first["type"], "hello");
    assert!(first["endpoints"].as_array().unwrap().contains(&json!(url)));

    // A desk paired at its own address falls back to the relay it named.
    let mut direct = desk.clone();
    direct.url = "https://127.0.0.1:9".into();
    direct.relay = Some(url.clone());
    drop(phone.open(&direct, None).await.unwrap());
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
