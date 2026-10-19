use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_teammate_is_made_watched_keyed_chaptered_and_removed_over_the_wire() {
    let (_root, port) = open("dry").await;
    let mut client = Client::connect(port, TOKEN).await;

    let roster = client.subscribe(json!({ "view": "roster" })).await;
    let empty = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "snapshot")
        })
        .await;
    assert_eq!(empty["snapshot"], json!([]));

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Prove the wire." } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["result"]["backendId"], "hotline");

    let row = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "event")
        })
        .await;
    assert_eq!(row["event"]["persona"]["id"], persona_id);
    assert_eq!(row["event"]["session"]["state"], "idle");
    assert_eq!(row["event"]["preview"], Value::Null);

    let tape = client.subscribe(json!({ "tape": persona_id })).await;
    let blank = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "snapshot")
        })
        .await;
    assert_eq!(blank["snapshot"], json!([]));

    let none = client.call("models.list", json!({})).await;
    assert_eq!(none["result"], json!([]));
    let keyed = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "harness", "secret": "sk-ant-harness" }),
        )
        .await;
    assert_eq!(keyed["ok"], true, "{keyed}");
    let some = client.call("models.list", json!({})).await;
    let ids: Vec<&str> = some["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.is_empty() && ids.iter().all(|id| id.starts_with("anthropic/")),
        "{some}"
    );

    // A session opens a chapter, and closing it is one command. Nothing was
    // said in this one, so it closes untitled without a model being asked.
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let marker = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "chapter"
        })
        .await;
    assert_eq!(marker["event"]["backendId"], "hotline");
    assert_eq!(marker["event"].get("endedAt"), None);

    let closed = client
        .call("chapter.start_fresh", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(closed["result"]["id"], marker["event"]["id"]);
    assert_eq!(closed["result"]["messages"], 0);
    assert_eq!(closed["result"].get("title"), None);

    let gone = client
        .call("persona.delete", json!({ "id": persona_id }))
        .await;
    assert_eq!(gone["ok"], true, "{gone}");
    let removed = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, roster, "removed")
        })
        .await;
    assert_eq!(removed["removed"], persona_id);
}

