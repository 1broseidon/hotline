use super::super::queue::{self, Frame};
use super::*;
use tokio::io::DuplexStream;
use tokio_tungstenite::tungstenite::protocol::Role;

type PhoneSocket = WebSocketStream<DuplexStream>;

async fn phone_socket(
    log: Log,
    quiet: Arc<Quiet>,
    seat: Seat,
) -> (PhoneSocket, tokio::task::JoinHandle<Result<(), Error>>) {
    // A tiny transport buffer makes a stopped reader block the real writer,
    // without depending on a kernel's TCP buffer size or cellular timings.
    let (client, server) = tokio::io::duplex(1024);
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let serving = tokio::spawn(super::super::socket::seated(
        server,
        seat,
        crate::wire::Agents {
            log,
            room: quiet,
            parts: Parts::default(),
        },
    ));
    (client, serving)
}

async fn write(socket: &mut PhoneSocket, frame: Value) {
    socket.send(Message::text(frame.to_string())).await.unwrap();
}

async fn next(socket: &mut PhoneSocket) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(30), socket.next())
        .await
        .expect("the phone receives a frame")
        .expect("the phone stays connected")
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn until(what: &str, condition: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never: {what}"));
}

#[tokio::test]
async fn dropping_a_seat_drops_its_blocked_writer_before_returning() {
    use tokio::io::AsyncReadExt;

    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let (mut phone, serving) = phone_socket(log.clone(), Arc::new(Quiet::new()), Seat::Owner).await;
    write(&mut phone, json!({"id":1,"sub":"room"})).await;
    assert_eq!(next(&mut phone).await["ok"], true);
    assert!(next(&mut phone).await["snapshot"].is_array());
    log.append(
        &StreamId::Room,
        &json!({"kind":"setting","id":"large","value":"x".repeat(65536)}),
    )
    .unwrap();
    let transport = phone.get_mut();
    // One byte proves the writer started a frame larger than the 1 KiB pipe.
    tokio::time::timeout(Duration::from_secs(30), transport.read_u8())
        .await
        .expect("the writer starts the large frame")
        .unwrap();
    serving.abort();
    assert!(serving.await.unwrap_err().is_cancelled());
    let mut remaining = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(30),
        transport.read_to_end(&mut remaining),
    )
    .await
    .expect("the cancelled seat owns no live transport")
    .unwrap();
    assert!(
        remaining.len() <= 1024,
        "a detached writer kept sending after the seat ended: {} bytes",
        remaining.len()
    );
}

#[tokio::test]
async fn a_burst_of_1000_tool_updates_keeps_a_slow_phone_connected() {
    for seat in [Seat::Phone, Seat::Owner] {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        log.append(
            &StreamId::Room,
            &json!({
                "kind":"persona", "id":"ada", "name":"Ada", "goal":"Help",
                "backendId":"hotline", "cwd":root.path(),
                "mcpPolicy":{"mode":"none","serverIds":[]},
                "sessionCheckpoints":[], "lastSessionId":null, "createdAt":1, "updatedAt":1
            }),
        )
        .unwrap();
        let quiet = Arc::new(Quiet::new());
        quiet.set_info(thinking("ada"));
        let (mut phone, serving) = phone_socket(log.clone(), quiet.clone(), seat).await;
        write(
            &mut phone,
            json!({"id":1,"cmd":"client.hello","params":{"capabilities":["lean","rosterOptions"]}}),
        )
        .await;
        assert_eq!(next(&mut phone).await["ok"], true);
        for (id, target) in [(2, json!({"tape":"ada"})), (3, json!({"view":"roster"}))] {
            write(&mut phone, json!({"id":id,"sub":target})).await;
            assert_eq!(next(&mut phone).await["ok"], true);
            assert!(next(&mut phone).await["snapshot"].is_array());
        }

        let tape = StreamId::Tape("ada".into());
        for n in 0..1000 {
            let reads = quiet.info_reads.lock().unwrap().len();
            log.append(
                &tape,
                &json!({
                    "kind":"tool", "id":format!("tool-{n}"), "toolCallId":format!("call-{n}"),
                    "ts":n, "title":format!("Read file {n}"), "status":"in_progress"
                }),
            )
            .unwrap();
            // Wait for each roster projection without reading the phone.
            // This exercises both sources of pressure, not just a queue API.
            until("the tool reaches the roster", || {
                quiet.info_reads.lock().unwrap().len() > reads || serving.is_finished()
            })
            .await;
            assert!(!serving.is_finished(), "tool {n} dropped {seat:?}");
        }
        log.append(
            &tape,
            &json!({"kind":"agent","id":"answer","ts":1001,"text":"Done"}),
        )
        .unwrap();
        log.append(&tape, &json!({"kind":"permission","id":"card","ts":1002,"requestId":"ask","title":"Review","options":[]})).unwrap();
        write(&mut phone, json!({"id":4,"cmd":"ping"})).await;

        let (mut answer, mut card, mut pong, mut tool_frames) = (false, false, false, 0);
        while !answer || !card || !pong {
            let frame = next(&mut phone).await;
            assert!(
                frame.get("snapshot").is_none(),
                "no reconnect or lag snapshot: {frame}"
            );
            answer |= frame["event"]["id"] == "answer";
            card |= frame["event"]["id"] == "card";
            pong |= frame["id"] == 4 && frame["ok"] == true;
            tool_frames += usize::from(frame["event"]["kind"] == "tool");
        }
        assert!(tool_frames < 1000, "the slow link must shed tool frames");
        assert!(!serving.is_finished());
        serving.abort();
    }
}

