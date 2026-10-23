//! The room's settings, as events on the room stream.

use super::{append, now};
use crate::contract::WebSearchProvider;
use crate::log::Log;
use crate::room;
use crate::wire::RoomHandle;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Turns one web search provider on or off at the desk, under the policy gate.
pub(super) async fn set_web_search_enabled(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    provider: WebSearchProvider,
    enabled: bool,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut disabled = crate::websearch::disabled_on_desk(&room::try_settings(log)?);
    if enabled {
        disabled.remove(&provider);
    } else {
        disabled.insert(provider);
    }
    let names: Vec<&str> = crate::websearch::ORDER
        .into_iter()
        .filter(|provider| disabled.contains(provider))
        .map(crate::websearch::id)
        .collect();
    let mut patch = Map::new();
    patch.insert("webSearch".into(), json!({ "disabled": names }));
    update_settings(log, patch)?;
    web_search_status(log, room)
}

/// `settings.update`: each value normalised, tool sources sealed before they
/// are written, and every session restarted when the tool sources change.
pub(super) async fn apply_update(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    mut patch: Map<String, Value>,
) -> Result<Value, String> {
    // The lead has rules a setting cannot check: it names a teammate on the
    // team, and a desk of one has that one.
    if patch.contains_key(room::TEAM_LEAD) {
        return Err("The lead is set with persona.lead.".to_string());
    }
    if patch.contains_key(room::AGENT_ACCOUNTS) {
        return Err("Agent accounts are changed with accounts.*.".to_string());
    }
    for (key, value) in &mut patch {
        if !value.is_null() {
            *value = room::normalize_setting(key, value)?;
        }
    }
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    if let Some(value) = patch.get_mut("mcpServers")
        && !value.is_null()
    {
        *value = room.protect_mcp_settings(value)?;
    }
    let servers = patch.contains_key("mcpServers");
    if servers {
        room.invalidate_all()?;
    }
    let updated = update_settings(log, patch)?;
    if servers {
        forget_removed_servers(log)?;
        // Replacing or deleting legacy sources also removes their
        // superseded plaintext values from the room's history.
        log.migrate_mcp_settings(|value| {
            room.protect_mcp_settings(value)
                .map_err(std::io::Error::other)
        })
        .map_err(|error| error.to_string())?;
        room.reattach_all().await?;
    }
    Ok(updated)
}

/// Each web search provider as the Tools pane lists it: on or off at the
/// desk, and whether a key is saved. Never a key.
pub(super) fn web_search_status(log: &Log, room: &Arc<dyn RoomHandle>) -> Result<Value, String> {
    let disabled = crate::websearch::disabled_on_desk(&room::try_settings(log)?);
    let keyed = room.websearch_keyed();
    Ok(json!(crate::contract::WebSearchStatus {
        providers: crate::websearch::ORDER
            .into_iter()
            .map(|provider| crate::contract::WebSearchProviderStatus {
                provider,
                name: crate::websearch::display_name(provider).to_string(),
                enabled: !disabled.contains(&provider),
                has_key: keyed.contains(&provider),
            })
            .collect(),
    }))
}

/// One event per key, and `null` clears a key rather than setting it to
/// nothing — which puts the room's default back, because a deleted setting is
/// not an override.
pub(super) fn update_settings(log: &Log, patch: Map<String, Value>) -> Result<Value, String> {
    for (key, value) in patch {
        // MCP entries are a settings boundary. Canonicalise them before the
        // room event is written so a caller cannot persist an OAuth client
        // secret alongside public server configuration.
        let value = if key == "mcpServers" && !value.is_null() {
            Value::Array(crate::mcp::normalize_servers(&value))
        } else {
            value
        };
        let event = if value.is_null() {
            json!({ "kind": "setting", "id": key, "deleted": true })
        } else {
            json!({ "kind": "setting", "id": key, "value": value })
        };
        append(log, &event)?;
    }
    let mut settings = room::settings(log);
    if let Some(servers) = settings.get_mut("mcpServers") {
        *servers = crate::mcp::public_servers(servers);
    }
    Ok(Value::Object(settings))
}

/// The ids of the room's tool sources, as settings hold them now.
fn server_ids(log: &Log) -> Vec<String> {
    room::settings(log)
        .get("mcpServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|server| server["id"].as_str().map(String::from))
        .collect()
}

/// A tool source removed from settings leaves every teammate's grant with
/// it. Removing it was the decision; a policy still naming it would only be
/// a warning that it is gone, on every teammate that ever had it. Each save
/// of the list settles every grant against it, so one left behind by an
/// older build goes too.
fn forget_removed_servers(log: &Log) -> Result<(), String> {
    let servers = server_ids(log);
    let _writes = room::persona_write_lock();
    for mut persona in room::roster(log) {
        let granted = persona.mcp_policy.server_ids.len();
        persona
            .mcp_policy
            .server_ids
            .retain(|id| servers.contains(id));
        if persona.mcp_policy.server_ids.len() != granted {
            persona.updated_at = now();
            room::append_persona(log, &persona)?;
        }
    }
    Ok(())
}
