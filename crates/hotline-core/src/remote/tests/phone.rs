use super::*;

#[tokio::test]
async fn a_phone_sees_the_real_roster_and_tape_but_cannot_administer_the_desk() {
    let h = Harness::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move |body: String| {
        captured.lock().unwrap().push(serde_json::from_str(&body).unwrap());
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async {
            let chunk = json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{"role":"assistant","content":"A real core turn"},"finish_reason":null}]});
            let end = json!({"id":"chat_1","object":"chat.completion.chunk","created":1,"model":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
            ([("Content-Type", "text/event-stream")], format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
        }
    }));
    let model = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    h.desk.credential_custom_save(None, serde_json::from_value(json!({"name":"Fixture", "baseUrl":base, "api":"chat_completions", "secret":"", "models":["test"]})).unwrap()).unwrap();
    let parts = h.desk.parts();
    let door = Door::bind(
        h.desk.log.clone(),
        "test-desk".into(),
        h.desk.clone(),
        parts,
    )
    .unwrap();
    let port = door.port();
    let task = tokio::spawn(door.run());
    let (mut desk, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=test-desk"))
            .await
            .unwrap();
    desk.send(Message::text(json!({"id":1,"cmd":"persona.create","params":{"draft":{"name":"Ada","goal":"Help with the project","cwd":h.root.path().to_str().unwrap()}}}).to_string())).await.unwrap();
    let created: Value =
        serde_json::from_str(desk.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    let persona = created["result"]["id"].as_str().unwrap();
    // The window may say the person is at it; a phone may not, or it could
    // keep its own notifications away.
    desk.send(Message::text(
        json!({"id":2,"cmd":"desk.looking","params":{"looking":true}}).to_string(),
    ))
    .await
    .unwrap();
    let looking: Value =
        serde_json::from_str(desk.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(looking["ok"], true, "{looking}");
    let grant = h.pair_companion().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    read(&mut phone).await;
    for frame in [
        json!({"id":1,"cmd":"credential.list"}),
        json!({"id":1,"cmd":"settings.update","params":{"patch":{}}}),
        json!({"id":1,"sub":"room"}),
        json!({"id":1,"cmd":"session.prompt","params":{"personaId":persona,"text":"bad","attachments":[]}}),
        json!({"id":1,"cmd":"desk.looking","params":{"looking":true}}),
    ] {
        send(&mut phone, frame).await;
        assert_eq!(read(&mut phone).await["ok"], false);
    }
    send(&mut phone, json!({"id":2,"sub":{"view":"roster"}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);
    assert_eq!(
        read(&mut phone).await["snapshot"][0]["persona"]["name"],
        "Ada"
    );
    send(&mut phone, json!({"id":3,"sub":{"tape":persona}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);
    assert_eq!(read(&mut phone).await["snapshot"], json!([]));
    // The core remains the writer; this tests that its append reaches the TLS subscription.
    h.desk
        .log
        .append(
            &crate::log::StreamId::Tape(persona.into()),
            &json!({"id":"one","kind":"agent","ts":1,"text":"From the real tape"}),
        )
        .unwrap();
    loop {
        let frame = read(&mut phone).await;
        if frame["sub"] == 3 {
            assert_eq!(frame["event"]["text"], "From the real tape");
            break;
        }
    }
    // The phone is the first client to start this teammate. Retransmitting
    // its operation after acceptance must not start a second model turn.
    let attachment = Uuid::new_v4();
    send(&mut phone, json!({"id":30,"cmd":"mobile.attachment","params":{"upload":{"id":attachment,"name":"brief.txt","mimeType":"text/plain","size":5,"offset":0,"data":"aGVsbG8="}}})).await;
    loop {
        let frame = read(&mut phone).await;
        if frame["id"] == 30 {
            assert_eq!(frame["result"]["complete"], true, "{frame}");
            break;
        }
    }
    let operation = Uuid::new_v4();
    send(&mut phone, json!({"id":4,"cmd":"mobile.prompt","params":{"operationId":operation,"personaId":persona,"text":"","attachmentIds":[attachment],"replyTo":"answered-1"}})).await;
    let mut accepted = false;
    let mut answered = false;
    while !accepted || !answered {
        let frame = read(&mut phone).await;
        if frame["id"] == 4 {
            assert_eq!(frame["result"]["state"], "accepted", "{frame}");
            accepted = true;
        }
        if frame["sub"] == 3
            && frame["event"]["kind"] == "agent"
            && frame["event"]["text"] == "A real core turn"
        {
            answered = true;
        }
    }
    send(&mut phone, json!({"id":5,"cmd":"mobile.prompt","params":{"operationId":operation,"personaId":persona,"text":"","attachmentIds":[attachment],"replyTo":"answered-1"}})).await;
    loop {
        let frame = read(&mut phone).await;
        if frame["id"] == 5 {
            assert_eq!(frame["result"]["state"], "accepted");
            break;
        }
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let tape = h.desk.log.load(&crate::log::StreamId::Tape(persona.into()));
    let users: Vec<_> = tape
        .iter()
        .filter(|event| event["kind"] == "user")
        .collect();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["text"], "");
    assert_eq!(users[0]["attachments"][0]["name"], "brief.txt");
    assert_eq!(users[0]["replyTo"], "answered-1");
    let path = users[0]["attachments"][0]["path"].as_str().unwrap();
    assert_eq!(fs::read(path).unwrap(), b"hello");
    // The request is JSON, where a Windows path's backslashes are escaped.
    let path_in_json = serde_json::to_string(path).unwrap();
    assert!(
        requests.lock().unwrap()[0]
            .to_string()
            .contains(path_in_json.trim_matches('"'))
    );
    send(&mut phone, json!({"id":6,"cmd":"mobile.prompt","params":{"operationId":operation,"personaId":persona,"text":"changed","attachmentIds":[]}})).await;
    loop {
        let frame = read(&mut phone).await;
        if frame["id"] == 6 {
            assert_eq!(frame["ok"], false);
            break;
        }
    }
    // Image uploads follow the same send-time retention path as the desk.
    // A phone can refetch its own evicted copy, but not a user text file.
    use base64::{Engine, prelude::BASE64_STANDARD};
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let image_id = Uuid::new_v4();
    send(&mut phone, json!({"id":31,"cmd":"mobile.attachment","params":{"upload":{"id":image_id,"name":"picture.png","mimeType":"image/png","size":png.len(),"offset":0,"data":BASE64_STANDARD.encode(&png)}}})).await;
    loop {
        let frame = read(&mut phone).await;
        if frame["id"] == 31 {
            assert_eq!(frame["result"]["complete"], true, "{frame}");
            break;
        }
    }
    send(&mut phone, json!({"id":32,"cmd":"mobile.prompt","params":{"operationId":Uuid::new_v4(),"personaId":persona,"text":"picture","attachmentIds":[image_id]}})).await;
    loop {
        let frame = read(&mut phone).await;
        if frame["id"] == 32 {
            assert_eq!(frame["result"]["state"], "accepted", "{frame}");
            break;
        }
    }
    let tape = h.desk.log.load(&crate::log::StreamId::Tape(persona.into()));
    let picture = tape
        .iter()
        .find(|event| event["kind"] == "user" && event["text"] == "picture")
        .unwrap();
    // Losing the original uploaded payload does not lose readback.
    fs::remove_file(picture["attachments"][0]["path"].as_str().unwrap()).unwrap();
    for (request_id, event, allowed) in [(33, picture, true), (34, users[0], false)] {
        send(&mut phone, json!({"id":request_id,"cmd":"file.read","params":{"personaId":persona,"eventId":event["id"]}})).await;
        loop {
            let frame = read(&mut phone).await;
            if frame["id"] == request_id {
                assert_eq!(frame["ok"], allowed, "{frame}");
                if allowed {
                    assert_eq!(frame["result"]["mimeType"], "image/png");
                    assert_eq!(
                        BASE64_STANDARD
                            .decode(frame["result"]["data"].as_str().unwrap())
                            .unwrap(),
                        png
                    );
                    assert!(frame["result"].get("next").is_none());
                }
                break;
            }
        }
    }
    h.remote.configure(false, network::ALL).await.unwrap();
    model.abort();
    task.abort();
}

/// A paired phone builds its own teammate through the real door: `backends.list`
/// answers with no secret, `mobile.persona_create` makes a confined teammate a
/// retry cannot duplicate, and the full `persona.create` stays refused, exactly
/// as it is for every other posture-changing command.
#[tokio::test]
async fn a_phone_creates_a_confined_teammate_but_not_through_persona_create() {
    let h = Harness::new().await;

    let grant = h.pair_companion().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    read(&mut phone).await;

    send(&mut phone, json!({"id": 1, "cmd": "backends.list"})).await;
    let backends = read(&mut phone).await;
    assert_eq!(backends["ok"], true, "{backends}");
    let choices = backends["result"].as_array().unwrap();
    assert!(
        choices
            .iter()
            .any(|choice| choice["id"] == "hotline" && choice["unavailable"].is_null()),
        "{choices:?}"
    );
    for choice in choices {
        let fields: Vec<&str> = choice
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for field in fields {
            assert!(
                ["id", "name", "description", "unavailable"].contains(&field),
                "backends.list carried an unexpected field {field}"
            );
        }
    }

    send(
        &mut phone,
        json!({
            "id": 2,
            "cmd": "persona.create",
            "params": {"draft": {"name": "Ada"}},
        }),
    )
    .await;
    let refused = read(&mut phone).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "forbidden");

    let request_id = Uuid::new_v4().to_string();
    let create = json!({
        "id": 3,
        "cmd": "mobile.persona_create",
        "params": {"requestId": request_id, "name": "Ada", "goal": "Help with the harbour"},
    });
    send(&mut phone, create.clone()).await;
    let created = read(&mut phone).await["result"].clone();
    assert_eq!(created["id"], request_id);
    assert_eq!(created["name"], "Ada");
    assert!(created.get("reach").is_none(), "{created}");
    assert!(created.get("computer").is_none(), "{created}");
    assert_eq!(created["backgroundWork"], false);

    let mut retry = create;
    retry["id"] = json!(4);
    send(&mut phone, retry).await;
    let retried = read(&mut phone).await["result"].clone();
    assert_eq!(retried, created);
    assert_eq!(
        crate::room::roster(&h.desk.log)
            .into_iter()
            .filter(|persona| persona.id == request_id)
            .count(),
        1
    );

    h.remote.configure(false, network::ALL).await.unwrap();
}

/// A paired phone renames, re-aims and deletes a teammate through the real
/// door, but `persona.update` — the patch that reaches a teammate's grants —
/// still answers `forbidden` for that seat.
#[tokio::test]
async fn a_phone_renames_and_deletes_a_teammate_but_not_through_persona_update() {
    let h = Harness::new().await;
    let grant = h.pair_companion().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    read(&mut phone).await;

    let request_id = Uuid::new_v4().to_string();
    send(
        &mut phone,
        json!({
            "id": 1,
            "cmd": "mobile.persona_create",
            "params": {"requestId": request_id, "name": "Ada"},
        }),
    )
    .await;
    assert_eq!(read(&mut phone).await["ok"], true);

    send(
        &mut phone,
        json!({
            "id": 2,
            "cmd": "persona.update",
            "params": {"id": request_id, "patch": {"reach": "machine"}},
        }),
    )
    .await;
    let refused = read(&mut phone).await;
    assert_eq!(refused["code"], "forbidden", "{refused}");

    send(
        &mut phone,
        json!({
            "id": 3,
            "cmd": "mobile.persona_update",
            "params": {"id": request_id, "name": "Ada Lovelace", "goal": "Chart the harbour"},
        }),
    )
    .await;
    let renamed = read(&mut phone).await;
    assert_eq!(renamed["result"]["name"], "Ada Lovelace", "{renamed}");
    assert_eq!(renamed["result"]["goal"], "Chart the harbour");
    assert!(renamed["result"].get("reach").is_none());

    send(
        &mut phone,
        json!({"id": 4, "cmd": "persona.delete", "params": {"id": request_id}}),
    )
    .await;
    assert_eq!(read(&mut phone).await["ok"], true);
    assert!(
        crate::room::roster(&h.desk.log)
            .iter()
            .all(|persona| persona.id != request_id)
    );

    h.remote.configure(false, network::ALL).await.unwrap();
}

/// A paired phone reads one teammate's schedules through the real door: an
/// empty list said out loud, each change as the whole list, a refusal for
/// anything that would change a job or reach the room, and on reconnecting
/// the list as it now is, with what changed while it was away. A revoked
/// phone reads nothing.
#[tokio::test]
async fn a_phone_reads_a_teammates_schedules_and_catches_up_on_what_changed_while_away() {
    let h = Harness::new().await;
    let parts = h.desk.parts();
    let door = Door::bind(
        h.desk.log.clone(),
        "test-desk".into(),
        h.desk.clone(),
        parts,
    )
    .unwrap();
    let port = door.port();
    let task = tokio::spawn(door.run());
    let (mut desk, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=test-desk"))
            .await
            .unwrap();
    let mut asked = 0;
    let mut at_desk = async |cmd: &str, params: Value| -> Value {
        asked += 1;
        desk.send(Message::text(
            json!({"id": asked, "cmd": cmd, "params": params}).to_string(),
        ))
        .await
        .unwrap();
        loop {
            let frame: Value =
                serde_json::from_str(desk.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            if frame["id"] == asked {
                assert_eq!(frame["ok"], true, "{cmd}: {frame}");
                return frame["result"].clone();
            }
        }
    };
    let cwd = h.root.path().to_str().unwrap();
    let ada = at_desk(
        "persona.create",
        json!({"draft": {"name": "Ada", "goal": "Keep the harbour running", "cwd": cwd}}),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = at_desk(
        "persona.create",
        json!({"draft": {"name": "Bob", "goal": "Count the boats", "cwd": cwd}}),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let grant = h.pair_companion().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    // The hello is how a phone knows it may ask; a desk from before this
    // list names nothing, and the phone then shows no schedules section. A
    // companion keeps the smaller command set.
    assert_eq!(
        read(&mut phone).await["capabilities"],
        json!([
            "personaCreate",
            "personaEdit",
            "schedules",
            "threads",
            "runs",
            "threads2",
            "rosterOptions",
            "lean",
            "thumbnails",
            "turnRetry"
        ])
    );

    send(&mut phone, json!({"id": 1, "sub": {"schedules": ada}})).await;
    assert_eq!(read(&mut phone).await, json!({"id": 1, "ok": true}));
    assert_eq!(read(&mut phone).await, json!({"sub": 1, "snapshot": []}));

    let sweep = at_desk(
        "schedule.create",
        json!({"personaId": ada, "kind": "loop", "every": 3_600_000, "prompt": "Sweep the inbox"}),
    )
    .await;
    let listed = read(&mut phone).await;
    assert_eq!(listed["sub"], 1, "{listed}");
    let entry = &listed["snapshot"][0];
    assert_eq!(entry["id"], sweep["id"]);
    assert_eq!(entry["personaId"], ada.as_str());
    assert_eq!(entry["kind"], "loop");
    assert_eq!(entry["every"], 3_600_000);
    assert_eq!(entry["prompt"], "Sweep the inbox");
    assert_eq!(entry["nextAt"], sweep["nextAt"]);
    // Who made a job is the desk's business, not a display field.
    assert!(entry.get("operatorCreated").is_none(), "{entry}");
    assert_eq!(listed["snapshot"].as_array().unwrap().len(), 1);

    let refusals = [
        json!({"id": 2, "cmd": "schedule.create", "params": {"personaId": ada, "kind": "loop", "every": 60_000, "prompt": "Not from here"}}),
        json!({"id": 3, "cmd": "schedule.cancel", "params": {"id": sweep["id"]}}),
        json!({"id": 4, "cmd": "schedule.set_quiet", "params": {"id": sweep["id"], "quiet": true}}),
        json!({"id": 5, "cmd": "schedule.list"}),
        json!({"id": 6, "sub": "room"}),
    ];
    for frame in refusals {
        let id = frame["id"].clone();
        send(&mut phone, frame).await;
        let refused = read(&mut phone).await;
        assert_eq!(refused["id"], id, "{refused}");
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "forbidden", "{refused}");
    }
    send(&mut phone, json!({"id": 7, "sub": {"schedules": "nobody"}})).await;
    let unknown = read(&mut phone).await;
    assert_eq!(unknown["ok"], false, "{unknown}");
    assert_eq!(unknown["code"], "unknown_teammate", "{unknown}");
    // Nothing the phone asked for moved a job.
    assert_eq!(h.desk.schedule_list().len(), 1);

    // Away: the loop is cancelled, a one-shot is made, and Bob gets a job of
    // his own. Back, the list is what is true now.
    phone.socket.close(None).await.unwrap();
    drop(phone);
    at_desk("schedule.cancel", json!({"id": sweep["id"]})).await;
    let when = crate::session::now_ms() + 3_600_000;
    let once = at_desk(
        "schedule.create",
        json!({"personaId": ada, "kind": "schedule", "when": when, "prompt": "Check the crane"}),
    )
    .await;
    at_desk(
        "schedule.create",
        json!({"personaId": bob, "kind": "loop", "every": 3_600_000, "prompt": "Count the boats"}),
    )
    .await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    read(&mut phone).await;
    send(&mut phone, json!({"id": 1, "sub": {"schedules": ada}})).await;
    assert_eq!(read(&mut phone).await, json!({"id": 1, "ok": true}));
    let now = read(&mut phone).await;
    let jobs: Vec<&Value> = now["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job.get("endedAt").is_none())
        .collect();
    assert_eq!(jobs.len(), 1, "{now}");
    assert_eq!(jobs[0]["id"], once["id"]);
    assert_eq!(jobs[0]["kind"], "schedule");
    assert_eq!(jobs[0]["when"], when);
    assert_eq!(jobs[0]["nextAt"], when);
    let ended: Vec<&Value> = now["snapshot"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job.get("endedAt").is_some())
        .collect();
    assert_eq!(
        ended.len(),
        1,
        "the cancelled loop is listed as ended: {now}"
    );
    assert_eq!(ended[0]["id"], sweep["id"]);

    // Revoked, the phone's socket closes and cannot be opened again.
    h.remote.revoke(&grant.id).unwrap();
    let closed = tokio::time::timeout(ANSWER, phone.socket.next())
        .await
        .unwrap();
    assert!(
        !matches!(closed, Some(Ok(Message::Text(_) | Message::Binary(_)))),
        "{closed:?}"
    );
    assert!(h.socket(&grant.private).await.is_none());

    h.remote.configure(false, network::ALL).await.unwrap();
    task.abort();
}

/// A phone says where to notify it; the desk keeps that on the grant, and the
/// room reads it back without holding the remote.
#[tokio::test]
async fn a_phone_registers_where_to_notify_it_and_the_room_can_read_it() {
    let h = Harness::new().await;
    let grant = h.pair().await;
    let phone = h.remote.authenticate_v2(&grant.public).unwrap();
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        [] as [String; 0]
    );
    assert!(
        phone
            .register_push("not a token".into(), "ios".into())
            .is_err()
    );
    assert!(
        phone
            .register_push("ExponentPushToken[abc]".into(), "watch".into())
            .is_err()
    );
    let mut socket = h.socket(&grant.private).await.unwrap();
    assert_eq!(read(&mut socket).await["protocolVersion"], 2);
    send(
        &mut socket,
        json!({"id":1,"cmd":"mobile.push_register","params":{"token":"ExponentPushToken[abc]", "platform":"ios"}}),
    )
    .await;
    let registered = read(&mut socket).await;
    assert_eq!(registered["id"], 1);
    assert_eq!(registered["ok"], true, "{registered}");
    let targets = crate::remote::push_targets(h.root.path());
    assert_eq!(targets.tokens, ["ExponentPushToken[abc]"]);
    assert_eq!(targets.desktop_id, h.remote.status_desktop_id());
    // A fresh token replaces the old one; revoking the phone drops it.
    phone
        .register_push("ExponentPushToken[def]".into(), "ios".into())
        .unwrap();
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        ["ExponentPushToken[def]"]
    );
    // Turning Remote off keeps the phone paired but stops notifying it.
    h.remote.configure(false, network::ALL).await.unwrap();
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        [] as [String; 0]
    );
    h.remote.configure(true, network::ALL).await.unwrap();
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        ["ExponentPushToken[def]"]
    );
    h.remote.revoke(&phone.id).unwrap();
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        [] as [String; 0]
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}

/// An owner's phone has the desk's authority but a phone's link: once it
/// names `lean`, its history is cut the way a companion's is, its window
/// is as short as it asks, its roster leaves out the desk's model list,
/// and `ping` answers. Without `lean` it is sent the desk's full lines.
#[tokio::test]
async fn an_owner_phone_that_names_lean_is_sent_phone_sized_frames() {
    let h = Harness::new().await;
    let parts = h.desk.parts();
    let door = Door::bind(
        h.desk.log.clone(),
        "test-desk".into(),
        h.desk.clone(),
        parts,
    )
    .unwrap();
    let port = door.port();
    let task = tokio::spawn(door.run());
    let (mut desk, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=test-desk"))
            .await
            .unwrap();
    desk.send(Message::text(json!({"id":1,"cmd":"persona.create","params":{"draft":{"name":"Ada","goal":"Help","cwd":h.root.path().to_str().unwrap()}}}).to_string())).await.unwrap();
    let created: Value =
        serde_json::from_str(desk.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    let persona = created["result"]["id"].as_str().unwrap().to_string();
    let tape = crate::log::StreamId::Tape(persona.clone());
    let long = "word ".repeat(4000);
    for (index, event) in [
        json!({"id":"said","kind":"agent","ts":1,"text":long}),
        json!({"id":"tool","kind":"tool","ts":2,"toolCallId":"t","title":"Read","status":"completed","output":[{"type":"text","text":"x"}]}),
        json!({"id":"last","kind":"agent","ts":3,"text":"short"}),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = index;
        h.desk.log.append(&tape, &event).unwrap();
    }

    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    let hello = read(&mut phone).await;
    assert!(
        hello["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("lean")),
        "{hello}"
    );

    // Before it says so, an owner phone is sent the lines whole.
    send(&mut phone, json!({"id":2,"sub":{"tape":persona}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);
    let whole = read(&mut phone).await;
    assert_eq!(whole["snapshot"][0]["text"], json!(long));
    assert!(whole["snapshot"][1].get("output").is_some());

    send(
        &mut phone,
        json!({"id":3,"cmd":"client.hello","params":{"capabilities":["threads2","lean"]}}),
    )
    .await;
    assert_eq!(read(&mut phone).await["ok"], true);
    send(&mut phone, json!({"id":4,"cmd":"ping","params":{}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);

    send(
        &mut phone,
        json!({"id":5,"sub":{"tape":persona},"window":2}),
    )
    .await;
    assert_eq!(read(&mut phone).await["ok"], true);
    let lean = read(&mut phone).await;
    let lines = lean["snapshot"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{lean}");
    assert_eq!(lines[0]["id"], "tool");
    assert!(lines[0].get("output").is_none());
    assert_eq!(lines[0]["mobileTruncated"], true);

    send(
        &mut phone,
        json!({"id":6,"cmd":"tape.page","params":{"personaId":persona,"before":"tool"}}),
    )
    .await;
    let page = read(&mut phone).await;
    let said = &page["result"]["events"][0];
    assert_eq!(said["mobileTruncated"], true, "{page}");
    assert!(said["text"].as_str().unwrap().len() < 4000);

    send(&mut phone, json!({"id":7,"sub":{"view":"roster"}})).await;
    assert_eq!(read(&mut phone).await["ok"], true);
    let roster = read(&mut phone).await;
    assert_eq!(roster["snapshot"][0]["session"]["slashCommands"], json!([]));
    task.abort();
}

#[tokio::test]
async fn a_phone_sees_its_own_grant_marked_current_and_no_other() {
    let h = Harness::new().await;
    let other = h.pair().await;
    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    assert_eq!(read(&mut phone).await["type"], "hello");
    send(&mut phone, json!({"id": 1, "cmd": "remote.devices"})).await;
    let listed = read(&mut phone).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let devices = listed["result"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    for device in devices {
        let ours = device["id"] == grant.id.as_str();
        assert!(ours || device["id"] == other.id.as_str());
        assert_eq!(device.get("current"), ours.then_some(&json!(true)));
    }
    // The mark is put on the answer, never on the saved grant.
    assert!(
        h.remote
            .devices()
            .iter()
            .all(|device| device.current.is_none())
    );
    let saved = fs::read_to_string(h.root.path().join("remote.json")).unwrap();
    assert!(!saved.contains("current"));
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn the_owner_hello_says_what_kind_of_desk_and_which_version() {
    let h = Harness::new().await;
    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    let hello = read(&mut phone).await;
    assert_eq!(hello["deskKind"], "desktop");
    assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(hello["serves"], json!(["agents"]));
    let companion = h.pair_companion().await;
    let mut companion = h.socket(&companion.private).await.unwrap();
    let hello = read(&mut companion).await;
    assert_eq!(hello.get("deskKind"), None);
    assert_eq!(hello.get("serves"), None);
    assert_eq!(hello.get("version"), None);
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn a_companion_hello_marks_the_public_endpoint_only_when_one_is_set() {
    let h = Harness::new().await;
    let grant = h.pair_companion().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    assert_eq!(read(&mut phone).await.get("publicEndpoint"), None);
    h.remote
        .set_public_url(Some("https://desk.example.com"))
        .unwrap();
    let mut phone = h.socket(&grant.private).await.unwrap();
    let hello = read(&mut phone).await;
    assert_eq!(hello["publicEndpoint"], "https://desk.example.com");
    // The endpoints keep this computer's own addresses first.
    let endpoints = hello["endpoints"].as_array().unwrap();
    assert_eq!(endpoints[0], h.endpoint().as_str());
    assert!(endpoints.contains(&json!("https://desk.example.com")));
    h.remote.configure(false, network::ALL).await.unwrap();
}

/// A panic that poisons Remote's state must not take the listener's
/// authority checks down with it, nor leave a revoked phone a way in: the
/// state is recovered, a paired phone still connects, and a revoke still
/// closes its socket and refuses the next one.
#[tokio::test]
async fn a_poisoned_remote_still_admits_its_phones_and_still_revokes_them() {
    let h = Harness::new().await;
    let kept = h.pair().await;
    let revoked = h.pair().await;
    let mut socket = h.socket(&revoked.private).await.unwrap();
    assert_eq!(read(&mut socket).await["protocolVersion"], 2);
    crate::remote::poison(&h.remote.state);

    let mut other = h.socket(&kept.private).await.unwrap();
    assert_eq!(read(&mut other).await["protocolVersion"], 2);
    h.remote.revoke(&revoked.id).unwrap();
    sealed_network::closed(&mut socket.socket).await;
    assert!(h.socket(&revoked.private).await.is_none());
    assert_eq!(h.remote.devices().len(), 1);

    h.remote.configure(false, network::ALL).await.unwrap();
}

/// Revocation ends a healthy seat without discarding its buffered replies.
#[tokio::test]
async fn a_revoked_seat_drains_tcp_instead_of_resetting_it() {
    use tokio::io::AsyncReadExt;

    let h = Harness::new().await;
    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    assert_eq!(read(&mut phone).await["protocolVersion"], 2);
    h.remote.revoke(&grant.id).unwrap();

    let tcp = phone.socket.get_mut().get_mut().0;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = [0; 65536];
        while tcp
            .read(&mut bytes)
            .await
            .expect("a normal close must not reset TCP")
            != 0
        {}
    })
    .await
    .expect("the revoked transport drains and closes");
    drop(phone);
    h.remote.configure(false, network::ALL).await.unwrap();
}

/// A phone that leaves the network stops reading without closing, and TCP
/// may keep its socket up for many minutes. The desk ends the seat itself
/// once a frame has waited past the write bound, though the seat is quiet
/// (its queue never fills) and never asked for the silence timer (no `lean`).
#[tokio::test]
async fn a_seat_that_stops_reading_is_closed_once_a_frame_waits_past_the_bound() {
    let (h, mut phone) = stalled_phone().await;
    // Bound each wait for progress; the whole drain may take longer under load.
    use tokio::io::AsyncReadExt;
    let tcp = phone.socket.get_mut().get_mut().0;
    let mut bytes = [0; 65536];
    let reset = loop {
        match tokio::time::timeout(Duration::from_secs(5), tcp.read(&mut bytes))
            .await
            .expect("the stalled transport makes progress until it resets")
        {
            Ok(0) => panic!("the stalled seat closed without resetting TCP"),
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    assert_eq!(reset.kind(), std::io::ErrorKind::ConnectionReset);
    drop(phone);
    h.remote.configure(false, network::ALL).await.unwrap();
}

/// Admission cannot report a stalled seat gone while TCP is still open.
/// The one-second ceiling is the task's transport requirement, measured only
/// after the paused deadline clock has resumed and admission reaches zero.
#[tokio::test]
async fn a_stalled_seat_resets_tcp_within_one_second_of_releasing_admission() {
    let (h, mut phone) = stalled_phone().await;
    use tokio::io::AsyncReadExt;
    let tcp = phone.socket.get_mut().get_mut().0;
    let reset = tokio::time::timeout(Duration::from_secs(1), async {
        let mut bytes = [0; 65536];
        loop {
            match tcp.read(&mut bytes).await {
                Ok(0) => panic!("the stalled seat closed without resetting TCP"),
                Ok(_) => {}
                Err(error) => break error,
            }
        }
    })
    .await
    .expect("TCP resets within one second of admission reaching zero");
    assert_eq!(reset.kind(), std::io::ErrorKind::ConnectionReset);
    drop(phone);
    h.remote.configure(false, network::ALL).await.unwrap();
}

async fn stalled_phone() -> (Harness, PhoneSocket) {
    let h = Harness::new().await;
    let grant = h.pair().await;
    let mut phone = h.socket(&grant.private).await.unwrap();
    assert_eq!(read(&mut phone).await["protocolVersion"], 2);
    send(&mut phone, json!({"id":1,"cmd":"mobile.persona_create","params":{"requestId":Uuid::new_v4(),"name":"Ada"}})).await;
    let created = read(&mut phone).await;
    assert_eq!(created["ok"], true, "{created}");
    let persona = created["result"]["id"].as_str().unwrap().to_owned();
    send(&mut phone, json!({"id":2,"sub":{"tape":persona}})).await;
    assert_eq!(read(&mut phone).await, json!({"id":2,"ok":true}));
    assert_eq!(read(&mut phone).await["sub"], 2);
    assert_eq!(h.remote.admission.counts(), (0, 1));

    // One frame larger than both ends' socket buffers, which the phone never
    // reads: the desk's writer is left holding the rest of it.
    h.desk
        .log
        .append(
            &crate::log::StreamId::Tape(persona),
            &json!({"id":"big","kind":"agent","ts":1,"text":"x".repeat(24 * 1024 * 1024)}),
        )
        .unwrap();
    // The writer's bound runs on a paused clock, moved past it once the
    // frame is stuck; the sockets around it read real time, so the test's own
    // bound is a count of real waits, not a timer on the clock it moves.
    let mut waits = 0;
    while h.remote.admission.counts() != (0, 0) {
        assert!(
            waits < 75,
            "a seat whose writes back up is ended by the desk"
        );
        waits += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
        tokio::time::pause();
        tokio::time::advance(crate::wire::WRITE_STALL).await;
        tokio::time::resume();
    }
    (h, phone)
}

#[tokio::test]
async fn a_pairing_queued_before_desktop_disconnect_cannot_outlive_its_socket() {
    use crate::wire::RoomHandle;
    let h = Harness::new().await;
    let gate = h.desk.policy_update_lock();
    let held = gate.lock().await;
    let mut parts = h.desk.parts();
    parts.remote = Arc::downgrade(&h.remote);
    let door =
        crate::wire::Door::bind(h.desk.log.clone(), "test".into(), h.desk.clone(), parts).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws?token=test"))
            .await
            .unwrap();
    for frame in [
        json!({"id":1,"cmd":"settings.update","params":{"patch":{"theme":"dark"}}}),
        json!({"id":2,"cmd":"remote.pairing"}),
        json!({"id":3,"cmd":"settings.update","params":{"patch":{"theme":"light"}}}),
        json!({"id":4,"cmd":"client.hello","params":{"capabilities":[]}}),
    ] {
        socket.send(Message::text(frame.to_string())).await.unwrap();
    }
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&reply.into_text().unwrap()).unwrap()["id"],
        4
    );
    socket.close(None).await.unwrap();
    // The server has observed disconnect before the ordered work is released.
    let _ = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    drop(held);
    tokio::time::timeout(Duration::from_secs(5), async {
        while crate::room::settings(&h.desk.log).get("theme") != Some(&json!("light")) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("accepted desk writes drain in order");
    assert!(!h.remote.pairing_open());
    serving.abort();
    h.remote.configure(false, network::ALL).await.unwrap();
}
