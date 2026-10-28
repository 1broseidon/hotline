use super::*;

/// A thread's link is what is stored, and a client on any build is sent the
/// marker its kind has always had: in a snapshot, as an event, and in a page.
#[tokio::test]
async fn a_thread_link_reaches_every_client_as_the_marker_its_kind_always_had() {
    let (_root, log, port) = door("link-wire");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    let link = |state: &str| {
        json!({
            "kind": "link", "id": "link:side:s1", "ts": 5, "thread": "s1",
            "threadKind": "side", "personaId": persona_id, "title": "Mend the crane",
            "state": state
        })
    };
    log.append(&tape, &link("live")).unwrap();
    log.append(&StreamId::Side("s1".into()), &link("live"))
        .unwrap();
    // One written before links, which is sent as it is.
    let old = json!({
        "kind": "subagent", "id": "subagent:r1", "ts": 6, "runId": "r1",
        "title": "Look it up", "status": "done"
    });
    log.append(&tape, &old).unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines[0]["kind"], "side");
    assert_eq!(lines[0]["id"], "link:side:s1");
    assert_eq!(lines[0]["sideId"], "s1");
    assert_eq!(lines[0]["status"], "live");
    assert_eq!(lines[1], old);

    // Rewritten as the thread goes: the event a client folds by id.
    log.append(&tape, &link("parked")).unwrap();
    let event = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(event["event"]["kind"], "side");
    assert_eq!(event["event"]["status"], "parked");

    ask(&mut socket, json!({ "id": 3, "sub": { "side": "s1" } })).await;
    let own = heard_where(&mut socket, |frame| {
        frame["sub"] == 3 && frame["snapshot"].is_array()
    })
    .await;
    assert_eq!(own["snapshot"][0]["kind"], "side");
    assert_eq!(own["snapshot"][0]["personaId"], persona_id);

    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 9, "text": "later" }),
    )
    .unwrap();
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "tape.page", "params": { "personaId": persona_id, "before": "u1" } }),
    )
    .await;
    let page = answered(&mut socket, 4).await;
    assert_eq!(page["result"]["events"][0]["kind"], "side");
    assert_eq!(page["result"]["events"][0]["status"], "parked");
}

/// The same stored lines, for a client that said it reads `threads2`: every
/// link is one `link` shape, an old marker included, and it keeps its id.
#[tokio::test]
async fn a_threads2_client_is_sent_every_link_as_the_link_itself() {
    let (_root, log, port) = door("link-wire-threads2");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let persona_id = ada["id"].as_str().unwrap().to_string();
    let tape = StreamId::Tape(persona_id.clone());
    let link = |state: &str| {
        json!({
            "kind": "link", "id": "link:side:s1", "ts": 5, "thread": "s1",
            "threadKind": "side", "personaId": persona_id, "title": "Mend the crane",
            "state": state
        })
    };
    log.append(&tape, &link("live")).unwrap();
    log.append(
        &tape,
        &json!({
            "kind": "subagent", "id": "subagent:r1", "ts": 6, "runId": "r1",
            "title": "Look it up", "status": "done"
        }),
    )
    .unwrap();

    let hello = hello_threads2(&mut socket, 2).await;
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("threads2")),
        "{hello}"
    );
    ask(
        &mut socket,
        json!({ "id": 3, "sub": { "tape": persona_id } }),
    )
    .await;
    let snapshot = heard_where(&mut socket, |frame| frame["snapshot"].is_array()).await;
    let lines = snapshot["snapshot"].as_array().unwrap();
    assert_eq!(lines[0], link("live"), "a link is sent as it is stored");
    assert_eq!(lines[1]["kind"], "link");
    assert_eq!(lines[1]["id"], "subagent:r1");
    assert_eq!(lines[1]["threadKind"], "run");
    assert_eq!(lines[1]["thread"], "r1");
    assert_eq!(lines[1]["end"], "done");
    // What a client is sent parses as the contract's `link`.
    for line in lines {
        let typed: TranscriptEvent = serde_json::from_value(line.clone()).unwrap();
        assert!(matches!(typed, TranscriptEvent::Link { .. }), "{line}");
    }

    log.append(&tape, &link("parked")).unwrap();
    let event = heard_where(&mut socket, |frame| frame["event"].is_object()).await;
    assert_eq!(event["event"]["kind"], "link");
    assert_eq!(event["event"]["state"], "parked");

    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 9, "text": "later" }),
    )
    .unwrap();
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "thread.page",
                "params": { "thread": { "kind": "dm", "key": persona_id }, "before": "u1" } }),
    )
    .await;
    let page = answered(&mut socket, 4).await;
    assert_eq!(page["result"]["events"][0]["kind"], "link");
    assert_eq!(page["result"]["events"][0]["state"], "parked");
    assert_eq!(page["result"]["more"], false);

    // The old name pages the same lines in the shape this socket reads.
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "tape.page", "params": { "personaId": persona_id, "before": "u1" } }),
    )
    .await;
    assert_eq!(
        answered(&mut socket, 5).await["result"],
        page["result"],
        "tape.page is thread.page on a DM"
    );

    // A second socket that never said so is still sent the marker.
    let mut old = desk(port).await;
    ask(&mut old, json!({ "id": 1, "sub": { "tape": persona_id } })).await;
    let snapshot = heard_where(&mut old, |frame| frame["snapshot"].is_array()).await;
    assert_eq!(snapshot["snapshot"][0]["kind"], "side");
    assert_eq!(snapshot["snapshot"][1]["kind"], "subagent");
}

