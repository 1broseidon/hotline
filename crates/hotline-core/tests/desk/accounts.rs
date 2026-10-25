//! More than one account of a provider, over the wire the window uses:
//! listed once each, chosen per teammate, and never stood in for.

use super::*;

async fn account(client: &mut Client, label: &str, secret: &str) -> String {
    let made = client
        .call(
            "credential.create",
            json!({"providerId":"anthropic", "label": label, "secret": secret}),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    made["result"]["id"].as_str().unwrap().to_string()
}

async fn models(client: &mut Client) -> Vec<Value> {
    let listed = client.call("models.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    listed["result"].as_array().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_list_their_models_apart_and_a_teammate_on_a_revoked_one_does_not_run() {
    let (root, port) = open("two-accounts").await;
    let mut client = Client::connect(port, TOKEN).await;
    let work = account(&mut client, "Work", "sk-ant-not-a-real-work-key").await;
    let one = models(&mut client).await;
    assert_ne!(one.len(), 0);
    assert!(
        one.iter()
            .all(|choice| !choice["id"].as_str().unwrap().contains('#')),
        "one account keeps plain ids"
    );
    assert!(
        one.iter()
            .all(|choice| choice["account"]["id"] == work.as_str())
    );
    assert!(
        one.iter()
            .all(|choice| choice["account"]["shared"] == false)
    );

    let personal = account(&mut client, "Personal", "sk-ant-not-a-real-personal-key").await;
    let two = models(&mut client).await;
    assert_eq!(two.len(), one.len() * 2);
    let personal_model = two
        .iter()
        .find(|choice| choice["account"]["id"] == personal.as_str())
        .expect("the second account lists models")["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        personal_model.starts_with(&format!("anthropic#{personal}/")),
        "{personal_model}"
    );
    for choice in &two {
        assert_eq!(choice["account"]["shared"], true, "{choice}");
        assert!(choice["account"].get("secret").is_none());
        assert!(
            !choice.to_string().contains("not-a-real"),
            "no secret in a choice: {choice}"
        );
    }

    // Renaming changes the name the picker shows and nothing else.
    let renamed = client
        .call(
            "credential.rename",
            json!({"id": personal, "label": "  Side projects "}),
        )
        .await;
    assert_eq!(renamed["ok"], true, "{renamed}");
    assert_eq!(renamed["result"]["label"], "Side projects");
    let relisted = models(&mut client).await;
    assert_eq!(relisted.len(), two.len());
    assert!(
        relisted
            .iter()
            .filter(|choice| choice["account"]["id"] == personal.as_str())
            .all(|choice| choice["account"]["label"] == "Side projects")
    );
    assert_eq!(
        relisted
            .iter()
            .map(|choice| &choice["id"])
            .collect::<Vec<_>>(),
        two.iter().map(|choice| &choice["id"]).collect::<Vec<_>>(),
        "a rename moves no teammate: every id is the same"
    );
    let blank = client
        .call("credential.rename", json!({"id": personal, "label": "   "}))
        .await;
    assert_eq!(blank["ok"], false, "{blank}");

    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let made = client
        .call(
            "persona.create",
            json!({"draft":{"name":"Huck", "goal":"Run on Personal.", "cwd":workspace.to_string_lossy()}}),
        )
        .await;
    let persona = made["result"]["id"].as_str().unwrap().to_string();
    let set = client
        .call(
            "session.set_model",
            json!({"personaId": persona, "modelId": personal_model}),
        )
        .await;
    assert_eq!(set["ok"], true, "{set}");
    // A plain id is still a model this desk can reach: the default account.
    let plain = hotline_core::models::base_id(&personal_model).into_owned();
    let other = client
        .call("persona.create", json!({"draft":{"name":"Goldie"}}))
        .await;
    let goldie = other["result"]["id"].as_str().unwrap().to_string();
    let plain_set = client
        .call(
            "session.set_model",
            json!({"personaId": goldie, "modelId": plain}),
        )
        .await;
    assert_eq!(plain_set["ok"], true, "{plain_set}");

    let revoked = client
        .call("credential.revoke", json!({"id": personal}))
        .await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    let tape = client.subscribe(json!({"tape": persona})).await;
    let started = client
        .call("session.start", json!({"personaId": persona}))
        .await;
    assert_eq!(started["ok"], true, "{started}");
    let sent = client
        .call(
            "session.prompt",
            json!({"personaId": persona, "text": "Are you there?"}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    let notice = client
        .next_where(Duration::from_secs(20), |frame| {
            is_sub(frame, tape, "event") && frame["event"]["kind"] == "notice"
        })
        .await;
    let text = notice["event"]["text"].as_str().unwrap();
    assert!(text.contains("no longer connected"), "{text}");
    assert!(text.contains(r#""kind":"account_gone""#), "{text}");
    assert!(
        text.contains(&format!(r#""model":"{plain}""#)),
        "the card names the plain model: {text}"
    );
    // Work is still connected, and the teammate did not move to it.
    let after = client.call("persona.get", json!({"id": persona})).await;
    if after["ok"] == true {
        assert_eq!(
            after["result"]["modelId"],
            personal_model.as_str(),
            "{after}"
        );
    }
    let _ = work;
}

/// An account of Codex, over the wire the window uses: added from
/// Settings › Providers with no secret, listed after its agent as one of
/// several, renamed like any account, never a model of Hotline Agent's, and
/// signed out with its folder when it is removed.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_own_account_is_a_backend_of_its_own_until_it_is_removed() {
    let (root, port) = open("agent-account").await;
    let mut client = Client::connect(port, TOKEN).await;
    let made = client
        .call(
            "credential.create",
            json!({"providerId": "codex-acp", "label": "Work", "secret": ""}),
        )
        .await;
    assert_eq!(made["ok"], true, "{made}");
    assert_eq!(made["result"]["credentialKind"], "agent", "{made}");
    let id = made["result"]["id"].as_str().unwrap().to_string();
    let folder = root.join("agent-accounts").join(&id);
    assert!(folder.is_dir());
    let backend = format!("codex-acp#{id}");

    let listed = |backends: &Value| -> Vec<String> {
        backends["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|one| one["id"].as_str().unwrap().to_string())
            .collect()
    };
    let backends = client.call("backends.list", json!({})).await;
    let ids = listed(&backends);
    let agent = ids.iter().position(|one| one == "codex-acp").unwrap();
    assert_eq!(ids[agent + 1], backend, "an account follows its agent");
    let account = &backends["result"][agent + 1];
    assert_eq!(account["account"]["label"], "Work", "{account}");
    assert_eq!(account["account"]["shared"], true, "{account}");
    assert_eq!(backends["result"][agent]["accounts"], true);
    assert!(
        models(&mut client).await.is_empty(),
        "no model of Hotline Agent's"
    );

    let renamed = client
        .call(
            "credential.rename",
            json!({"id": id, "label": "Side projects"}),
        )
        .await;
    assert_eq!(renamed["ok"], true, "{renamed}");
    let backends = client.call("backends.list", json!({})).await;
    assert!(
        backends["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|one| one["id"] == backend.as_str() && one["account"]["label"] == "Side projects"),
        "{backends}"
    );

    let refused = client
        .call(
            "credential.create",
            json!({"providerId": "cursor", "label": "Work", "secret": ""}),
        )
        .await;
    assert_eq!(
        refused["ok"], false,
        "only an agent with accounts takes one: {refused}"
    );

    let removed = client.call("credential.delete", json!({"id": id})).await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(!folder.exists(), "removing the account signs it out");
    let backends = client.call("backends.list", json!({})).await;
    assert!(!listed(&backends).contains(&backend));
}
