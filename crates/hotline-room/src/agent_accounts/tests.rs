use super::*;
use serde_json::json;

fn id(n: u8) -> String {
    format!("00000000-0000-4000-a000-{n:012}")
}

fn names(accounts: &[AgentAccount]) -> Vec<(String, String)> {
    accounts
        .iter()
        .map(|account| (account.id.clone(), account.name.clone()))
        .collect()
}

fn entry(id: &str, backend_id: &str, name: &str) -> Value {
    json!({ "id": id, "backendId": backend_id, "name": name, "createdAt": 1 })
}

#[test]
fn a_bad_account_entry_costs_only_itself() {
    let input = json!([
        entry(&id(1), "claude-acp", " Account 1 "),
        entry("not-a-uuid", "claude-acp", "Not a uuid"),
        entry(&id(2).to_uppercase(), "claude-acp", "Upper case id"),
        entry(&id(1), "claude-acp", "Other"),
        entry(&id(3), "", "No backend"),
        entry(&id(4), "claude-acp", "   "),
        entry(&id(5), "claude-acp", &"x".repeat(65)),
        entry(&id(6), "claude-acp", "ACCOUNT 1"),
        entry(&id(7), "codex-acp", "Account 1"),
        entry(&id(8), "claude-acp", &"x".repeat(64)),
        entry(&id(9), "claude-acp", "Not a uuid"),
        entry(&id(3), "claude-acp", "Three"),
        json!(42),
    ]);
    assert_eq!(
        names(&normalize_accounts(&input)),
        [
            (id(1), "Account 1".to_string()),
            (id(7), "Account 1".to_string()),
            (id(8), "x".repeat(64)),
            (id(9), "Not a uuid".to_string()),
            (id(3), "Three".to_string()),
        ]
    );
    assert!(normalize_accounts(&Value::Null).is_empty());
}

#[test]
fn an_account_name_is_measured_in_characters_not_bytes() {
    let accepted = json!([
        entry(&id(1), "claude-acp", "Conta ção"),
        entry(&id(2), "claude-acp", &"ç".repeat(64)),
        entry(&id(3), "claude-acp", &"🙂".repeat(64)),
    ]);
    assert_eq!(normalize_accounts(&accepted).len(), 3);

    let too_long = json!([
        entry(&id(1), "claude-acp", &"ç".repeat(65)),
        entry(&id(2), "claude-acp", &"🙂".repeat(65)),
    ]);
    assert!(normalize_accounts(&too_long).is_empty());

    let duplicates = json!([
        entry(&id(1), "claude-acp", "Conta Teste ção"),
        entry(&id(2), "claude-acp", "conta teste ÇÃO"),
        entry(&id(3), "codex-acp", "conta teste ÇÃO"),
    ]);
    assert_eq!(
        names(&normalize_accounts(&duplicates)),
        [
            (id(1), "Conta Teste ção".to_string()),
            (id(3), "conta teste ÇÃO".to_string()),
        ]
    );
}