/// A teammate's threads, of every kind, as one list: what each kind's own
/// record says, in one shape.
#[tokio::test]
async fn thread_list_reads_every_kind_as_one_summary() {
    let (_root, log, port) = door("thread-list");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let tape = StreamId::Tape(ada.clone());
    log.append(
        &tape,
        &json!({ "kind": "user", "id": "u1", "ts": 100, "text": "morning" }),
    )
    .unwrap();
    log.append(
        &tape,
        &json!({ "kind": "agent", "id": "a1", "ts": 110, "text": "Good morning." }),
    )
    .unwrap();

    let link = |kind: &str, key: &str, title: &str, state: &str, extra: Value| {
        let mut line = json!({
            "kind": "link", "id": format!("link:{kind}:{key}"), "ts": 200, "thread": key,
            "threadKind": kind, "personaId": ada, "title": title, "state": state,
        });
        for (name, value) in extra.as_object().unwrap() {
            line[name] = value.clone();
        }
        line
    };
    // A work thread a colleague opened, closed with an outcome.
    let side = link(
        "side",
        "s1",
        "Mend the crane",
        "closed",
        json!({ "end": "agent", "outcome": "It was the cache.", "at": 300,
                "openerId": bob, "openerName": "Bob" }),
    );
    log.append(&tape, &side).unwrap();
    log.append(&StreamId::Side("s1".into()), &side).unwrap();
    log.append(
        &StreamId::Side("s1".into()),
        &json!({ "kind": "agent", "id": "sa", "ts": 250, "text": "Checked the cache." }),
    )
    .unwrap();
    // A parked one, with a card waiting.
    let parked = link("side", "s2", "Sweep the docks", "parked", json!({}));
    log.append(&tape, &parked).unwrap();
    log.append(&StreamId::Side("s2".into()), &parked).unwrap();
    log.append(
        &StreamId::Side("s2".into()),
        &json!({ "kind": "permission", "id": "perm:r", "ts": 400, "requestId": "r",
                 "title": "run it", "options": [] }),
    )
    .unwrap();
    let run = link(
        "run",
        "r1",
        "Look it up",
        "closed",
        json!({ "end": "done" }),
    );
    log.append(&tape, &run).unwrap();
    log.append(&StreamId::Run("r1".into()), &run).unwrap();
    let call = link(
        "call",
        "c1",
        "Call",
        "closed",
        json!({ "end": "idle", "at": 260 }),
    );
    log.append(&tape, &call).unwrap();
    log.append(&StreamId::Call("c1".into()), &call).unwrap();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    log.append(
        &StreamId::Pair(pair.clone()),
        &json!({ "kind": "user", "id": "pu", "ts": 150, "text": "Got a minute?" }),
    )
    .unwrap();

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "thread.list", "params": { "personaId": ada } }),
    )
    .await;
    let listed = answered(&mut socket, 3).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let rows = listed["result"].as_array().unwrap();
    let find = |kind: &str, key: &str| {
        rows.iter()
            .find(|row| row["thread"] == json!({ "kind": kind, "key": key }))
            .unwrap_or_else(|| panic!("no {kind} {key} in {rows:?}"))
            .clone()
    };
    assert_eq!(rows.len(), 6, "{rows:?}");
    // Live first, then parked, then closed; newest first inside each.
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        ["live", "parked", "parked", "closed", "closed", "closed"],
        "{order:?}"
    );

    let dm = find("dm", &ada);
    assert_eq!(dm["personaId"], ada.as_str());
    assert_eq!(dm["preview"], "Good morning.");
    assert_eq!(dm["state"], "live");
    assert_eq!(dm["working"], false);

    let closed = find("side", "s1");
    assert_eq!(closed["title"], "Mend the crane");
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["end"], "agent");
    assert_eq!(closed["outcome"], "It was the cache.");
    assert_eq!(closed["preview"], "Checked the cache.");
    assert_eq!(closed["opener"], json!({ "personaId": bob, "name": "Bob" }));
    assert_eq!(closed["updatedAt"], 250);
    assert_eq!(closed["waiting"], false);

    let waiting = find("side", "s2");
    assert_eq!(waiting["state"], "parked");
    assert_eq!(waiting["waiting"], true);

    assert_eq!(find("run", "r1")["end"], "done");
    assert_eq!(find("call", "c1")["title"], "Call");
    let pair_row = find("pair", &pair);
    assert_eq!(pair_row["state"], "parked");
    assert_eq!(pair_row["preview"], "Got a minute?");
    assert!(
        [ada.as_str(), bob.as_str()].contains(&pair_row["personaId"].as_str().unwrap())
            && [ada.as_str(), bob.as_str()].contains(&pair_row["withPersonaId"].as_str().unwrap())
            && pair_row["personaId"] != pair_row["withPersonaId"],
        "{pair_row}"
    );
    for row in rows {
        serde_json::from_value::<crate::contract::ThreadSummary>(row.clone()).unwrap();
    }

    // Room-wide: Bob's DM joins, and the pair is still listed once.
    ask(
        &mut socket,
        json!({ "id": 4, "cmd": "thread.list", "params": {} }),
    )
    .await;
    let all = answered(&mut socket, 4).await["result"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(all.len(), 7);
    assert_eq!(
        all.iter()
            .filter(|row| row["thread"]["kind"] == "pair")
            .count(),
        1
    );

    // A teammate nobody holds has no threads.
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "thread.list", "params": { "personaId": "nobody" } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 5).await["result"], json!([]));
}

