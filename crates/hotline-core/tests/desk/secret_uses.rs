use super::*;

#[tokio::test]
async fn secret_use_policy_survives_a_real_desk_restart_and_saved_cards_cannot_answer() {
    use hotline_core::log::{Log, StreamId};
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    log.append(&StreamId::Room, &json!({"kind":"persona","id":"ada","name":"Ada","cwd":root.path(),"goal":"Test","backendId":"hotline","mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
    log.append(&StreamId::Tape("ada".into()), &json!({"kind":"secret_use_ask","id":"saved-use","useId":"old-id","ts":1,"askedAt":1,"expiresAt":9_999_999_999_999_i64,"secret":"GH_PASSKEY","secretKind":"passkey","site":"github.com","path":"/login","teammate":"ada","status":"pending"})).unwrap();
    assert_eq!(hotline_core::room::roster(&log).len(), 1);
    drop(log);
    let desk = common::open_desk(root.path()).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    assert_eq!(client.call("secrets.login.set", json!({"name":"HN_LOGIN","sites":["https://news.ycombinator.com"],"username":"person","password":"private-password"})).await["ok"], true);
    assert_eq!(
        client
            .call("secrets.use.set", json!({"name":"HN_LOGIN","use":"ask"}))
            .await["result"]["use"],
        "ask"
    );
    assert_eq!(
        client
            .call(
                "secrets.use.answer",
                json!({"personaId":"ada","useId":"old-id","answer":"approve"})
            )
            .await["ok"],
        false
    );
    assert_eq!(
        log.load(&StreamId::Tape("ada".into()))[0]["status"],
        "expired"
    );
    drop(client);
    serving.abort();
    let _ = serving.await;
    let reopened = common::open_desk(root.path()).unwrap();
    let parts = reopened.parts();
    let door = Door::bind(
        reopened.log.clone(),
        TOKEN.into(),
        Arc::new(reopened),
        parts,
    )
    .unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    assert_eq!(
        client.call("secrets.list", json!({})).await["result"][0]["use"],
        "ask"
    );
    assert_eq!(
        client
            .call("secrets.use.set", json!({"name":"HN_LOGIN","use":"auto"}))
            .await["result"]["use"],
        "auto"
    );
    serving.abort();
}
