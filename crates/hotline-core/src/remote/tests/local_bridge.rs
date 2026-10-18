use super::*;

#[tokio::test]
async fn bridge_authenticates_locally_and_forwards_files_under_the_remote_seat() {
    use crate::remote::bridge::Bridge;
    use base64::engine::general_purpose::STANDARD;
    let h = Harness::new().await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("uploaded.txt");
    for role in [DeviceRole::Companion, DeviceRole::Owner] {
        let client = client::Client::new(Arc::new(MemoryStore::default()));
        let payload = h.remote.pairing_v2(role).unwrap().payload;
        let desk = client.pair(&payload, "Bridge test").await.unwrap();
        let bridge = Bridge::with_client(&desk, client).await.unwrap();
        let origin = bridge.origin.replace("http:", "ws:");
        assert!(
            tokio_tungstenite::connect_async(format!("{origin}/ws?token=wrong"))
                .await
                .is_err()
        );
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("{origin}/ws?token={}", bridge.token))
                .await
                .unwrap();
        let hello = socket.next().await.unwrap().unwrap();
        assert!(hello.to_text().unwrap().contains("hello"));
        async fn ask(
            socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
            cmd: &str,
            params: Value,
        ) -> Value {
            socket
                .send(Message::text(
                    json!({"id":1,"cmd":cmd,"params":params}).to_string(),
                ))
                .await
                .unwrap();
            let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            serde_json::from_str(frame.to_text().unwrap()).unwrap()
        }
        let upload = ask(&mut socket, "files.upload_start", json!({"path":path})).await;
        if role == DeviceRole::Companion {
            assert_eq!(upload["code"], "forbidden");
            assert!(!path.exists());
            assert_eq!(
                ask(
                    &mut socket,
                    "files.upload_start",
                    json!({"name":"refused.txt"})
                )
                .await["code"],
                "forbidden"
            );
            assert!(!h.root.path().join("uploads").exists());
            assert_eq!(
                ask(&mut socket, "files.browse", json!({"path":root.path()})).await["code"],
                "forbidden"
            );
            assert_eq!(
                ask(
                    &mut socket,
                    "files.download",
                    json!({"path":path,"offset":0})
                )
                .await["code"],
                "forbidden"
            );
            continue;
        }
        assert_eq!(upload["ok"], true, "{upload}");
        assert_eq!(upload["result"]["path"], json!(path));
        for params in [
            json!({}),
            json!({"name":"a","path":path}),
            json!({"name":null}),
        ] {
            assert_eq!(
                ask(&mut socket, "files.upload_start", params).await["ok"],
                false
            );
        }
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "/absolute",
            "sub/file",
            "sub\\file",
            "C:drive",
            "bad\0name",
        ] {
            assert_eq!(
                ask(&mut socket, "files.upload_start", json!({"name":name})).await["ok"],
                false,
                "accepted {name:?}"
            );
        }
        let named = ask(
            &mut socket,
            "files.upload_start",
            json!({"name":"notes.txt"}),
        )
        .await;
        assert_eq!(named["ok"], true, "{named}");
        assert_eq!(named["result"]["offset"], 0);
        let staged_path = std::path::PathBuf::from(named["result"]["path"].as_str().unwrap());
        assert_eq!(staged_path.file_name().unwrap(), "notes.txt");
        // The desk names paths resolved, and macOS's temporary folder is
        // behind a symlink (/var is /private/var).
        assert_eq!(
            staged_path.parent().unwrap().parent().unwrap(),
            dunce::canonicalize(h.root.path()).unwrap().join("uploads")
        );
        assert!(!staged_path.exists());
        let named_id = &named["result"]["uploadId"];
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_chunk",
                json!({"uploadId":named_id,"offset":0,"data":STANDARD.encode(b"staged attachment")})
            )
            .await["ok"],
            true
        );
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_finish",
                json!({"uploadId":named_id})
            )
            .await["result"]["path"],
            json!(staged_path)
        );
        assert_eq!(std::fs::read(&staged_path).unwrap(), b"staged attachment");
        let cancelled = ask(
            &mut socket,
            "files.upload_start",
            json!({"name":"notes.txt"}),
        )
        .await;
        let cancelled_path =
            std::path::PathBuf::from(cancelled["result"]["path"].as_str().unwrap());
        assert_ne!(cancelled_path, staged_path);
        assert!(cancelled_path.parent().unwrap().exists());
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_cancel",
                json!({"uploadId":cancelled["result"]["uploadId"]})
            )
            .await["ok"],
            true
        );
        assert!(!cancelled_path.parent().unwrap().exists());
        let unfinished_named = ask(
            &mut socket,
            "files.upload_start",
            json!({"name":"unfinished.txt"}),
        )
        .await;
        let unfinished_named_path =
            std::path::PathBuf::from(unfinished_named["result"]["path"].as_str().unwrap());

        let id = &upload["result"]["uploadId"];
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_chunk",
                json!({"uploadId":id,"offset":0,"data":STANDARD.encode(b"hello laptop")})
            )
            .await["ok"],
            true
        );
        assert!(!path.exists(), "destination stays absent until finish");
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_chunk",
                json!({"uploadId":id,"offset":0,"data":"YQ=="})
            )
            .await["ok"],
            false
        );
        let (mut other, _) =
            tokio_tungstenite::connect_async(format!("{origin}/ws?token={}", bridge.token))
                .await
                .unwrap();
        other.next().await.unwrap().unwrap();
        assert_eq!(
            ask(&mut other, "files.upload_finish", json!({"uploadId":id})).await["ok"],
            false
        );
        assert_eq!(
            ask(&mut socket, "files.upload_finish", json!({"uploadId":id})).await["ok"],
            true
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"hello laptop");
        let downloaded = ask(
            &mut socket,
            "files.download",
            json!({"path":path,"offset":0}),
        )
        .await;
        assert_eq!(
            STANDARD
                .decode(downloaded["result"]["data"].as_str().unwrap())
                .unwrap(),
            b"hello laptop"
        );
        assert_eq!(
            ask(&mut socket, "files.browse", json!({"path":root.path()})).await["result"]["entries"]
                [0]["name"],
            "uploaded.txt"
        );
        assert_eq!(
            ask(&mut socket, "files.upload_start", json!({"path":path})).await["ok"],
            false
        );
        let unfinished = root.path().join("unfinished");
        assert_eq!(
            ask(
                &mut socket,
                "files.upload_start",
                json!({"path":unfinished})
            )
            .await["ok"],
            true
        );
        drop(bridge);
        tokio::time::timeout(Duration::from_secs(10), async {
            while std::fs::read_dir(root.path()).unwrap().count() != 1
                || unfinished_named_path.parent().unwrap().exists()
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(!unfinished.exists());
    }
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn bridge_viewer_preserves_binary_frames_and_takeover_without_exporting_the_bearer() {
    let (port, asked) = fake_computer().await;
    let (h, room) = Harness::with_computer().await;
    *room.viewer.lock().unwrap() = Some(format!("http://127.0.0.1:{port}/#secret-bearer"));
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let payload = h.remote.pairing_v2(DeviceRole::Owner).unwrap().payload;
    let desk = client.pair(&payload, "Viewer laptop").await.unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    let url = format!("{}/computer/ada/ws", bridge.origin.replace("http:", "ws:"));
    assert!(!url.contains("secret-bearer"));
    assert!(!url.contains(&bridge.token));
    let protocol = format!("hotline-viewer.{}", bridge.viewer_token("ada"));
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        http::header::SEC_WEBSOCKET_PROTOCOL,
        protocol.parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        response.headers()[http::header::SEC_WEBSOCKET_PROTOCOL],
        protocol
    );
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
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
    assert_eq!(
        asked.lock().unwrap().as_deref(),
        Some("token=secret-bearer")
    );
    h.remote.revoke(&h.remote.devices()[0].id).unwrap();
    assert!(!matches!(
        tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap(),
        Some(Ok(Message::Text(_) | Message::Binary(_)))
    ));
    h.remote.configure(false, network::ALL).await.unwrap();
}