/// The welcome pane's state, walked over the wire from a fresh data
/// directory: nothing to run on, then a key, then a harness made the room's
/// default, then a teammate, then that teammate deleted, which leaves the
/// room empty but still set up. Each step is derived from what the room knows,
/// so there is no flag to reset and nothing to have seen. The first turn
/// itself needs a real key and is proved by the keyed harness below; here the
/// tape is empty after the start, which is what the starter card reads.
#[tokio::test(flavor = "multi_thread")]
async fn the_welcome_state_derives_from_a_fresh_room_to_a_first_teammate() {
    let (_root, port) = open("welcome").await;
    let mut client = Client::connect(port, TOKEN).await;

    let fresh = client.call("welcome", json!({})).await;
    assert_eq!(fresh["ok"], true, "{fresh}");
    assert_eq!(fresh["result"]["canRun"], false);
    assert_eq!(fresh["result"]["providers"], json!([]));
    assert_eq!(fresh["result"]["teammates"], 0);
    assert_eq!(fresh["result"]["setUp"], false);
    assert_eq!(fresh["result"]["defaultBackendId"], "hotline");
    let harnesses = fresh["result"]["harnesses"].as_array().unwrap().clone();
    assert!(
        harnesses
            .iter()
            .all(|one| one["id"] != "hotline" && one.get("unavailable").is_none()),
        "only startable ACP harnesses are offered: {harnesses:?}"
    );

    // A harness this machine can start is a way in once it is the room's
    // default, and only then. Which harnesses there are depends on the
    // machine, so the step is walked with whatever is installed.
    if let Some(harness) = harnesses.first() {
        let chosen = client
            .call(
                "settings.update",
                json!({ "patch": { "defaultBackendId": harness["id"] } }),
            )
            .await;
        assert_eq!(chosen["ok"], true, "{chosen}");
        let on_harness = client.call("welcome", json!({})).await;
        assert_eq!(on_harness["result"]["canRun"], true, "{on_harness}");
        assert_eq!(on_harness["result"]["defaultBackendId"], harness["id"]);
        let back = client
            .call(
                "settings.update",
                json!({ "patch": { "defaultBackendId": "hotline" } }),
            )
            .await;
        assert_eq!(back["ok"], true, "{back}");
        let off_harness = client.call("welcome", json!({})).await;
        assert_eq!(off_harness["result"]["canRun"], false, "{off_harness}");
    }

    let keyed = client
        .call(
            "credential.create",
            json!({ "providerId": "anthropic", "label": "harness", "secret": "sk-ant-harness" }),
        )
        .await;
    assert_eq!(keyed["ok"], true, "{keyed}");
    let with_key = client.call("welcome", json!({})).await;
    assert_eq!(with_key["result"]["canRun"], true, "{with_key}");
    assert_eq!(with_key["result"]["providers"], json!(["Anthropic"]));
    assert_eq!(
        with_key["result"]["teammates"], 0,
        "step two is still to do"
    );

    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "goal": "Help with whatever I bring you." } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let with_teammate = client.call("welcome", json!({})).await;
    assert_eq!(with_teammate["result"]["teammates"], 1, "{with_teammate}");
    assert_eq!(with_teammate["result"]["setUp"], true);
    assert_eq!(with_teammate["result"]["canRun"], true);

    // Started, the tape carries the chapter marker and nothing said: the
    // composer's starter card reads exactly that, a tape with no user line.
    let tape = client.subscribe(json!({ "tape": persona_id })).await;
    let blank = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "snapshot")
        })
        .await;
    assert_eq!(blank["snapshot"], json!([]));
    let started = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let marker = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "chapter"
        })
        .await;
    assert_eq!(marker["event"].get("endedAt"), None);
    assert!(
        client
            .inbox
            .iter()
            .all(|frame| frame["event"]["kind"] != "user")
    );

    // A revoked key is no way to run: the state reads the credentials, not
    // their history.
    let revoked = client
        .call("credential.revoke", json!({ "id": keyed["result"]["id"] }))
        .await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    let after = client.call("welcome", json!({})).await;
    assert_eq!(after["result"]["canRun"], false, "{after}");
    assert_eq!(after["result"]["providers"], json!([]));

    // Deleting the last teammate empties the room but does not unmake it:
    // the tombstone says a teammate was here, so the window opens on New
    // teammate rather than walking setup again.
    let deleted = client
        .call("persona.delete", json!({ "id": persona_id }))
        .await;
    assert_eq!(deleted["ok"], true, "{deleted}");
    let emptied = client.call("welcome", json!({})).await;
    assert_eq!(emptied["result"]["teammates"], 0, "{emptied}");
    assert_eq!(emptied["result"]["setUp"], true);
}

/// A peer thread, listed and read over the wire.
///
/// The core is the only writer of a stream, so the two teammates and the
/// conversation between them are written into the data directory before the
/// core opens — which is also exactly what an imported one looks like.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_thread_is_listed_streamed_and_marked_read_over_the_wire() {
    let root = scratch("peers");
    let teammate = |id: &str, name: &str| {
        json!({
            "kind": "persona", "id": id, "name": name, "goal": "", "backendId": "hotline",
            "cwd": root.to_string_lossy(), "mcpPolicy": { "mode": "all", "serverIds": [] },
            "sessionCheckpoints": [], "createdAt": 1, "updatedAt": 1,
        })
    };
    std::fs::write(
        root.join("room.jsonl"),
        format!("{}\n{}\n", teammate("ada", "Ada"), teammate("bob", "Bob")),
    )
    .unwrap();
    let threads = root.join("threads");
    std::fs::create_dir_all(&threads).unwrap();
    std::fs::write(
        threads.join("ada~bob.json"),
        json!({
            "version": 1, "a": "ada", "b": "bob",
            "sides": { "user": "ada", "agent": "bob" },
            "sessions": [], "createdAt": 1, "updatedAt": 1,
        })
        .to_string(),
    )
    .unwrap();
    let said = [
        json!({ "kind": "user", "id": "u1", "ts": 10, "text": "did the crane jam?", "receipt": "read" }),
        json!({ "kind": "agent", "id": "a1", "ts": 11, "text": "on the second lift", "receipt": "sent" }),
        json!({ "kind": "turn", "id": "t1", "ts": 12, "stopReason": "end_turn" }),
    ];
    std::fs::write(
        threads.join("ada~bob.jsonl"),
        said.iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let mut client = Client::connect(open_at(&root), TOKEN).await;
    let thread = client.subscribe(json!({ "thread": "ada~bob" })).await;
    let snapshot = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, thread, "snapshot")
        })
        .await;
    assert_eq!(snapshot["snapshot"], json!(said));

    // The same conversation from each side, named by the other.
    let listed = client
        .call("peers.list", json!({ "personaId": "ada" }))
        .await;
    assert_eq!(listed["ok"], true, "{listed}");
    let summary = &listed["result"][0];
    assert_eq!(summary["threadKey"], "ada~bob");
    assert_eq!(summary["withPersonaId"], "bob");
    assert_eq!(summary["withName"], "Bob");
    assert_eq!(summary["exchanges"], 1);
    assert_eq!(summary["lastAt"], 12);
    assert_eq!(summary["waiting"], false);
    assert_eq!(summary["preview"]["fromName"], "Bob");
    assert_eq!(summary["preview"]["text"], "on the second lift");
    let theirs = client
        .call("peers.list", json!({ "personaId": "bob" }))
        .await;
    assert_eq!(theirs["result"][0]["withName"], "Ada");

    // The reply is read, and the subscription is told on the same id.
    let read = client
        .call(
            "peers.mark_read",
            json!({ "key": "ada~bob", "eventIds": ["a1"] }),
        )
        .await;
    assert_eq!(read["result"], 1, "{read}");
    let moved = client
        .next_where(Duration::from_secs(5), |frame| {
            is_sub(frame, thread, "event")
        })
        .await;
    assert_eq!(moved["event"]["id"], "a1");
    assert_eq!(moved["event"]["receipt"], "read");

    // A receipt that has already landed moves nothing the second time.
    let again = client
        .call(
            "peers.mark_read",
            json!({ "key": "ada~bob", "eventIds": ["a1", "no-such-id"] }),
        )
        .await;
    assert_eq!(again["result"], 0, "{again}");
}