/// A companion is listed the threads it may read, which leaves out a call:
/// its summary's preview is what was spoken in it.
#[tokio::test]
async fn a_companions_thread_list_leaves_out_calls_and_pairs() {
    let (_root, log, port) = door("thread-list-companion");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let tape = StreamId::Tape(ada.clone());
    let link = |kind: &str, key: &str| {
        json!({
            "kind": "link", "id": format!("link:{kind}:{key}"), "ts": 200, "thread": key,
            "threadKind": kind, "personaId": ada, "title": key, "state": "closed",
            "end": "idle", "at": 300,
        })
    };
    for (kind, key, stream) in [
        ("side", "s1", StreamId::Side("s1".into())),
        ("call", "c1", StreamId::Call("c1".into())),
    ] {
        log.append(&tape, &link(kind, key)).unwrap();
        log.append(&stream, &link(kind, key)).unwrap();
        log.append(
            &stream,
            &json!({ "kind": "agent", "id": "x", "ts": 250, "text": format!("said in {key}") }),
        )
        .unwrap();
    }
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    log.append(
        &StreamId::Pair(pair.clone()),
        &json!({ "kind": "user", "id": "pu", "ts": 150, "text": "Got a minute?" }),
    )
    .unwrap();

    let handle: Arc<dyn RoomHandle> = Arc::new(Quiet::new());
    let kinds = |answer: &Value| -> Vec<String> {
        let mut kinds: Vec<String> = answer["result"]
            .as_array()
            .unwrap_or_else(|| panic!("{answer}"))
            .iter()
            .map(|row| row["thread"]["kind"].as_str().unwrap().to_string())
            .collect();
        kinds.sort();
        kinds
    };
    for params in [json!({ "personaId": ada }), json!({})] {
        let request = json!({ "id": 1, "cmd": "thread.list", "params": params });
        let phone = remote_control_answer(
            Seat::Phone,
            &handle,
            &log,
            &Parts::default(),
            request.clone(),
        )
        .await;
        assert!(
            !phone.to_string().contains("said in c1"),
            "a call's words are not a companion's to list: {phone}"
        );
        assert!(
            !kinds(&phone)
                .iter()
                .any(|kind| kind == "call" || kind == "pair")
        );
        assert!(kinds(&phone).iter().any(|kind| kind == "side"));
        let owner =
            remote_control_answer(Seat::Owner, &handle, &log, &Parts::default(), request).await;
        assert!(kinds(&owner).iter().any(|kind| kind == "call"));
        assert!(kinds(&owner).iter().any(|kind| kind == "pair"));
    }
}

