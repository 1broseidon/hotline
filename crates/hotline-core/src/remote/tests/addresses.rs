use super::*;

#[tokio::test]
async fn all_interfaces_are_the_default_and_every_advertised_ip_has_valid_tls() {
    let h = Harness::on_interfaces().await;
    let status = h.remote.status();
    assert_eq!(status.host, network::ALL);
    assert!(status.enabled);
    assert_eq!(status.endpoints.len(), status.addresses.len());
    let mut answered = Vec::new();
    for endpoint in &status.endpoints {
        let sent = h
            .http
            .get(format!("{endpoint}/missing"))
            .header("connection", "close")
            .timeout(Duration::from_secs(3))
            .send()
            .await;
        // A point-to-point VPN address this host has no route back to (a
        // utun some VPNs bring up) times out from here; that says nothing
        // about its certificate, which every address that answers must pass.
        let response = match sent {
            Err(error) if error.is_timeout() => continue,
            sent => sent.unwrap(),
        };
        assert_eq!(response.status(), 404);
        answered.push(endpoint);
    }
    assert!(!answered.is_empty(), "no advertised address answered");
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (noise_public, _) = sealed_network::invitation(&pairing);
    let pairing_url = url::Url::parse(&pairing.url).unwrap();
    assert_eq!(
        pairing_url
            .query_pairs()
            .find(|(key, _)| key == "u")
            .unwrap()
            .1,
        status.endpoint.clone().unwrap()
    );
    for host in ["127.0.0.1", "127.2.3.4", "::1", "0.0.0.0", "::"] {
        assert!(h.remote.configure(true, host).await.is_err());
        assert_eq!(h.remote.status().endpoints, status.endpoints);
    }
    let grant = h.pair().await;
    let specific = status.addresses[0].clone();
    let narrowed = h.remote.configure(true, &specific).await.unwrap();
    assert_eq!(narrowed.host, specific);
    assert_eq!(narrowed.endpoints, vec![status.endpoint.clone().unwrap()]);
    assert!(h.remote.authenticate_v2(&grant.public).is_some());
    assert_eq!(h.remote.noise_keys().unwrap().1, noise_public);
    // Only an address that answered can show the narrowed listener refusing.
    for other in answered
        .into_iter()
        .filter(|endpoint| **endpoint != narrowed.endpoints[0])
    {
        let result = h
            .http
            .get(format!("{other}/missing"))
            .timeout(Duration::from_secs(1))
            .send()
            .await;
        assert!(result.is_err(), "restricted listener accepted {other}");
    }
    let off = h.remote.configure(false, network::ALL).await.unwrap();
    assert!(!off.enabled);
    assert_eq!(off.host, network::ALL);
    assert!(off.endpoints.is_empty());
    assert!(h.remote.authenticate_v2(&grant.public).is_none());
    assert_eq!(
        h.remote
            .configure(true, network::ALL)
            .await
            .unwrap()
            .endpoints,
        status.endpoints
    );
    assert!(h.remote.authenticate_v2(&grant.public).is_some());
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_public_address_is_advertised_after_this_computers_own_and_never_replaces_the_pairing_address()
 {
    let h = Harness::new().await;
    let before = h.remote.status();
    assert_eq!(before.public_url, None);
    let set = h
        .remote
        .set_public_url(Some("  https://Desk.Example.com/  "))
        .unwrap();
    assert_eq!(set.public_url.as_deref(), Some("https://desk.example.com"));
    // This computer's own addresses lead; the public one follows them.
    let own = before.endpoints.len();
    assert_eq!(set.endpoints[..own], before.endpoints[..]);
    assert_eq!(set.endpoints[own], "https://desk.example.com");
    assert_eq!(set.endpoint, before.endpoint);
    // Pairing still happens on the network the phone is standing in.
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let pairing_url = url::Url::parse(&pairing.url).unwrap();
    let paired_on = pairing_url
        .query_pairs()
        .find(|(key, _)| key == "u")
        .unwrap()
        .1;
    assert_eq!(paired_on, before.endpoint.clone().unwrap());
    // A port is kept; the default one is dropped as the origin spells it.
    let ported = h
        .remote
        .set_public_url(Some("https://desk.example.com:8443"))
        .unwrap();
    assert_eq!(
        ported.public_url.as_deref(),
        Some("https://desk.example.com:8443")
    );
    assert_eq!(
        h.remote
            .set_public_url(Some("https://desk.example.com:443"))
            .unwrap()
            .public_url
            .as_deref(),
        Some("https://desk.example.com")
    );
    // Anything that is not a bare https origin is refused, and the kept one stays.
    for refused in [
        "http://desk.example.com",
        "desk.example.com",
        "https://desk.example.com/hotline",
        "https://user:pass@desk.example.com",
        "https://desk.example.com/?a=b",
        "https://desk.example.com/#x",
        "wss://desk.example.com",
        "https://",
    ] {
        assert!(h.remote.set_public_url(Some(refused)).is_err(), "{refused}");
        assert_eq!(
            h.remote.status().public_url.as_deref(),
            Some("https://desk.example.com")
        );
    }
    // Kept with Remote off, advertised only while it is on, and kept across a restart.
    let off = h.remote.configure(false, network::ALL).await.unwrap();
    assert!(off.endpoints.is_empty());
    assert_eq!(off.public_url.as_deref(), Some("https://desk.example.com"));
    let restored = Remote::open_with_store(
        h.root.path(),
        h.desk.log.clone(),
        h.desk.clone(),
        h.store.clone(),
    )
    .unwrap();
    assert_eq!(
        restored.status().public_url.as_deref(),
        Some("https://desk.example.com")
    );
    // Blank or none clears it.
    for cleared in [Some("   "), None] {
        h.remote
            .set_public_url(Some("https://desk.example.com"))
            .unwrap();
        assert_eq!(h.remote.set_public_url(cleared).unwrap().public_url, None);
    }
}

#[tokio::test]
async fn the_pairing_qr_offers_the_public_address_after_the_local_one_and_before_the_relay() {
    let h = Harness::new().await;
    let params = |role| {
        let pairing = h.remote.pairing_v2(role).unwrap();
        let params: HashMap<String, String> = url::Url::parse(&pairing.url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        (params, pairing.payload)
    };
    let local = h.remote.status().endpoint.unwrap();
    // Without a public address the QR is the local one alone.
    let (plain, payload) = params(DeviceRole::Owner);
    assert_eq!(plain["u"], local);
    assert!(!plain.contains_key("x") && !plain.contains_key("a"));
    assert_eq!(payload.public, None);
    // A public address rides along as `x`; `u` stays this computer's own.
    h.remote
        .set_public_url(Some("https://desk.example.com"))
        .unwrap();
    let (public, payload) = params(DeviceRole::Companion);
    assert_eq!(public["u"], local);
    assert_eq!(public["x"], "https://desk.example.com");
    assert_eq!(public["r"], "companion");
    assert_eq!(payload.public.as_deref(), Some("https://desk.example.com"));
    // With a relay as well, each keeps its own parameter.
    let relay = "https://relay.example/room/relay/desk-1".to_string();
    h.remote.state.lock().unwrap().relay.url = Some(relay.clone());
    let (both, payload) = params(DeviceRole::Owner);
    assert_eq!(both["u"], local);
    assert_eq!(both["x"], "https://desk.example.com");
    assert_eq!(both["a"], relay);
    assert_eq!(payload.relay, Some(relay));
    // A public address that is the pairing address is not repeated.
    h.remote.set_public_url(Some(&local)).unwrap();
    let (same, payload) = params(DeviceRole::Owner);
    assert!(!same.contains_key("x"));
    assert_eq!(payload.public, None);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn rust_client_pairs_and_connects_through_the_public_address_when_its_own_does_not_answer() {
    let h = Harness::new().await;
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let mut payload = h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload;
    // The device is away from the desk's network: its own address refuses,
    // and the public one (a tunnel to the same listener) answers.
    payload.public = Some(payload.url.clone());
    payload.url = "https://127.0.0.1:1".into();
    let desk = client.pair(&payload, "Away laptop").await.unwrap();
    assert_eq!(desk.desk_id, h.remote.status_desktop_id());
    assert_eq!(desk.public, payload.public);
    let mut session = client.connect(&desk);
    let hello = tokio::time::timeout(ANSWER, session.incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&hello).unwrap()["type"],
        "hello"
    );
    drop(session);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_saved_loopback_selection_migrates_to_all_host_ips() {
    let h = Harness::on_interfaces().await;
    h.remote.configure(false, network::ALL).await.unwrap();
    let path = h.root.path().join("remote.json");
    let mut saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    saved.host = "127.0.0.1".into();
    atomic_write(&path, &serde_json::to_vec(&saved).unwrap()).unwrap();
    let restored = Remote::open_with_store(
        h.root.path(),
        h.desk.log.clone(),
        h.desk.clone(),
        h.store.clone(),
    )
    .unwrap();
    assert_eq!(restored.status().host, network::ALL);
    assert!(!restored.status().enabled);
    assert!(
        restored
            .status()
            .addresses
            .iter()
            .all(|host| network::reachable(host.parse().unwrap()))
    );
    restored.configure(true, network::ALL).await.unwrap();
    let saved: Saved = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(saved.host, network::ALL);
    restored.configure(false, network::ALL).await.unwrap();
}

/// A tunnel on this computer, as a person runs one for a Public address: it
/// forwards each connection, TLS untouched, to wherever it is pointed.
struct Tunnel {
    url: String,
    target: Arc<std::sync::Mutex<String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Tunnel {
    async fn to(target: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let target = Arc::new(std::sync::Mutex::new(target.to_owned()));
        let pointed = target.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let target = pointed.lock().unwrap().clone();
                tokio::spawn(async move {
                    let address = target.trim_start_matches("https://");
                    if let Ok(mut outbound) = TcpStream::connect(address).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
            }
        });
        Self { url, target, task }
    }
    fn point(&self, target: &str) {
        *self.target.lock().unwrap() = target.into();
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
/// What the Public address hint in RemoteSection tells a person to point a
/// tunnel at; ui/tests/remote-section.test.tsx proves the window says it.
fn tunnel_hint(status: &RemoteStatus) -> String {
    let endpoint = status.endpoint.clone().unwrap();
    if status.host == network::ALL {
        let port = url::Url::parse(&endpoint).unwrap().port().unwrap();
        format!("https://localhost:{port}")
    } else {
        endpoint
    }
}
async fn sealed_via(h: &Harness, base: &str, private: &[u8; 32]) -> PhoneSocket {
    let (_, desk_key) = h.remote.noise_keys().unwrap();
    let (socket, state, payload) =
        sealed_network::exchange_via(h, base, "/v2", private, &desk_key, &[])
            .await
            .unwrap();
    assert!(payload.is_empty());
    PhoneSocket { socket, state }
}

/// Epic-10 on the desk side: a phone off the desk's networks pairs from the
/// QR through the Public address, and its hello and device list tell it
/// which desk it reached, which endpoint is public, and which grant is its own.
#[tokio::test]
async fn a_phone_away_from_home_pairs_through_the_public_address_and_sees_itself_and_the_desk() {
    let h = Harness::on_interfaces().await;
    // A phone paired at home, so the mark has a grant to leave off.
    let home = h.pair().await;
    // On all host IPs the hint names localhost, and a tunnel pointed there
    // reaches the desk.
    let status = h.remote.status();
    let port = url::Url::parse(&h.endpoint()).unwrap().port().unwrap();
    assert_eq!(tunnel_hint(&status), format!("https://localhost:{port}"));
    let tunnel = Tunnel::to(&tunnel_hint(&status)).await;
    h.remote.set_public_url(Some(&tunnel.url)).unwrap();

    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let params: HashMap<String, String> = url::Url::parse(&pairing.url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    // Away from home, none of this computer's own addresses route. In one
    // process they would, so the phone passes over them as its network
    // would refuse them, trying the link's addresses in the order it must.
    let routes = |base: &str| {
        let url = url::Url::parse(base).unwrap();
        let host = url.host_str().unwrap().trim_matches(['[', ']']);
        !status.addresses.iter().any(|own| own == host)
    };
    assert!(!routes(&params["u"]));
    let base = ["u", "x", "a"]
        .into_iter()
        .filter_map(|key| params.get(key))
        .find(|base| routes(base))
        .unwrap();
    assert_eq!(base, &tunnel.url);

    let (desk_key, secret) = sealed_network::invitation(&pairing);
    let (private, _) = sealed::keypair().unwrap();
    let claim = json!({"secret": secret, "name": "Away phone"}).to_string();
    let (_, _, answer) =
        sealed_network::exchange_via(&h, base, "/v2/pair", &private, &desk_key, claim.as_bytes())
            .await
            .unwrap();
    let answer: Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(answer["role"], "owner", "{answer}");
    let ours = h.remote.pairing_result(&pairing.id).unwrap().unwrap().id;

    // The hello names the desk and marks as public the very address the QR
    // offered, because both read one Public address.
    let mut phone = sealed_via(&h, base, &private).await;
    let hello = read(&mut phone).await;
    assert_eq!(hello["deskKind"], "desktop");
    assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(hello["publicEndpoint"], params["x"].as_str());
    assert!(
        hello["endpoints"]
            .as_array()
            .unwrap()
            .contains(&hello["publicEndpoint"])
    );

    send(&mut phone, json!({"id": 1, "cmd": "remote.devices"})).await;
    let listed = read(&mut phone).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let devices = listed["result"].as_array().unwrap();
    let ids: Vec<_> = devices.iter().map(|device| &device["id"]).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&&json!(home.id)) && ids.contains(&&json!(ours)));
    for device in devices {
        let current = device["id"] == ours.as_str();
        assert_eq!(device.get("current"), current.then_some(&json!(true)));
    }
    drop(phone);

    // Listening on one address, the hint names that address and port, and
    // localhost no longer reaches the desk.
    let address = status.addresses[0].clone();
    let status = h.remote.configure(true, &address).await.unwrap();
    assert_eq!(tunnel_hint(&status), network::endpoint(&address, port));
    assert!(TcpStream::connect(("localhost", port)).await.is_err());
    tunnel.point(&tunnel_hint(&status));
    let mut phone = sealed_via(&h, base, &private).await;
    assert_eq!(
        read(&mut phone).await["publicEndpoint"],
        params["x"].as_str()
    );
    drop(phone);
    h.remote.configure(false, network::ALL).await.unwrap();
}