#[tokio::test]
async fn the_desktop_restart_lease_refuses_new_wire_work_and_keeps_saved_data() {
    let root = scratch("restart-wire");
    let desk = Arc::new(common::open_desk(&root).unwrap());
    assert!(
        desk.prepare_restart().is_ok(),
        "An empty installation can update too"
    );
    let parts = desk.parts();
    let door = Door::bind(desk.log.clone(), TOKEN.into(), desk.clone(), parts).unwrap();
    let port = door.port();
    let door_task = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let made = client
        .call("persona.create", json!({"draft":{"name":"Update tester"}}))
        .await;
    assert_eq!(made["ok"], true, "{made}");
    let id = made["result"]["id"].as_str().unwrap();
    let saved = desk.log.load(&hotline_core::log::StreamId::Room);
    let held = desk.prepare_restart().unwrap();
    let start = client.call("session.start", json!({"personaId":id})).await;
    assert_eq!(start["ok"], false, "{start}");
    assert!(start.to_string().contains("restart"), "{start}");
    let prompt = client
        .call(
            "session.prompt",
            json!({"personaId":id,"text":"Wait until after the update"}),
        )
        .await;
    assert_eq!(prompt["ok"], false, "{prompt}");
    assert!(prompt.to_string().contains("restart"), "{prompt}");
    assert_eq!(desk.log.load(&hotline_core::log::StreamId::Room), saved);
    drop(held);
    assert!(desk.prepare_restart().is_ok());
    door_task.abort();
    let reopened = common::open_desk(&root).unwrap();
    assert_eq!(reopened.log.load(&hotline_core::log::StreamId::Room), saved);
}