#[tokio::test]
async fn a_thread_subscription_by_id_reads_the_stream_its_kind_keeps() {
    let (_root, log, port) = door("thread-id-sub");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();
    for (stream, line) in [
        (StreamId::Tape(ada.clone()), "dm"),
        (StreamId::Side("s1".into()), "side"),
        (StreamId::Run("r1".into()), "run"),
        (StreamId::Call("c1".into()), "call"),
        (StreamId::Pair(pair.clone()), "pair"),
    ] {
        log.append(
            &stream,
            &json!({ "kind": "user", "id": line, "ts": 1, "text": line }),
        )
        .unwrap();
    }
    for (n, (kind, key)) in [
        ("dm", ada.as_str()),
        ("side", "s1"),
        ("run", "r1"),
        ("call", "c1"),
        ("pair", pair.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 10 + n as i64;
        ask(
            &mut socket,
            json!({ "id": id, "sub": { "threadId": { "kind": kind, "key": key } } }),
        )
        .await;
        assert_eq!(heard(&mut socket).await, json!({ "id": id, "ok": true }));
        let snapshot = heard(&mut socket).await;
        assert_eq!(snapshot["snapshot"][0]["text"], kind, "{snapshot}");
    }
    // `{thread: key}` still means a pair, for the clients that know no other.
    ask(&mut socket, json!({ "id": 20, "sub": { "thread": pair } })).await;
    assert_eq!(heard(&mut socket).await, json!({ "id": 20, "ok": true }));
    assert_eq!(heard(&mut socket).await["snapshot"][0]["text"], "pair");
}

/// Each seat may do what it could do under the older names, and no more.
#[test]
fn the_phone_seat_runs_thread_commands_as_it_ran_the_old_ones() {
    let thread = |kind: ThreadKind| ThreadId::new(kind, "k");
    let prompt = |kind| Command::ThreadPrompt {
        thread: thread(kind),
        text: "x".to_string(),
        reply_to: None,
        attachments: None,
    };
    let answer = |kind| Command::ThreadAnswer {
        thread: thread(kind),
        answer: crate::contract::ThreadAnswer::Permission {
            request_id: "r".to_string(),
            option_id: "o".to_string(),
        },
    };
    // A phone speaks to a teammate through `mobile.prompt`, as before, and in
    // a work thread through this.
    assert!(Seat::Phone.permits(&prompt(ThreadKind::Side)));
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Pair,
        ThreadKind::Run,
        ThreadKind::Call,
        ThreadKind::Job,
    ] {
        assert!(!Seat::Phone.permits(&prompt(kind)), "{kind:?}");
    }
    // Closing a job cancels it, and stopping its firing is the same owner's
    // call: a phone may do neither, as it may not cancel a schedule.
    assert!(!Seat::Phone.permits(&Command::ThreadClose {
        thread: thread(ThreadKind::Job),
    }));
    assert!(!Seat::Phone.permits(&Command::ThreadCancel {
        thread: thread(ThreadKind::Job),
        interrupt: false,
    }));
    assert!(Seat::Desk.permits(&Command::ThreadCancel {
        thread: thread(ThreadKind::Job),
        interrupt: false,
    }));
    // A pair raises no card, so a companion answers none there.
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Side,
        ThreadKind::Run,
        ThreadKind::Call,
        ThreadKind::Job,
    ] {
        assert!(Seat::Phone.permits(&answer(kind)), "{kind:?}");
    }
    assert!(!Seat::Phone.permits(&answer(ThreadKind::Pair)));
    for command in [
        Command::ThreadList { persona_id: None },
        Command::ThreadOpen {
            persona_id: "ada".to_string(),
            text: "x".to_string(),
        },
        Command::ThreadCancel {
            thread: thread(ThreadKind::Side),
            interrupt: true,
        },
        Command::ThreadPark {
            thread: thread(ThreadKind::Side),
        },
        Command::ThreadClose {
            thread: thread(ThreadKind::Side),
        },
        Command::ThreadContinue {
            thread: thread(ThreadKind::Side),
        },
        Command::ClientHello {
            capabilities: Vec::new(),
        },
    ] {
        assert!(Seat::Phone.permits(&command), "{command:?}");
    }
    // The voice dispatcher's tape and a call are not the phone's to read.
    let page = |thread| Command::ThreadPage {
        thread,
        before: "b".to_string(),
        limit: None,
        through: None,
    };
    assert!(Seat::Phone.permits(&page(thread(ThreadKind::Pair))));
    assert!(!Seat::Phone.permits(&page(thread(ThreadKind::Call))));
    assert!(!Seat::Phone.permits(&page(ThreadId::dm("voice-dispatcher"))));
    for kind in [
        ThreadKind::Dm,
        ThreadKind::Side,
        ThreadKind::Pair,
        ThreadKind::Run,
        ThreadKind::Job,
    ] {
        assert!(
            Seat::Phone.permits_sub(&Target::ThreadId(ThreadId::new(kind, "k"))),
            "{kind:?}"
        );
    }
    assert!(!Seat::Phone.permits_sub(&Target::ThreadId(thread(ThreadKind::Call))));
    assert!(!Seat::Phone.permits_sub(&Target::ThreadId(ThreadId::dm("voice-dispatcher"))));
    // The desk and an owner may do all of it.
    for seat in [Seat::Desk, Seat::Owner] {
        assert!(seat.permits(&prompt(ThreadKind::Dm)));
        assert!(seat.permits(&answer(ThreadKind::Pair)));
        assert!(seat.permits_sub(&Target::ThreadId(thread(ThreadKind::Call))));
    }
}

