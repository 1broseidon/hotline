use super::*;

#[tokio::test]
async fn served_listener_waits_for_exact_socket_and_has_no_legacy_routes() {
    let root = tempfile::tempdir().unwrap();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen = occupied.local_addr().unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(Desk::open_with_store(root.path(), store.clone()).unwrap());
    let options = ServeOptions {
        listen,
        public_url: format!("https://{listen}/room"),
        tls_cert: None,
        tls_key: None,
        relay: true,
    };
    let remote = Remote::open_served_with_store(
        root.path(),
        Some(Agents {
            log: desk.log.clone(),
            room: desk.clone(),
        }),
        store,
        options.clone(),
    )
    .unwrap();
    assert!(!remote.status().enabled);
    tokio::time::timeout(ANSWER, remote.restore())
        .await
        .unwrap();
    assert!(!remote.status().enabled);
    assert!(remote.status().error.unwrap().contains("Waiting"));
    assert!(remote.configure(true, "0.0.0.0").await.is_err());
    assert!(remote.pairing_v2(DeviceRole::Owner).is_err());
    drop(occupied);
    tokio::time::timeout(ANSWER, async {
        while !remote.status().enabled {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(remote.status().host, listen.to_string());
    assert_eq!(
        remote.status().endpoint.as_deref(),
        Some(options.public_url.as_str())
    );
    let identity: Identity =
        serde_json::from_slice(&remote.identity.read().unwrap().unwrap()).unwrap();
    let http = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(&identity.certificate).unwrap())
        .build()
        .unwrap();
    for path in ["/pair", "/pair/manual/start", "/pair/manual/finish"] {
        assert_eq!(
            http.post(format!("{}{path}", options.public_url))
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    for path in ["/ws", "/computer/ada/ws", "/v2/pair"] {
        assert_eq!(
            http.get(format!("{}{path}", options.public_url))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    remote.configure(false, &listen.to_string()).await.unwrap();
    assert!(TcpStream::connect(listen).await.is_err());
}

#[test]
fn served_listen_validation_never_permits_wildcard_or_implicit_tls() {
    let mut options = ServeOptions {
        listen: "127.0.0.1:8788".parse().unwrap(),
        public_url: "https://desk.example/room".into(),
        tls_cert: None,
        tls_key: None,
        relay: true,
    };
    assert!(options.validate().is_ok());
    for address in [
        "0.0.0.0:8788",
        "[::]:8788",
        "127.0.0.1:0",
        "[::ffff:127.0.0.1]:8788",
    ] {
        options.listen = address.parse().unwrap();
        assert!(options.validate().is_err(), "{address}");
    }
    options.listen = "127.0.0.1:8788".parse().unwrap();
    for url in [
        "http://desk.example",
        "https://user:password@desk.example",
        "https://desk.example/?token=x",
        "https://desk.example/#secret",
    ] {
        options.public_url = url.into();
        assert!(options.validate().is_err(), "{url}");
    }
    options.public_url = "https://desk.example".into();
    options.tls_cert = Some("cert.pem".into());
    assert!(options.validate().is_err());
}

#[tokio::test]
async fn served_listener_uses_supplied_pem_certificate_and_key() {
    let root = tempfile::tempdir().unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let listen = reserved.local_addr().unwrap();
    drop(reserved);
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let cert_path = root.path().join("cert.pem");
    let key_path = root.path().join("key.pem");
    fs::write(&cert_path, cert.cert.pem()).unwrap();
    fs::write(&key_path, cert.signing_key.serialize_pem()).unwrap();
    let store = Arc::new(MemoryStore::default());
    let desk = Arc::new(Desk::open_with_store(root.path(), store.clone()).unwrap());
    let options = ServeOptions {
        listen,
        public_url: format!("https://{listen}"),
        tls_cert: Some(cert_path),
        tls_key: Some(key_path),
        relay: true,
    };
    let remote = Remote::open_served_with_store(
        root.path(),
        Some(Agents {
            log: desk.log.clone(),
            room: desk,
        }),
        store,
        options.clone(),
    )
    .unwrap();
    remote.restore().await;
    tokio::time::timeout(ANSWER, async {
        while !remote.status().enabled {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(cert.cert.der()).unwrap())
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{}/v2/pair", options.public_url))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(
        remote.identity.read().unwrap().is_none(),
        "supplied TLS must not create an unrelated self-signed identity"
    );
    assert!(remote.noise_identity.read().unwrap().is_some());
    remote.configure(false, &listen.to_string()).await.unwrap();
}
