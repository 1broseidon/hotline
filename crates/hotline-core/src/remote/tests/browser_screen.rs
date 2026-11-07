//! A teammate's desk browser as a live screen, through the desktop's bridge
//! and the desk's sealed door (task-390): which tokens open it, the
//! computer viewer's protocol on it, and its `control` as the browser's
//! Take control and Give it back.
use super::*;
use crate::browser::screen::{Frame, RELEASE, Rect};
use crate::contract::BrowserPhase;
use http::header::SEC_WEBSOCKET_PROTOCOL;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Error, handshake::client::Request};

fn request(bridge: &bridge::Bridge, path: &str, token: Option<&str>) -> Request {
    let mut request = format!("{}{path}", bridge.origin.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            format!("hotline-viewer.{token}").parse().unwrap(),
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

type Viewer = WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// A desk with Ada's browser on a page, the teammate driving it, and a
/// bridge to it from a device paired as `role`.
async fn browsing(role: DeviceRole) -> (Harness, Arc<Computered>, bridge::Bridge) {
    let (h, room) = Harness::with_computer().await;
    *room.browser.phase.lock().unwrap() = Some(BrowserPhase::Teammate);
    let client = client::Client::new(Arc::new(MemoryStore::default()));
    let desk = client
        .pair(
            &h.remote.pairing_v2(role).unwrap().payload,
            "Browser screen",
        )
        .await
        .unwrap();
    let bridge = bridge::Bridge::with_client(&desk, client).await.unwrap();
    (h, room, bridge)
}

async fn open(bridge: &bridge::Bridge) -> Viewer {
    let token = bridge.viewer_token("ada", bridge::Viewer::Browser);
    let (socket, response) =
        tokio_tungstenite::connect_async(request(bridge, "/browser/ada/ws", Some(&token)))
            .await
            .unwrap();
    assert_eq!(
        response.headers()[SEC_WEBSOCKET_PROTOCOL],
        format!("hotline-viewer.{token}")
    );
    socket
}

async fn next(socket: &mut Viewer) -> Message {
    tokio::time::timeout(ANSWER, socket.next())
        .await
        .expect("the screen said nothing")
        .unwrap()
        .unwrap()
}

/// The next JSON message of kind `t`, past frames and other messages.
async fn said(socket: &mut Viewer, t: &str) -> Value {
    loop {
        if let Message::Text(text) = next(socket).await {
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["t"] == t {
                return value;
            }
        }
    }
}

async fn send(socket: &mut Viewer, message: Value) {
    socket
        .send(Message::text(message.to_string()))
        .await
        .unwrap();
}

/// The next state that names `holder`: the door says who has the browser
/// once a second and at each change, so an older one may come first.
async fn holder(socket: &mut Viewer, holder: &str) -> Value {
    loop {
        let state = said(socket, "state").await;
        if state["holder"] == holder {
            return state;
        }
    }
}

/// Everything sent before a ping has been read by the door once its pong
/// is back.
async fn settled(socket: &mut Viewer, id: u64) {
    send(socket, json!({"t": "ping", "id": id})).await;
    assert_eq!(said(socket, "pong").await["id"], id);
}

#[tokio::test]
async fn a_browser_token_opens_only_that_teammates_browser_and_it_speaks_the_computers_protocol() {
    let (h, _room, bridge) = browsing(DeviceRole::Owner).await;
    let computer = bridge.viewer_token("ada", bridge::Viewer::Computer);
    refused(request(&bridge, "/browser/ada/ws", Some(&computer))).await;
    let browser = bridge.viewer_token("ada", bridge::Viewer::Browser);
    refused(request(&bridge, "/computer/ada/ws", Some(&browser))).await;
    refused(request(&bridge, "/browser/bob/ws", Some(&browser))).await;
    // The owner's own credential opens the wire, never a screen.
    refused(request(&bridge, "/browser/ada/ws", None)).await;
    refused(request(
        &bridge,
        &format!("/browser/ada/ws?token={}", bridge.token),
        None,
    ))
    .await;
    refused(request(&bridge, "/browser/ada/ws", Some(&bridge.token))).await;

    let (mut socket, _) =
        tokio_tungstenite::connect_async(request(&bridge, "/browser/ada/ws", Some(&browser)))
            .await
            .unwrap();
    // A frame first, its 12-byte header and the PNG, as a binary message.
    let Message::Binary(frame) = next(&mut socket).await else {
        panic!("the first message is a frame");
    };
    assert_eq!(Frame::header(&frame), Some((Rect::new(0, 0, 2, 2), (2, 2))));
    assert_eq!(&frame[12..], b"\x89PNG");
    assert_eq!(said(&mut socket, "pointer").await["x"], 1);
    let state = holder(&mut socket, "agent").await;
    assert_eq!(state["driving"], false);
    assert_eq!(state["browser"]["url"], "https://example.com/");
    // Spent: a reconnect asks for a fresh one.
    refused(request(&bridge, "/browser/ada/ws", Some(&browser))).await;
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn control_is_the_browsers_take_control_and_only_the_seat_driving_it_reaches_its_hands() {
    let (h, room, bridge) = browsing(DeviceRole::Owner).await;
    let mut socket = open(&bridge).await;
    let moved = json!({"t": "move", "x": 10, "y": 20});

    // Watching: input goes nowhere.
    send(&mut socket, moved.clone()).await;
    settled(&mut socket, 1).await;
    assert!(room.browser.hands.lock().unwrap().is_empty());

    send(&mut socket, json!({"t": "control", "take": true})).await;
    let state = holder(&mut socket, "you").await;
    assert_eq!(state["driving"], true);
    assert_eq!(state["browser"]["phase"], "person");
    assert_eq!(*room.browser.takes.lock().unwrap(), [true]);

    send(&mut socket, moved.clone()).await;
    send(&mut socket, json!({"t": "button", "b": 1, "down": true})).await;
    send(&mut socket, json!({"t": "wheel", "dy": 1})).await;
    send(&mut socket, json!({"t": "key", "key": "é", "down": true})).await;
    settled(&mut socket, 2).await;
    let reached: Vec<Value> = room
        .browser
        .hands
        .lock()
        .unwrap()
        .iter()
        .map(|said| serde_json::from_str(said).unwrap())
        .collect();
    assert_eq!(
        reached,
        [
            moved.clone(),
            json!({"t": "button", "b": 1, "down": true}),
            json!({"t": "wheel", "dy": 1}),
            json!({"t": "key", "key": "é", "down": true}),
        ]
    );

    // Something outside the protocol is answered as an error and goes nowhere.
    send(&mut socket, json!({"t": "files", "id": 1})).await;
    assert_eq!(said(&mut socket, "error").await["driving"], true);

    send(&mut socket, json!({"t": "control", "take": false})).await;
    let state = holder(&mut socket, "agent").await;
    assert_eq!(state["driving"], false);
    assert_eq!(*room.browser.takes.lock().unwrap(), [true, false]);
    settled(&mut socket, 3).await;
    assert_eq!(
        room.browser
            .hands
            .lock()
            .unwrap()
            .last()
            .map(String::as_str),
        Some(RELEASE),
        "giving it back lets go of every key and button"
    );
    let before = room.browser.hands.lock().unwrap().len();
    send(&mut socket, moved).await;
    settled(&mut socket, 4).await;
    assert_eq!(room.browser.hands.lock().unwrap().len(), before);
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn handed_back_from_elsewhere_the_seat_watches_again() {
    let (h, room, bridge) = browsing(DeviceRole::Owner).await;
    let mut socket = open(&bridge).await;
    send(&mut socket, json!({"t": "control", "take": true})).await;
    holder(&mut socket, "you").await;
    // The phone's Hand back, or the pane's.
    *room.browser.phase.lock().unwrap() = Some(BrowserPhase::Teammate);
    assert_eq!(holder(&mut socket, "agent").await["driving"], false);
    let before = room.browser.hands.lock().unwrap().len();
    send(&mut socket, json!({"t": "move", "x": 1, "y": 1})).await;
    settled(&mut socket, 1).await;
    assert_eq!(room.browser.hands.lock().unwrap().len(), before);
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_companion_watches_the_browser_and_never_drives_it() {
    let (h, room, bridge) = browsing(DeviceRole::Companion).await;
    let mut socket = open(&bridge).await;
    assert!(matches!(next(&mut socket).await, Message::Binary(_)));
    send(&mut socket, json!({"t": "control", "take": true})).await;
    let error = said(&mut socket, "error").await;
    assert!(
        error["error"].as_str().unwrap().contains("watch"),
        "{error}"
    );
    send(&mut socket, json!({"t": "paste", "text": "hi"})).await;
    said(&mut socket, "error").await;
    send(&mut socket, json!({"t": "move", "x": 1, "y": 1})).await;
    settled(&mut socket, 1).await;
    assert!(room.browser.takes.lock().unwrap().is_empty());
    assert!(room.browser.hands.lock().unwrap().is_empty());
    drop(socket);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_seat_that_vanished_while_driving_hands_the_browser_back_unless_another_took_it() {
    let grace = super::super::browser_screen::GRACE;
    let (h, room, bridge) = browsing(DeviceRole::Owner).await;

    // Gone, and nobody came back: the teammate has its browser again.
    let mut socket = open(&bridge).await;
    send(&mut socket, json!({"t": "control", "take": true})).await;
    holder(&mut socket, "you").await;
    drop(socket);
    tokio::time::sleep(grace + Duration::from_millis(500)).await;
    assert_eq!(*room.browser.takes.lock().unwrap(), [true, false]);
    assert_eq!(
        *room.browser.phase.lock().unwrap(),
        Some(BrowserPhase::Teammate)
    );

    // Gone, but the person's viewer came back and took it again.
    let mut first = open(&bridge).await;
    send(&mut first, json!({"t": "control", "take": true})).await;
    holder(&mut first, "you").await;
    let mut again = open(&bridge).await;
    send(&mut again, json!({"t": "control", "take": true})).await;
    holder(&mut again, "you").await;
    drop(first);
    tokio::time::sleep(grace + Duration::from_millis(500)).await;
    assert_eq!(
        *room.browser.takes.lock().unwrap(),
        [true, false, true, true]
    );
    assert_eq!(
        *room.browser.phase.lock().unwrap(),
        Some(BrowserPhase::Person)
    );
    drop(again);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_teammate_with_no_page_open_is_said_so_and_the_screen_closes() {
    let (h, room, bridge) = browsing(DeviceRole::Owner).await;
    *room.browser.phase.lock().unwrap() = None;
    let mut socket = open(&bridge).await;
    let error = said(&mut socket, "error").await;
    assert!(
        error["error"].as_str().unwrap().contains("no page"),
        "{error}"
    );
    assert!(
        tokio::time::timeout(ANSWER, socket.next())
            .await
            .unwrap()
            .is_none_or(|message| message.is_err() || matches!(message, Ok(Message::Close(_)))),
        "the screen closes"
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}
