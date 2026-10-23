use super::*;
use tokio::sync::mpsc::error::TryRecvError;

async fn asked(requests: &mut mpsc::Receiver<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(15), requests.recv())
        .await
        .expect("the server was never asked")
        .unwrap()
}

fn forget_what_was_asked(requests: &mut mpsc::Receiver<Value>) {
    while requests.try_recv().is_ok() {}
}

fn failure_in(notice: &str) -> Value {
    let at = notice
        .find(r#"{"hotlineFailure":"#)
        .unwrap_or_else(|| panic!("the notice carries no diagnosis: {notice}"));
    let diagnosis: Value = serde_json::from_str(&notice[at..]).unwrap();
    diagnosis["hotlineFailure"].clone()
}

fn never_asked(server: &str, requests: &mut mpsc::Receiver<Value>) {
    assert_eq!(
        requests.try_recv(),
        Err(TryRecvError::Empty),
        "server {server} was asked"
    );
}

/// Two servers of one provider are two accounts. A teammate set to one is
/// answered by it and every other teammate by the provider's default; when
/// the chosen account is revoked the teammate's turn fails naming it, and the
/// default is never asked in its place.
#[tokio::test(flavor = "multi_thread")]
async fn a_teammate_runs_on_its_chosen_account_and_others_on_the_default() {
    let (one_url, mut one, one_server) = fake_ollama("one").await;
    let (two_url, mut two, two_server) = fake_ollama("two").await;
    let (root, port) = open("accounts").await;
    let mut client = Client::connect(port, TOKEN).await;

    let first = client
        .call("credential.connect_local", json!({"baseUrl": one_url}))
        .await;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["result"]["label"], "Ollama Local");
    let second = client
        .call("credential.connect_local", json!({"baseUrl": two_url}))
        .await;
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(second["result"]["label"], "Ollama Local 2");
    let one_id = first["result"]["id"].as_str().unwrap().to_string();
    let two_id = second["result"]["id"].as_str().unwrap().to_string();

    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let model = format!("ollama/{FAKE_OLLAMA_MODEL}");
    let ada = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Ada", "goal":"Answer plainly.", "cwd":workspace.to_string_lossy(),
                "modelId":model, "accountId":two_id}}),
        )
        .await;
    assert_eq!(ada["ok"], true, "{ada}");
    assert_eq!(ada["result"]["accountId"], json!(two_id));
    let mack = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Mack", "goal":"Answer plainly.", "cwd":workspace.to_string_lossy(),
                "modelId":model}}),
        )
        .await;
    assert_eq!(mack["ok"], true, "{mack}");
    assert!(mack["result"]["accountId"].is_null(), "{mack}");
    let ada_id = ada["result"]["id"].as_str().unwrap().to_string();
    let mack_id = mack["result"]["id"].as_str().unwrap().to_string();
    let ada_tape = client.subscribe(json!({"tape": ada_id})).await;
    let mack_tape = client.subscribe(json!({"tape": mack_id})).await;
    for persona in [&ada_id, &mack_id] {
        let started = client
            .call("session.start", json!({"personaId": persona}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
    }

    let sent = client
        .call(
            "session.prompt",
            json!({"personaId": ada_id, "text": "Is the crane free?"}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(asked(&mut two).await["model"], FAKE_OLLAMA_MODEL);
    let said = said_this_turn(&mut client, ada_tape, Duration::from_secs(15)).await;
    assert!(said.contains("Hello from two."), "{said}");
    never_asked("one", &mut one);

    let sent = client
        .call(
            "session.prompt",
            json!({"personaId": mack_id, "text": "Is the crane free?"}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(asked(&mut one).await["model"], FAKE_OLLAMA_MODEL);
    let said = said_this_turn(&mut client, mack_tape, Duration::from_secs(15)).await;
    assert!(said.contains("Hello from one."), "{said}");
    never_asked("two", &mut two);

    let refused = client
        .call("credential.delete", json!({"id": two_id}))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"],
        "Ollama Local 2 can't be removed while Ada uses it."
    );
    let kept = client.call("credential.list", json!({})).await;
    assert!(
        kept["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == json!(two_id) && row["revoked"] == false),
        "a refused removal revokes nothing: {kept}"
    );

    let renamed = client
        .call("credential.update", json!({"id": two_id, "label": "Spare"}))
        .await;
    assert_eq!(renamed["ok"], true, "{renamed}");
    assert_eq!(renamed["result"]["id"], json!(two_id));
    assert_eq!(renamed["result"]["label"], "Spare");
    let listed = client.call("accounts.list", json!({})).await;
    let rows = listed["result"]["credentials"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{listed}");
    let row = |id: &str| rows.iter().find(|row| row["credential"]["id"] == id);
    let (on_one, on_two) = (row(&one_id).unwrap(), row(&two_id).unwrap());
    assert_eq!(on_one["default"], true, "{listed}");
    assert_eq!(on_one["usedBy"], json!([mack_id]), "{listed}");
    assert_eq!(on_two["default"], false, "{listed}");
    assert_eq!(on_two["usedBy"], json!([ada_id]), "{listed}");
    assert_eq!(on_two["credential"]["label"], "Spare", "{listed}");

    let revoked = client
        .call("credential.revoke", json!({"id": two_id}))
        .await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    forget_what_was_asked(&mut one);
    forget_what_was_asked(&mut two);
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId": ada_id, "text": "And now?"}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let notice = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, ada_tape, "event")
                && frame["event"]["kind"] == "notice"
                && frame["event"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(r#"{"hotlineFailure":"#))
        })
        .await;
    let failure = failure_in(notice["event"]["text"].as_str().unwrap());
    assert_eq!(failure["kind"], "auth", "{failure}");
    assert_eq!(failure["accountId"], json!(two_id), "{failure}");
    let turn = client
        .next_where(Duration::from_secs(15), |frame| {
            is_sub(frame, ada_tape, "event") && frame["event"]["kind"] == "turn"
        })
        .await;
    assert_eq!(turn["event"]["stopReason"], "failed", "{turn}");
    never_asked("one", &mut one);
    never_asked("two", &mut two);

    for persona in [&ada_id, &mack_id] {
        client
            .call("session.stop", json!({"personaId": persona}))
            .await;
    }
    one_server.abort();
    two_server.abort();
}
