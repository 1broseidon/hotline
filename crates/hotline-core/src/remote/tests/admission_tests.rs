use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;

/// Disposable TLS-terminating proxy. Every upstream connection has the same
/// loopback peer IP; HTTP keepalive and WebSocket upgrades pass through it.
struct Proxy {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
    tls: TlsConnector,
    name: rustls::pki_types::ServerName<'static>,
}
impl Proxy {
    async fn new(h: &Harness) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = url::Url::parse(&h.endpoint()).unwrap();
        let name = rustls::pki_types::ServerName::try_from(
            endpoint
                .host_str()
                .unwrap()
                .trim_matches(['[', ']'])
                .to_owned(),
        )
        .unwrap();
        let origin = endpoint.socket_addrs(|| None).unwrap()[0];
        let identity = serde_json::from_slice(&h.remote.identity.read().unwrap().unwrap()).unwrap();
        let acceptor = server::tls(&identity).unwrap();
        let connector = h.tls.clone();
        let upstream_name = name.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (socket, _) = result.unwrap();
                        let acceptor = acceptor.clone();
                        let connector = connector.clone();
                        let name = upstream_name.clone();
                        connections.spawn(async move {
                            let Ok(mut client) = acceptor.accept(socket).await else { return; };
                            let socket = TcpStream::connect(origin).await.unwrap();
                            let Ok(mut desk) = connector.connect(name, socket).await else { return; };
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut desk).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {},
                }
            }
        });
        Self {
            address,
            task,
            tls: h.tls.clone(),
            name,
        }
    }
    async fn tls(&self) -> TlsStream<TcpStream> {
        self.tls
            .connect(
                self.name.clone(),
                TcpStream::connect(self.address).await.unwrap(),
            )
            .await
            .unwrap()
    }
    async fn socket(&self, path: &str, bearer: &str) -> Socket {
        let mut request = format!("wss://{}{path}", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
        tokio_tungstenite::client_async(request, self.tls().await)
            .await
            .unwrap()
            .0
    }
    async fn handshake(
        &self,
        private: &[u8; 32],
        public: &[u8; 32],
    ) -> (Socket, snow::TransportState) {
        let mut socket = self.socket("/v2", "").await;
        let mut noise = sealed_network::initiator(private, public);
        let mut buffer = [0u8; 4096];
        let size = noise.write_message(&[], &mut buffer).unwrap();
        socket
            .send(Message::Binary(buffer[..size].to_vec().into()))
            .await
            .unwrap();
        let Some(Ok(Message::Binary(answer))) = socket.next().await else {
            panic!("Noise answer");
        };
        assert_eq!(noise.read_message(&answer, &mut buffer).unwrap(), 0);
        (socket, noise.into_transport_mode().unwrap())
    }
    async fn sealed(
        &self,
        private: &[u8; 32],
        public: &[u8; 32],
    ) -> (Socket, snow::TransportState) {
        let (mut socket, mut state) = self.handshake(private, public).await;
        let hello = sealed_network::read_sealed(&mut socket, &mut state).await;
        assert_eq!(hello["type"], "hello");
        (socket, state)
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn counts(h: &Harness, expected: (usize, usize)) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while h.remote.admission.counts() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn http_get(socket: &mut TlsStream<TcpStream>) -> std::io::Result<String> {
    // Forwarding headers are arbitrary client input and cannot purchase a
    // separate budget, even on a connection arriving from a loopback proxy.
    socket.write_all(b"GET / HTTP/1.1\r\nHost: desk.example\r\nX-Forwarded-For: 203.0.113.9\r\nCF-Connecting-IP: 203.0.113.10\r\n\r\n").await?;
    let mut response = Vec::new();
    loop {
        let byte = socket.read_u8().await?;
        response.push(byte);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let response = String::from_utf8(response).unwrap();
    let length: usize = response
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .unwrap()
        .parse()
        .unwrap();
    socket.read_exact(&mut vec![0; length]).await?;
    Ok(response)
}

#[tokio::test]
async fn paired_devices_reconnect_through_a_proxy_while_anonymous_sockets_rotate() {
    let h = Harness::new().await;
    let proxy = Proxy::new(&h).await;
    let (private, _) = sealed::keypair().unwrap();
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (public, _) = sealed_network::invitation(&pairing);
    sealed_network::claim(&h, &pairing, &private).await.unwrap();
    let (mut established, mut state) = proxy.sealed(&private, &public).await;
    counts(&h, (0, 1)).await;

    // Refill the shared proxy IP repeatedly: idle HTTPS and stalled Noise
    // upgrades exercise both owners of a pending permit. Each reconnect must
    // succeed promptly, rather than waiting for their five-second expiry.
    for _ in 0..3 {
        let mut anonymous = Vec::new();
        for _ in 0..2 {
            let mut socket = proxy.tls().await;
            assert!(
                http_get(&mut socket)
                    .await
                    .unwrap()
                    .starts_with("HTTP/1.1 404")
            );
            anonymous.push(socket);
        }
        let mut stalled = Vec::new();
        for _ in 0..2 {
            stalled.push(proxy.socket("/v2", "").await);
        }
        counts(&h, (4, 1)).await;
        let (mut reconnected, _) =
            tokio::time::timeout(Duration::from_secs(3), proxy.sealed(&private, &public))
                .await
                .expect("anonymous sockets must yield reconnect capacity");
        counts(&h, (3, 2)).await;
        assert!(
            http_get(&mut anonymous[0]).await.is_err(),
            "oldest anonymous connection closed"
        );
        reconnected.close(None).await.unwrap();
        drop(reconnected);
        drop(anonymous);
        drop(stalled);
        counts(&h, (0, 1)).await;
        sealed_network::send_sealed(
            &mut established,
            &mut state,
            json!({"id":1,"cmd":"backends.list"}),
        )
        .await;
        assert_eq!(
            sealed_network::read_sealed(&mut established, &mut state).await["ok"],
            true
        );
    }
    h.remote.revoke(&h.remote.status().devices[0].id).unwrap();
    sealed_network::closed(&mut established).await;
    counts(&h, (0, 0)).await;
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn http_keepalive_cannot_extend_the_absolute_pre_authentication_lifetime() {
    let h = Harness::new().await;
    let proxy = Proxy::new(&h).await;
    let mut socket = proxy.tls().await;
    let started = Instant::now();
    let mut requests = 0;
    tokio::time::timeout(Duration::from_secs(7), async {
        while let Ok(response) = http_get(&mut socket).await {
            assert!(response.starts_with("HTTP/1.1 404"));
            requests += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("keepalive cannot renew anonymous capacity");
    assert!(requests >= 2);
    assert!(started.elapsed() < Duration::from_secs(7));
    counts(&h, (0, 0)).await;
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn raw_tcp_and_stalled_noise_are_evicted_without_leaking_upgrade_permits() {
    let h = Harness::new().await;
    let address = h.endpoint().trim_start_matches("https://").to_owned();
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(TcpStream::connect(&address).await.unwrap());
    }
    counts(&h, (4, 0)).await;
    let mut upgraded = Vec::new();
    for _ in 0..4 {
        upgraded.push(h.socket_at("/v2", "").await.unwrap());
    }
    for socket in &mut held {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), socket.read_u8())
                .await
                .unwrap(),
            Err(_)
        ));
    }
    counts(&h, (4, 0)).await;
    let extra = h.socket_at("/v2", "").await.unwrap();
    sealed_network::closed(&mut upgraded[0]).await;
    counts(&h, (4, 0)).await;
    tokio::time::timeout(Duration::from_secs(7), async {
        while h.remote.admission.counts() != (0, 0) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Noise shares the absolute deadline");
    drop(extra);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn authenticated_budgets_follow_devices_behind_one_proxy_address() {
    let h = Harness::new().await;
    let proxy = Proxy::new(&h).await;
    let mut devices = Vec::new();
    for _ in 0..5 {
        let (private, _) = sealed::keypair().unwrap();
        let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
        let (public, _) = sealed_network::invitation(&pairing);
        sealed_network::claim(&h, &pairing, &private).await.unwrap();
        devices.push((private, public));
    }
    let mut sockets = Vec::new();
    // More than four authenticated sessions behind this one TCP IP work.
    for (private, public) in &devices {
        sockets.push(proxy.sealed(private, public).await.0);
    }
    counts(&h, (0, 5)).await;
    let (private, public) = &devices[0];
    for _ in 0..3 {
        sockets.push(proxy.sealed(private, public).await.0);
    }
    let (mut denied, _) = proxy.handshake(private, public).await;
    sealed_network::closed(&mut denied).await;
    counts(&h, (0, 8)).await;
    // The separate global bound also refuses a device below its own cap.
    for (index, (private, public)) in devices.iter().enumerate().take(4).skip(1) {
        for _ in 0..if index == 3 { 2 } else { 3 } {
            sockets.push(proxy.sealed(private, public).await.0);
        }
    }
    counts(&h, (0, 16)).await;
    let (private, public) = &devices[4];
    let (mut denied, _) = proxy.handshake(private, public).await;
    sealed_network::closed(&mut denied).await;
    counts(&h, (0, 16)).await;
    assert_eq!(
        h.remote.devices().len(),
        5,
        "capacity refusal never revokes a pairing"
    );
    sockets.pop();
    counts(&h, (0, 15)).await;
    sockets.push(proxy.sealed(private, public).await.0);
    counts(&h, (0, 16)).await;
    drop(sockets);
    counts(&h, (0, 0)).await;
    h.remote.configure(false, network::ALL).await.unwrap();
}