/// The verbs a kind does have reach what the older command reached: a request
/// for the person is answered through the teammate, a pair's cancel and
/// continue are its exchange's stop and resume.
#[tokio::test]
async fn thread_verbs_reach_the_handlers_the_older_commands_did() {
    let (_root, log, port) = door("thread-verbs-reach");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob = create(&mut socket, 2, "Bob").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let pair = paths::thread_key(&ada, &bob).unwrap();
    crate::log::thread::ensure(log.root(), &pair).unwrap();

    let human =
        json!({ "answer": { "kind": "human", "actionId": "a", "status": "done", "note": "ok" } });
    for (id, cmd, params) in [
        (
            3,
            "thread.answer",
            json!({ "thread": { "kind": "dm", "key": ada }, "answer": human["answer"] }),
        ),
        (
            4,
            "thread.cancel",
            json!({ "thread": { "kind": "pair", "key": pair } }),
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], true, "{cmd}: {answer}");
        assert!(
            answer.get("result").is_none(),
            "{cmd} answers nothing: {answer}"
        );
    }
    ask(
        &mut socket,
        json!({ "id": 5, "cmd": "thread.continue", "params": { "thread": { "kind": "pair", "key": pair } } }),
    )
    .await;
    let resumed = answered(&mut socket, 5).await;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(
        resumed["result"]["thread"],
        json!({ "kind": "pair", "key": pair })
    );
    // A thread the room has no record of is refused, not answered as nothing.
    ask(
        &mut socket,
        json!({ "id": 6, "cmd": "thread.answer", "params": { "thread": { "kind": "side", "key": "ghost" }, "answer": human["answer"] } }),
    )
    .await;
    assert_eq!(answered(&mut socket, 6).await["ok"], false);
}

/// A verb a kind has no meaning for is refused in a sentence, never silently.
#[tokio::test]
async fn a_thread_verb_the_kind_has_no_meaning_for_is_refused() {
    let (_root, _log, port) = door("thread-verbs");
    let mut socket = desk(port).await;
    let mut id = 0;
    for (cmd, kind, params) in [
        ("thread.prompt", "pair", json!({ "text": "hi" })),
        ("thread.prompt", "run", json!({ "text": "hi" })),
        ("thread.prompt", "call", json!({ "text": "hi" })),
        ("thread.park", "dm", json!({})),
        ("thread.close", "dm", json!({})),
        ("thread.close", "run", json!({})),
        ("thread.continue", "dm", json!({})),
        ("thread.cancel", "run", json!({})),
        (
            "thread.answer",
            "run",
            json!({ "answer": { "kind": "permission", "requestId": "r", "optionId": "o" } }),
        ),
        (
            "thread.answer",
            "call",
            json!({ "answer": { "kind": "human", "actionId": "a", "status": "done" } }),
        ),
        (
            "thread.answer",
            "pair",
            json!({ "answer": { "kind": "human", "actionId": "a", "status": "done" } }),
        ),
    ] {
        id += 1;
        let mut params = params;
        params["thread"] = json!({ "kind": kind, "key": "k" });
        ask(
            &mut socket,
            json!({ "id": id, "cmd": cmd, "params": params }),
        )
        .await;
        let answer = answered(&mut socket, id).await;
        assert_eq!(answer["ok"], false, "{cmd} {kind}: {answer}");
        assert!(
            answer["error"].as_str().unwrap().ends_with('.'),
            "{cmd} {kind}: {answer}"
        );
    }
}
