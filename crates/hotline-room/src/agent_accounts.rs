//! The agent accounts stored under the room's `agentAccounts` setting, read
//! entry by entry the way `mcp_servers` reads its list: an entry that cannot
//! be used is dropped, and the rest stand. Settings are a file a person can
//! edit, and `normalize_setting` keeps a whole list whatever is wrong with
//! it, so the filter lives here instead. Pure over the setting's JSON.

use hotline_contract::AgentAccount;
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

const MAX_NAME_CHARS: usize = 64;

/// The accounts in the stored list, with every entry that cannot be used
/// dropped, and each of those only for its own fault.
///
/// An entry stays when it reads as an [`AgentAccount`], its id is a uuid in
/// its canonical spelling (the id names the account's folder, so a second
/// spelling of one id must not be a second account), its backend is named,
/// and its name is 1 to 64 characters once trimmed. Among those, the first
/// entry with an id wins, and then the first with a name on a backend,
/// whatever the case. A kept entry keeps its name trimmed. An entry dropped
/// for another fault claims neither its id nor its name.
pub fn normalize_accounts(value: &Value) -> Vec<AgentAccount> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut kept = Vec::new();
    for entry in value.as_array().into_iter().flatten() {
        let Ok(mut account) = serde_json::from_value::<AgentAccount>(entry.clone()) else {
            continue;
        };
        let name = account.name.trim().to_string();
        let usable = is_canonical_uuid(&account.id)
            && !account.backend_id.is_empty()
            && (1..=MAX_NAME_CHARS).contains(&name.chars().count());
        if !usable {
            continue;
        }
        let name_key = (account.backend_id.clone(), name.to_lowercase());
        if ids.contains(&account.id) || names.contains(&name_key) {
            continue;
        }
        ids.insert(account.id.clone());
        names.insert(name_key);
        account.name = name;
        kept.push(account);
    }
    kept
}

fn is_canonical_uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id)
}

#[cfg(test)]
mod tests;
