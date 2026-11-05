use super::*;

/// A room from before cookie asks opens and runs: its offers setting and
/// Never choices are ignored, a pending offer expires, a share stays
/// listed and removable, and a cookie ask saved before the restart can be
/// answered neither with cookies nor with a login, and expires.
#[tokio::test]
async fn old_offer_records_are_read_and_a_saved_ask_is_history_after_a_real_desk_restart() {
    use hotline_core::log::{Log, StreamId};
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    log.append(&StreamId::Room, &json!({"kind":"persona","id":"ada","name":"ada","cwd":root.path(),"goal":"Test","backendId":"hotline","mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
    log.append(&StreamId::Room, &json!({"id":"cookie-offer-never:ada","kind":"cookie_offer_never","ts":1,"teammate":"ada","sites":["porkbun.com"]})).unwrap();
    log.append(&StreamId::Room, &json!({"kind":"setting","id":"cookieOffers","ts":1,"value":{"enabled":false,"sites":["porkbun.com"]}})).unwrap();
    for (id, status) in [
        ("waiting", "pending"),
        ("accepted", "lent"),
        ("refused", "never"),
    ] {
        log.append(&StreamId::Tape("ada".into()), &json!({"kind":"cookie_offer","id":id,"ts":1,"teammate":"ada","site":"example.com","browserName":"Brave","profileName":"Default","login":"lent-brave-default","status":status})).unwrap();
    }
    log.append(&StreamId::Tape("ada".into()), &json!({"kind":"cookie_ask","id":"cookie-ask:before-restart","ts":1,"site":"github.com","origin":"https://github.com","reason":"To read PR #203.","status":"pending","sources":[{"browserName":"Brave","profileName":"Default"}],"name":"GITHUB_COM","expiresAt":i64::MAX})).unwrap();
    drop(log);
    let desk = common::open_desk(root.path()).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;

    let settings = client
        .call("settings.update", json!({"patch":{"linkPreviews":true}}))
        .await;
    assert_eq!(settings["result"]["linkPreviews"], true, "{settings}");
    assert!(
        settings["result"].get("cookieOffers").is_none(),
        "{settings}"
    );
    let offers = client
        .call("computer.cookies.offers", json!({"personaId":"ada"}))
        .await;
    let statuses: Vec<_> = offers["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|offer| offer["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["expired", "lent", "never"]);

    let used = client
        .call(
            "computer.cookies.answer",
            json!({"personaId":"ada","id":"cookie-ask:before-restart","answer":"use"}),
        )
        .await;
    assert_eq!(used["ok"], false);
    let signed = client.call("secrets.capture.answer", json!({"actionId":"cookie-ask:before-restart","name":"GITHUB_COM","values":{"username":"ask-private-user","password":"ask-private-password"},"ask":false})).await;
    assert_eq!(signed["ok"], false);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    let tape = log.load(&StreamId::Tape("ada".into()));
    assert!(
        tape.iter()
            .any(|event| event["id"] == "cookie-ask:before-restart" && event["status"] == "expired")
    );
    tokio::time::resume();
    assert_eq!(
        client.call("secrets.list", json!({})).await["result"],
        json!([])
    );
    assert!(
        !serde_json::to_string(&tape)
            .unwrap()
            .contains("ask-private")
    );
    serving.abort();
}