#[tokio::test]
async fn a_lagging_phone_is_resnapshotted_with_its_full_phone_window() {
    for seat in [Seat::Phone, Seat::Owner, Seat::Desk] {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        let tape = StreamId::Tape("ada".into());
        for n in 0..300 {
            log.append(
                &tape,
                &json!({"kind":"user","id":format!("m{n}"),"ts":n,"text":"line"}),
            )
            .unwrap();
        }
        let (mut phone, serving) = phone_socket(log.clone(), Arc::new(Quiet::new()), seat).await;
        if seat == Seat::Owner {
            write(
                &mut phone,
                json!({"id":1,"cmd":"client.hello","params":{"capabilities":["lean"]}}),
            )
            .await;
            assert_eq!(next(&mut phone).await["ok"], true);
        }
        write(&mut phone, json!({"id":2,"sub":{"tape":"ada"},"window":40})).await;
        assert_eq!(next(&mut phone).await["ok"], true);
        let seed = next(&mut phone).await;
        assert_eq!(
            seed["snapshot"].as_array().unwrap().len(),
            if seat == Seat::Desk { 300 } else { 40 }
        );

        // No await: on this current-thread runtime the subscriber cannot run
        // before the log's 1024-entry broadcast has definitely overflowed.
        for n in 300..1500 {
            log.append(
                &tape,
                &json!({"kind":"user","id":format!("m{n}"),"ts":n,"text":"line"}),
            )
            .unwrap();
        }
        let recovered = next(&mut phone).await;
        let lines = recovered["snapshot"].as_array().unwrap();
        let window = if seat == Seat::Desk {
            400
        } else {
            super::super::phone::PHONE_WINDOW
        };
        assert_eq!(lines.len(), window);
        assert_eq!(lines[0]["id"], format!("m{}", 1500 - window));
        assert_eq!(lines.last().unwrap()["id"], "m1499");
        log.append(
            &tape,
            &json!({"kind":"agent","id":"after","ts":1501,"text":"still live"}),
        )
        .unwrap();
        assert_eq!(
            next(&mut phone).await["event"]["id"],
            "after",
            "recovery must not replay stale broadcast entries"
        );
        serving.abort();
    }
}

#[tokio::test]
async fn the_phone_queue_preserves_required_frames_and_its_bound() {
    let (tx, mut rx) = queue::channel();
    tx.send(Frame::Required("snapshot".into()), usize::MAX)
        .unwrap();
    for n in 0..1000 {
        tx.send(Frame::Droppable(format!("tool {n}")), usize::MAX)
            .unwrap();
    }
    tx.send(Frame::Required("reply".into()), usize::MAX)
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(frame) = rx.recv().await {
        frames.push(frame);
    }
    assert_eq!(frames.len(), queue::CAPACITY);
    assert_eq!(frames.first().unwrap(), "snapshot");
    assert_eq!(frames.last().unwrap(), "reply");
    assert!(frames.contains(&"tool 999".into()));

    let (tx, receiver) = queue::channel();
    for _ in 0..queue::CAPACITY {
        tx.send(Frame::Required("message".into()), usize::MAX)
            .unwrap();
    }
    assert!(tx.send(Frame::Droppable("tool".into()), usize::MAX).is_ok());
    assert!(
        tx.send(Frame::Required("message".into()), usize::MAX)
            .is_err()
    );
    assert!(tx.send(Frame::Droppable("too large".into()), 1).is_err());
    drop(receiver);
    assert!(
        tx.send(Frame::Droppable("tool".into()), usize::MAX)
            .is_err()
    );
}

#[tokio::test]
async fn coalesced_roster_rows_keep_catalogs_without_crossing_a_snapshot() {
    let (tx, mut rx) = queue::channel();
    let row = |sub, id, session| Frame::Roster {
        sub,
        row: json!({"persona":{"id":id},"session":session}),
    };
    tx.send(
        row(
            1,
            "ada",
            json!({"models":["a"],"slashCommands":["b"],"state":"idle"}),
        ),
        usize::MAX,
    )
    .unwrap();
    tx.send(
        row(2, "ada", json!({"models":[],"slashCommands":[]})),
        usize::MAX,
    )
    .unwrap();
    tx.send(
        row(1, "bob", json!({"models":[],"slashCommands":[]})),
        usize::MAX,
    )
    .unwrap();
    tx.send(row(1, "ada", json!({"state":"thinking"})), usize::MAX)
        .unwrap();
    tx.send(
        row(1, "ada", json!({"models":[],"state":"idle"})),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&rx.recv().await.unwrap()).unwrap()["sub"],
        2
    );
    assert_eq!(
        serde_json::from_str::<Value>(&rx.recv().await.unwrap()).unwrap()["event"]["persona"]["id"],
        "bob"
    );
    let merged: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(
        merged["event"]["session"],
        json!({"models":[],"slashCommands":["b"],"state":"idle"})
    );

    tx.send(row(1, "ada", json!({"models":["old"]})), usize::MAX)
        .unwrap();
    tx.send(
        Frame::Required("snapshot with new catalogs".into()),
        usize::MAX,
    )
    .unwrap();
    tx.send(row(1, "ada", json!({"state":"thinking"})), usize::MAX)
        .unwrap();
    assert!(rx.recv().await.unwrap().contains("old"));
    assert_eq!(rx.recv().await.unwrap(), "snapshot with new catalogs");
    let after: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert!(after["event"]["session"].get("models").is_none());
}
