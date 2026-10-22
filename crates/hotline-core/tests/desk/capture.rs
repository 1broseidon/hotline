use super::*;

/// A saved ask is history, not permission to write the vault after its lease died.
#[tokio::test]
async fn capture_restart_refuses_saved_cards_and_never_stores_submitted_values() {
    use hotline_core::log::{Log, StreamId};
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    // Seed before the core owns these streams.
    log.append(&StreamId::Room,&json!({"kind":"persona","id":"pork-chop","name":"Pork Chop","cwd":root.path(),"goal":"Test","backendId":"hotline","mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
    log.append(&StreamId::Tape("pork-chop".into()),&json!({"kind":"credential_ask","id":"credential:before-restart","actionId":"credential:before-restart","ts":1,"site":"https://github.com","reason":"Sign in","name":"GITHUB_LOGIN","credentialKind":"login","status":"pending","expiresAt":1,"existing":[],"armed":false})).unwrap();
    assert_eq!(hotline_core::room::roster(&log).len(), 1);
    drop(log);
    let desk = common::open_desk(root.path()).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let serving = tokio::spawn(door.run());
    let mut client = Client::connect(port, TOKEN).await;
    let answer=client.call("secrets.capture.answer",json!({"actionId":"credential:before-restart","name":"GITHUB_LOGIN","values":{"username":"capture-private-user","password":"capture-private-password"},"ask":false})).await;
    assert_eq!(answer["ok"], false);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    let saved = log.load(&StreamId::Tape("pork-chop".into()));
    assert!(saved.iter().any(|event| event["id"] == "credential:before-restart" && event["status"] == "expired"));
    tokio::time::resume();
    let listed = client.call("secrets.list", json!({})).await;
    assert_eq!(listed["result"], json!([]));
    for sink in [
        answer.to_string(),
        serde_json::to_string(&log.load(&StreamId::Tape("pork-chop".into()))).unwrap(),
    ] {
        assert!(!sink.contains("capture-private-user"));
        assert!(!sink.contains("capture-private-password"));
    }
    serving.abort();
}
