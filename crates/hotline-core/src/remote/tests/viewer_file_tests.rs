use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{collections::BTreeMap, convert::Infallible, sync::Mutex};

#[derive(Default)]
struct ComputerFiles {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    downloads: std::sync::atomic::AtomicUsize,
    old: std::sync::atomic::AtomicBool,
    race: std::sync::atomic::AtomicBool,
    slow: std::sync::atomic::AtomicBool,
}
struct FileComputer {
    port: u16,
    state: Arc<ComputerFiles>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for FileComputer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl FileComputer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(ComputerFiles::default());
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let shared = shared.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request| file_service(request, shared.clone()));
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades()
                        .await;
                });
            }
        });
        Self { port, state, task }
    }
}
fn body_response(code: u16, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(code)
        .body(Full::new(body.into()))
        .unwrap()
}
async fn file_service(
    mut request: Request<Incoming>,
    state: Arc<ComputerFiles>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use std::sync::atomic::Ordering::SeqCst;
    let query: BTreeMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    if request.uri().path() == "/ws" {
        assert_eq!(
            query.get("token").map(String::as_str),
            Some("secret-bearer")
        );
        let handshake = Request::builder()
            .method(request.method())
            .uri(request.uri());
        let mut handshake = handshake.body(()).unwrap();
        *handshake.headers_mut() = request.headers().clone();
        let response =
            tokio_tungstenite::tungstenite::handshake::server::create_response(&handshake).unwrap();
        let upgraded = hyper::upgrade::on(&mut request);
        tokio::spawn(async move {
            let Ok(stream) = upgraded.await else { return };
            let mut socket = WebSocketStream::from_raw_socket(
                TokioIo::new(stream),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            let _ = socket.send(Message::binary(b"PNG".to_vec())).await;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_ne!(
                    value["type"], "files",
                    "file requests must terminate at the desk"
                );
                if socket.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        return Ok(response.map(|_| Full::new(Bytes::new())));
    }
    assert!(
        !query.contains_key("token"),
        "file credentials must use headers"
    );
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer secret-bearer"
    );
    let path = query.get("path").cloned().unwrap_or_default();
    if !path.starts_with("/home/agent") || path.contains("..") {
        return Ok(body_response(403, "private upstream details secret-bearer"));
    }
    let route = request.uri().path().to_string();
    let result = if route == "/files/download" {
        state.downloads.fetch_add(1, SeqCst);
        match state.files.lock().unwrap().get(&path) {
            Some(bytes) => body_response(200, bytes.clone()),
            None => body_response(404, "missing secret-bearer"),
        }
    } else if request.method() == hyper::Method::POST && route == "/files" {
        assert_eq!(query.get("create_only").map(String::as_str), Some("true"));
        let bytes = request
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        let mut files = state.files.lock().unwrap();
        if state.race.swap(false, SeqCst) {
            files.insert(path.clone(), b"concurrent writer".to_vec());
        }
        match files.entry(path.clone()) {
            std::collections::btree_map::Entry::Occupied(_) => body_response(409, "exists"),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let size = bytes.len();
                entry.insert(bytes);
                body_response(200, json!({"path":path,"bytes":size}).to_string())
            }
        }
    } else if route == "/files" {
        if state.slow.load(SeqCst) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let entries: Vec<Value> = state.files.lock().unwrap().iter()
            .filter(|(name, _)| name.rsplit_once('/').unwrap().0 == path)
            .map(|(name, bytes)| json!({"name":name.rsplit('/').next().unwrap(),"is_dir":false,"size":bytes.len(),"modified":123}))
            .collect();
        let mut response = body_response(
            200,
            json!({"path":path,"home":"/home/agent","entries":entries}).to_string(),
        );
        if !state.old.load(SeqCst) {
            response
                .headers_mut()
                .insert("x-hotline-upload-create-only", "1".parse().unwrap());
        }
        response
    } else {
        body_response(404, "unknown")
    };
    Ok(result)
}
type ViewerSocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;
async fn viewer(bridge: &bridge::Bridge) -> ViewerSocket {
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "{}/computer/ada/ws?token={}",
        bridge.origin.replace("http:", "ws:"),
        bridge.token
    ))
    .await
    .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::binary(b"PNG".to_vec())
    );
    socket
}
async fn files(socket: &mut ViewerSocket, mut request: Value) -> Value {
    request["type"] = json!("files");
    if request.get("id").is_none() {
        request["id"] = json!("viewer-request");
    }
    socket
        .send(Message::text(request.to_string()))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = response else {
        panic!("expected file reply")
    };
    assert!(!text.contains("secret-bearer"));
    let response: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(response["type"], "files");
    assert_eq!(response["id"], request["id"]);
    response
}
async fn wait_for_cleanup(root: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::fs::read_dir(root).is_ok_and(|mut entries| entries.next().is_some()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sealed_viewer_files_are_chunked_private_and_connection_scoped() {
    use base64::engine::general_purpose::STANDARD;
    use std::sync::atomic::Ordering::SeqCst;
    let computer = FileComputer::start().await;
    let data = vec![42; 512 * 1024 + 17];
    computer
        .state
        .files
        .lock()
        .unwrap()
        .insert("/home/agent/source.bin".into(), data.clone());
    let (h, room) = Harness::with_computer().await;
    *room.viewer.lock().unwrap() =
        Some(format!("http://127.0.0.1:{}/#secret-bearer", computer.port));
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = client
        .pair(
            &h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload,
            "File laptop",
        )
        .await
        .unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let mut socket = viewer(&bridge).await;
    let listed = files(&mut socket, json!({"id":7,"op":"list","path":""})).await;
    assert_eq!(
        listed["result"],
        json!({"path":"/home/agent","home":"/home/agent","entries":[{"name":"source.bin","is_dir":false,"size":data.len()}]})
    );
    assert_eq!(
        files(&mut socket, json!({"op":"list","path":"/etc"})).await["ok"],
        false
    );
    let first = files(
        &mut socket,
        json!({"op":"download","path":"/home/agent/source.bin","offset":0}),
    )
    .await;
    assert_eq!(
        STANDARD
            .decode(first["result"]["data"].as_str().unwrap())
            .unwrap(),
        data[..512 * 1024]
    );
    assert_eq!(first["result"]["next"], 512 * 1024);
    let last = files(
        &mut socket,
        json!({"op":"download","path":"/home/agent/source.bin","offset":512*1024}),
    )
    .await;
    assert_eq!(
        STANDARD
            .decode(last["result"]["data"].as_str().unwrap())
            .unwrap(),
        data[512 * 1024..]
    );
    assert!(last["result"]["next"].is_null());
    assert_eq!(computer.state.downloads.load(SeqCst), 1);
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"download","path":"/home/agent/source.bin","offset":data.len()+1})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_start","path":"/home/agent/source.bin"})
        )
        .await["ok"],
        false
    );
    let started = files(
        &mut socket,
        json!({"op":"upload_start","path":"/home/agent/copy.bin"}),
    )
    .await;
    assert_eq!(started["ok"], true, "{started}");
    let id = started["result"]["uploadId"].clone();
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_chunk","uploadId":id,"offset":0,"data":""})
        )
        .await["result"]["offset"],
        0
    );
    assert_eq!(files(&mut socket,json!({"op":"upload_chunk","uploadId":id,"offset":0,"data":STANDARD.encode(vec![0;512*1024+1])})).await["ok"],false);
    let mut other = viewer(&bridge).await;
    assert_eq!(
        files(&mut other, json!({"op":"upload_cancel","uploadId":id})).await["ok"],
        false
    );
    drop(other);
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_chunk","uploadId":id,"offset":1,"data":"eA=="})
        )
        .await["ok"],
        false
    );
    assert_eq!(files(&mut socket,json!({"op":"upload_chunk","uploadId":id,"offset":0,"data":STANDARD.encode(&data[..512*1024])})).await["result"]["offset"], 512*1024);
    assert_eq!(files(&mut socket,json!({"op":"upload_chunk","uploadId":id,"offset":512*1024,"data":STANDARD.encode(&data[512*1024..])})).await["result"]["offset"], data.len());
    let done = files(&mut socket, json!({"op":"upload_finish","uploadId":id})).await;
    assert_eq!(
        done["result"],
        json!({"path":"/home/agent/copy.bin","size":data.len()})
    );
    assert_eq!(
        computer.state.files.lock().unwrap()["/home/agent/copy.bin"],
        data
    );
    // A competing writer after both preflights is protected by atomic create-only.
    let started = files(
        &mut socket,
        json!({"op":"upload_start","path":"/home/agent/race.bin"}),
    )
    .await;
    computer.state.race.store(true, SeqCst);
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_finish","uploadId":started["result"]["uploadId"]})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        computer.state.files.lock().unwrap()["/home/agent/race.bin"],
        b"concurrent writer"
    );
    let staged = h.root.path().join("viewer-uploads");
    wait_for_cleanup(&staged).await;
    let started = files(
        &mut socket,
        json!({"op":"upload_start","path":"/home/agent/cancel.bin"}),
    )
    .await;
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_cancel","uploadId":started["result"]["uploadId"]})
        )
        .await["ok"],
        true
    );
    wait_for_cleanup(&staged).await;
    let started = files(
        &mut socket,
        json!({"op":"upload_start","path":"/home/agent/disconnect.bin"}),
    )
    .await;
    assert_eq!(started["ok"], true);
    assert_eq!(std::fs::read_dir(&staged).unwrap().count(), 1);
    drop(socket);
    wait_for_cleanup(&staged).await;
    assert!(
        !computer
            .state
            .files
            .lock()
            .unwrap()
            .contains_key("/home/agent/disconnect.bin")
    );
    let mut socket = viewer(&bridge).await;
    assert_eq!(
        files(
            &mut socket,
            json!({"op":"upload_start","path":"/home/agent/revoke.bin"})
        )
        .await["ok"],
        true
    );
    h.remote.revoke(&h.remote.devices()[0].id).unwrap();
    wait_for_cleanup(&staged).await;
    assert!(!matches!(
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap(),
        Some(Ok(Message::Text(_) | Message::Binary(_)))
    ));
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn sealed_viewer_files_reject_legacy_overwrite_and_keep_controls_responsive() {
    use std::sync::atomic::Ordering::SeqCst;
    let computer = FileComputer::start().await;
    computer.state.old.store(true, SeqCst);
    let (h, room) = Harness::with_computer().await;
    *room.viewer.lock().unwrap() =
        Some(format!("http://127.0.0.1:{}/#secret-bearer", computer.port));
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = client
        .pair(
            &h.remote.pairing_v2(DeviceRole::Companion).unwrap().payload,
            "Companion viewer",
        )
        .await
        .unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let mut socket = viewer(&bridge).await;
    assert_eq!(
        files(&mut socket, json!({"op":"list","path":""})).await["ok"],
        true
    );
    let rejected = files(
        &mut socket,
        json!({"op":"upload_start","path":"/home/agent/new.bin"}),
    )
    .await;
    assert_eq!(rejected["ok"], false);
    assert!(
        rejected["error"]
            .as_str()
            .unwrap()
            .contains("Update this computer")
    );
    assert!(!h.root.path().join("viewer-uploads").exists());
    assert_eq!(
        files(&mut socket, json!({"op":"unknown"})).await["ok"],
        false
    );
    computer.state.slow.store(true, SeqCst);
    socket
        .send(Message::text(
            json!({"type":"files","id":"slow","op":"list","path":""}).to_string(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::text("{\"type\":\"takeover\"}"))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::text("{\"type\":\"takeover\"}")
    );
    let Message::Text(reply) = socket.next().await.unwrap().unwrap() else {
        panic!("expected reply")
    };
    assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["id"], "slow");
    h.remote.configure(false, network::ALL).await.unwrap();
}