/// The new-teammate form asks the access choices most people decide up
/// front — the whole machine, background work, a computer — so a draft that
/// carries them lands on the teammate as created, not as a later patch, and
/// the record on disk says so from its first line.
#[tokio::test(flavor = "multi_thread")]
async fn a_draft_carrying_access_choices_lands_them_on_the_teammate() {
    let (root, port) = open("access-draft").await;
    let mut client = Client::connect(port, TOKEN).await;

    let created = client
        .call(
            "persona.create",
            json!({ "draft": {
                "name": "Ada",
                "reach": "machine",
                "backgroundWork": true,
                "computer": { "enabled": true }
            } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona = &created["result"];
    assert_eq!(persona["reach"], "machine");
    assert_eq!(persona["backgroundWork"], true);
    assert_eq!(persona["computer"], json!({ "enabled": true }));

    // Left blank, the same choices are off, and the workspace reach is not
    // written down at all.
    let plain = client
        .call("persona.create", json!({ "draft": { "name": "Bob" } }))
        .await;
    assert_eq!(plain["ok"], true, "{plain}");
    assert_eq!(plain["result"]["backgroundWork"], false);
    assert!(plain["result"].get("reach").is_none(), "{plain}");
    assert!(plain["result"].get("computer").is_none(), "{plain}");

    let room = std::fs::read_to_string(root.join("room.jsonl")).unwrap();
    let ada = room
        .lines()
        .find(|line| line.contains("\"name\":\"Ada\""))
        .expect("Ada's record is on the room stream");
    assert!(ada.contains("\"backgroundWork\":true"), "{ada}");
    assert!(ada.contains("\"reach\":\"machine\""), "{ada}");
}

/// User readback is a send-time snapshot, not permission to reopen whichever
/// file happens to occupy a path from the tape today.
#[tokio::test(flavor = "multi_thread")]
async fn user_images_read_back_by_original_index_without_reopening_the_source() {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use hotline_core::log::{Log, StreamId};
    const CHUNK: usize = 512 * 1024;
    const CAP: u64 = 25 * 1024 * 1024;
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let small = png.into_inner();
    let mut original = small.clone();
    original.resize(CHUNK + 123, 42);
    let first_path = source.path().join("first.jpg");
    let second_path = source.path().join("second.png");
    let text_path = source.path().join("not-an-image.png");
    let large_path = source.path().join("large.png");
    std::fs::write(&first_path, &original).unwrap();
    std::fs::write(&second_path, &small).unwrap();
    std::fs::write(&text_path, "private non-image contents").unwrap();
    std::fs::File::create(&large_path)
        .unwrap()
        .set_len(CAP + 1)
        .unwrap();
    let image = |path: &Path, size: u64| json!({"kind":"image", "name":path.file_name().unwrap().to_str().unwrap(), "path":path, "mimeType":"image/jpeg", "size":size});
    // A legacy event has only a path, and not enough evidence to establish
    // what it pointed at when sent. Seed it before the core starts.
    let log = Log::open(root.path());
    log.append(&StreamId::Tape("legacy".into()), &json!({"kind":"user", "id":"old", "ts":1, "text":"old picture", "attachments":[image(&first_path, original.len() as u64)]})).unwrap();
    log.append(&StreamId::Room, &json!({"kind":"persona", "id":"legacy", "name":"Legacy", "goal":"", "backendId":"hotline", "cwd":root.path(), "mcpPolicy":{"mode":"none","serverIds":[]}, "sessionCheckpoints":[], "createdAt":1, "updatedAt":1})).unwrap();
    drop(log);
    let port = open_at(root.path());
    let mut client = Client::connect(port, TOKEN).await;
    // A disposable local provider is enough to start a real session. No
    // model response is needed to persist the user's line or read it back.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().route("/v1/chat/completions", axum::routing::post(|| async {
            ([("Content-Type", "text/event-stream")], "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Received\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
        }))).await.unwrap();
    });
    let saved = client.call("credential.custom_save", json!({"draft":{"name":"Readback fixture", "baseUrl":base, "api":"chat_completions", "models":["test"]}})).await;
    assert_eq!(saved["ok"], true, "{saved}");
    let created = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Ada", "cwd":root.path()}}),
        )
        .await;
    let persona = created["result"]["id"].as_str().unwrap().to_string();
    let other = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Bob", "cwd":root.path()}}),
        )
        .await;
    let other = other["result"]["id"].as_str().unwrap().to_string();
    let subscription = client.subscribe(json!({"tape":persona})).await;
    let started = client
        .call("session.start", json!({"personaId":persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let legacy = client
        .call("file.read", json!({"personaId":"legacy", "eventId":"old"}))
        .await;
    assert_eq!(legacy["ok"], false, "{legacy}");
    assert!(
        legacy["error"]
            .as_str()
            .unwrap()
            .contains("no retained copy"),
        "{legacy}"
    );
    let sent = client.call("session.prompt", json!({"personaId":persona, "text":"Pictures", "attachments":[
        image(&first_path, original.len() as u64),
        {"kind":"file", "name":"not-an-image.png", "path":text_path, "mimeType":"image/png"},
        image(&second_path, small.len() as u64),
        image(&large_path, CAP + 1),
        image(&text_path, 26),
        image(&source.path().join("missing.png"), 10)
    ]})).await;
    assert_eq!(sent["ok"], true, "{sent}");
    let event = client
        .next_where(Duration::from_secs(10), |frame| {
            is_sub(frame, subscription, "event") && frame["event"]["kind"] == "user"
        })
        .await;
    let id = event["event"]["id"].as_str().unwrap().to_string();
    let params = json!({"personaId":persona,"eventId":id});
    let first = client.call("file.read", params.clone()).await;
    assert_eq!(first["ok"], true, "{first}");
    let first = &first["result"];
    assert_eq!(first["name"], "first.jpg");
    assert_eq!(first["mimeType"], "image/png");
    assert_eq!(first["size"], original.len());
    assert_eq!(first["offset"], 0);
    assert_eq!(first["next"], CHUNK);
    let mut whole = BASE64_STANDARD
        .decode(first["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(whole.len(), CHUNK);
    // Same-size replacement and deletion cannot change either chunk or the
    // other attachment. The MIME is the actual bytes', not the supplied one.
    std::fs::write(&first_path, vec![7; original.len()]).unwrap();
    std::fs::remove_file(&second_path).unwrap();
    let rest = client
        .call(
            "file.read",
            json!({"personaId":persona,"eventId":id,"offset":CHUNK}),
        )
        .await;
    assert_eq!(rest["ok"], true, "{rest}");
    assert_eq!(rest["result"]["size"], original.len());
    assert_eq!(rest["result"]["offset"], CHUNK);
    assert!(rest["result"].get("next").is_none());
    whole.extend(
        BASE64_STANDARD
            .decode(rest["result"]["data"].as_str().unwrap())
            .unwrap(),
    );
    assert_eq!(whole, original);
    let second = client
        .call(
            "file.read",
            json!({"personaId":persona,"eventId":id,"index":2}),
        )
        .await;
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(
        BASE64_STANDARD
            .decode(second["result"]["data"].as_str().unwrap())
            .unwrap(),
        small
    );
    let zero = client
        .call(
            "file.read",
            json!({"personaId":persona,"eventId":id,"index":0}),
        )
        .await;
    assert_eq!(&zero["result"], first);
    let end = client
        .call(
            "file.read",
            json!({"personaId":persona,"eventId":id,"offset":original.len()}),
        )
        .await;
    assert_eq!(end["ok"], true, "{end}");
    assert_eq!(end["result"]["data"], "");
    assert!(end["result"].get("next").is_none());
    for denied in [
        json!({"index":-1}),
        json!({"index":1.5}),
        json!({"index":"2"}),
        json!({"index":6}),
        json!({"index":1}),
        json!({"index":3}),
        json!({"index":4}),
        json!({"index":5}),
        json!({"offset":-1}),
        json!({"offset":original.len()+1}),
        json!({"personaId":other}),
        json!({"personaId":"nobody"}),
        json!({"eventId":"missing"}),
        json!({"eventId":"../old"}),
        json!({"eventId":first_path}),
        json!({"personaId":"legacy","eventId":"old"}),
    ] {
        let mut request = params.clone();
        request
            .as_object_mut()
            .unwrap()
            .extend(denied.as_object().unwrap().clone());
        let refused = client.call("file.read", request.clone()).await;
        assert_eq!(refused["ok"], false, "{request}: {refused}");
        assert!(refused.get("result").is_none());
    }
    // A caller's extra path cannot select different bytes; the only address
    // is the message. Loss of a retained copy does not fall back to a source.
    let mut request = params.clone();
    request["path"] = json!(text_path);
    assert_eq!(client.call("file.read", request).await["result"], *first);
    let retained = hotline_core::paths::sent_file_dir(root.path(), &persona, &id)
        .unwrap()
        .join("user/0");
    std::fs::remove_file(&retained).unwrap();
    let refused = client.call("file.read", params.clone()).await;
    assert_eq!(refused["ok"], false, "{refused}");
    // A symlink planted where the copy belonged is not a retained image.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&first_path, &retained).unwrap();
        assert_eq!(client.call("file.read", params.clone()).await["ok"], false);
        std::fs::remove_file(&retained).unwrap();
    }
    // The actual retained-file size is also capped, even if tape metadata
    // claims it is smaller (e.g. damage in the data directory).
    std::fs::File::create(&retained)
        .unwrap()
        .set_len(CAP + 1)
        .unwrap();
    let refused = client.call("file.read", params).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused["error"].as_str().unwrap().contains("25 MB"));
    client
        .call("session.stop", json!({"personaId":persona}))
        .await;
    server.abort();
}
