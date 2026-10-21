//! The reader returns the stored fold without snippets or lost tool updates.
use super::*;

fn page(text: &str) -> Value {
    let start = text.find('{').unwrap();
    let end = text.rfind('}').unwrap();
    serde_json::from_str(&text[start..=end]).unwrap()
}

#[tokio::test]
async fn thread_pages_keep_full_text_order_and_one_line_per_tool() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    enrol(&log, &persona("ada"));
    enrol(&log, &persona("bea"));
    let stream = StreamId::Call("c1".into());
    log.append(&stream, &json!({"kind":"link","id":"link:call:c1","ts":1,"thread":"c1","threadKind":"call","personaId":"ada","title":"Mascot","state":"parked"})).unwrap();
    let whole = "A long complete line. ".repeat(1000);
    for event in [
        json!({"kind":"user","id":"u1","ts":2,"text":whole}),
        json!({"kind":"tool","id":"t1","ts":2,"toolCallId":"tc1","title":"Read mascot.txt","toolKind":"read_file","status":"pending"}),
        json!({"kind":"agent","id":"a1","ts":2,"text":"Written plan","spoken":"Bramble, a wiry terrier"}),
        json!({"kind":"tool","id":"t1","ts":2,"toolCallId":"tc1","title":"Read mascot.txt","toolKind":"read_file","status":"completed","output":[{"type":"text","text":"Found the plan"}]}),
    ] {
        log.append(&stream, &event).unwrap();
    }
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
        crate::computer::Computer::with_path(root.path().join("no-runtime")),
    );
    let tools = TeammateTools::new(&room, "ada").for_run();
    let mut lines = Vec::new();
    let mut args = json!({"thread":"call:c1","limit":2});
    loop {
        let page = page(&tools.call("read_thread", &args).await.unwrap());
        lines.extend(page["lines"].as_array().unwrap().clone());
        if page["next_after"].is_null() {
            break;
        }
        args["after"] = page["next_after"].clone();
    }
    assert_eq!(
        lines
            .iter()
            .map(|line| line["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["link:call:c1", "u1", "t1", "a1"]
    );
    assert_eq!(lines[1]["text"], whole);
    assert_eq!(lines[1]["speaker"], "person");
    assert_eq!(lines[2]["status"], "completed");
    assert_eq!(lines[3]["spoken"], "Bramble, a wiry terrier");
    assert_eq!(lines[3]["speaker"], "ada");
    assert!(
        TeammateTools::new(&room, "bea")
            .call("read_thread", &args)
            .await
            .is_err()
    );
    for invalid in [
        json!({"thread":"call:c1","limit":0}),
        json!({"thread":"call:c1","limit":101}),
        json!({"thread":"call:c1","after":42}),
        json!({"thread":"missing"}),
        json!({"thread":"call:../../c1"}),
    ] {
        assert!(
            tools.call("read_thread", &invalid).await.is_err(),
            "{invalid}"
        );
    }
}
