//! A scripted voice client uses the same door as the desktop.
mod common;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use hotline_core::wire::Door;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send(socket: &mut Socket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

async fn until(socket: &mut Socket, matches: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if matches(&frame) {
                return frame;
            }
        }
    })
    .await
    .expect("the voice wire did not answer")
}

#[tokio::test]
async fn a_voice_call_carries_heard_said_and_a_playable_clip() {
    let root = tempfile::tempdir().unwrap();
    hotline_core::log::Log::open(root.path())
        .append(
            &hotline_core::log::StreamId::Room,
            &json!({"kind":"setting","id":"voice","value":{"stub":true}}),
        )
        .unwrap();
    let desk = Arc::new(common::open_desk(root.path()).unwrap());
    let door = Door::bind(desk.log.clone(), "voice-test".into(), desk).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let (mut socket, _) = connect_async(format!("ws://127.0.0.1:{port}/ws?token=voice-test"))
        .await
        .unwrap();
    let call = Uuid::new_v4().to_string();
    send(
        &mut socket,
        json!({"id":1,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["id"] == 1).await["result"]["callId"],
        call
    );
    send(&mut socket, json!({"id":2,"sub":{"call":call}})).await;
    assert_eq!(
        until(&mut socket, |f| f["sub"] == 2 && f["snapshot"].is_array()).await["snapshot"][0]["state"],
        "listening"
    );
    send(&mut socket, json!({"id":3,"cmd":"voice.utterance","params":{"callId":call,"seq":0,"mimeType":"audio/wav","data":STANDARD.encode(b"RIFFstub"),"durationMs":1000}})).await;
    let heard = until(&mut socket, |f| f["event"]["type"] == "heard").await;
    assert_eq!(heard["event"]["seq"], 0);
    let said = until(&mut socket, |f| f["event"]["type"] == "said").await;
    let clip = until(&mut socket, |f| f["event"]["type"] == "clip").await;
    assert_eq!(clip["event"]["id"], said["event"]["id"]);
    assert_eq!(clip["event"]["final"], true);
    let wav = STANDARD
        .decode(clip["event"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(
        u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize + 8,
        wav.len()
    );
    send(
        &mut socket,
        json!({"id":4,"cmd":"voice.call_start","params":{"callId":call}}),
    )
    .await;
    assert_eq!(until(&mut socket, |f| f["id"] == 4).await["ok"], true);
    send(
        &mut socket,
        json!({"id":5,"cmd":"voice.call_start","params":{"callId":Uuid::new_v4().to_string()}}),
    )
    .await;
    assert_eq!(
        until(&mut socket, |f| f["event"]["state"] == "ended").await["event"]["reason"],
        "replaced"
    );
    socket.close(None).await.unwrap();
    server.abort();
}
