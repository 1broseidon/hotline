use super::*;
use crate::contract::AccountStatusState;
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;

/// The canonical `<scratch>/Conta Teste ção`: spaces and a non-ASCII letter,
/// as a profile name can have. It exists, because the parser resolves both
/// sides of the comparison.
fn folder() -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Conta Teste ção");
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dunce::canonicalize(dir).unwrap();
    (tmp, dir)
}

/// The agent's answer as bytes. Built from `json!`, so a Windows path keeps
/// its backslashes escaped, as the real answer does.
fn said(answer: &Value) -> Vec<u8> {
    answer.to_string().into_bytes()
}

/// A subscription login in `dir`, as the agent words it.
fn signed_in_at(dir: &Path) -> Value {
    json!({
        "loggedIn": true,
        "authMethod": "claude.ai",
        "apiProvider": "firstParty",
        "email": "ada@example.com",
        "orgId": "6f0a3c1e-2b7d-4e55-9a10-3d1c8b5e7f42",
        "orgName": "Ada's Org",
        "subscriptionType": "max",
        "analyticsDisabled": false,
        "configDirectory": dir.to_str().unwrap(),
    })
}

fn view(state: AccountStatusState, email: Option<&str>, plan: Option<&str>) -> AccountStatusView {
    AccountStatusView {
        state,
        checked_at: None,
        email: email.map(str::to_string),
        plan: plan.map(str::to_string),
    }
}

#[test]
fn signed_in_needs_a_first_party_claude_ai_login_in_this_folder() {
    let (tmp, dir) = folder();
    let signed_in = view(
        AccountStatusState::SignedIn,
        Some("ada@example.com"),
        Some("max"),
    );
    assert_eq!(
        parse_claude_status(&said(&signed_in_at(&dir)), &dir),
        signed_in
    );

    // The folder spelled another way, a null key source and the newline the
    // CLI ends with are still that login.
    let respelled = dir.join("..").join("Conta Teste ção");
    assert_eq!(
        parse_claude_status(&said(&signed_in_at(&respelled)), &dir),
        signed_in
    );
    let mut null_source = signed_in_at(&dir);
    null_source["apiKeySource"] = Value::Null;
    assert_eq!(parse_claude_status(&said(&null_source), &dir), signed_in);
    let mut with_newline = said(&signed_in_at(&dir));
    with_newline.push(b'\n');
    assert_eq!(parse_claude_status(&with_newline, &dir), signed_in);
    drop(tmp);
}

#[test]
fn logged_out_is_signed_out_whatever_else_it_says() {
    let (_tmp, dir) = folder();
    let signed_out = view(AccountStatusState::SignedOut, None, None);
    let answer = json!({
        "loggedIn": false,
        "authMethod": "none",
        "apiProvider": "firstParty",
        "configDirectory": dir.to_str().unwrap(),
    });
    assert_eq!(parse_claude_status(&said(&answer), &dir), signed_out);
    assert_eq!(
        parse_claude_status(&said(&json!({ "loggedIn": false })), &dir),
        signed_out
    );
}

#[test]
fn any_other_answer_is_unavailable_and_says_nothing_more() {
    let (tmp, dir) = folder();
    let another = tmp.path().join("Outra Conta");
    std::fs::create_dir_all(&another).unwrap();
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    let mut with = |label, change: &dyn Fn(&mut Value)| {
        let mut answer = signed_in_at(&dir);
        change(&mut answer);
        cases.push((label, said(&answer)));
    };
    with("an API key", &|answer| {
        answer["authMethod"] = json!("api_key");
        answer["apiKeySource"] = json!("ANTHROPIC_API_KEY");
    });
    with("an OAuth token", &|answer| {
        answer["authMethod"] = json!("oauth_token");
    });
    with("a third party", &|answer| {
        answer["authMethod"] = json!("third_party");
        answer["apiProvider"] = json!("bedrock");
    });
    // The account's own settings replacing the login (spec §7.6).
    with("a key helper", &|answer| {
        answer["apiKeySource"] = json!("apiKeyHelper");
    });
    with("another folder", &|answer| {
        answer["configDirectory"] = json!(another.to_str().unwrap());
    });
    with("no folder", &|answer| {
        answer.as_object_mut().unwrap().remove("configDirectory");
    });
    with("a logged-in string", &|answer| {
        answer["loggedIn"] = json!("true");
    });
    cases.push(("nothing", Vec::new()));
    cases.push(("not json", b"not json".to_vec()));

    for (label, stdout) in cases {
        assert_eq!(
            parse_claude_status(&stdout, &dir),
            view(AccountStatusState::Unavailable, None, None),
            "{label}"
        );
    }
}
