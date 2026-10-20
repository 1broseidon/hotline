//! The tool sources stored under the room's `mcpServers` setting, read the way
//! the roster's settings fold reads them: an entry that cannot be used is
//! dropped, and a legacy header map keeps its shape and loses its values.
//! Pure over the setting's JSON, so it sits beside the fold rather than with
//! the MCP client that starts the servers.

use serde_json::{Map, Value, json};

/// The server list stored under `mcpServers`, with anything unusable
/// dropped. Settings are a file a person can edit.
pub fn normalize_servers(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(normalize_server)
        .collect()
}

fn normalize_server(value: &Value) -> Option<Value> {
    let candidate = value.as_object()?;
    let id = candidate
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_string);
    let name = candidate
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return None;
    }

    if candidate.get("type").and_then(Value::as_str) == Some("http") {
        let url = candidate
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if url.is_empty() {
            return None;
        }
        let legacy_headers = string_map(candidate.get("headers"));
        let auth = normalize_http_auth(candidate.get("auth"), legacy_headers);
        return Some(json!({ "id": id, "type": "http", "name": name, "url": url, "auth": auth }));
    }

    let command = candidate
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return None;
    }
    let args: Vec<&str> = candidate
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut server =
        json!({ "id": id, "type": "stdio", "name": name, "command": command, "args": args });
    // Keep env even when it is not an object of strings, so parse can
    // refuse the server rather than starting it without the variables.
    if let Some(env) = candidate.get("env") {
        server
            .as_object_mut()
            .expect("just built as an object")
            .insert("env".to_string(), env.clone());
    }
    if let Some(reference) = candidate.get("credentialRef") {
        server["credentialRef"] = reference.clone();
    }
    Some(server)
}

/// A legacy `headers` map (the old app kept header values in settings) keeps
/// its shape and loses its values: `Authorization` becomes bearer mode and
/// one other header becomes header mode, and the token has to be pasted
/// again in Settings, into the vault this time.
fn normalize_http_auth(value: Option<&Value>, legacy: Option<&Map<String, Value>>) -> Value {
    if let Some(legacy) = legacy {
        let mut names = legacy.keys().filter(|name| !name.is_empty());
        return match (names.next(), names.next()) {
            (Some(name), None) if !name.eq_ignore_ascii_case("authorization") => {
                json!({ "mode": "header", "name": name })
            }
            (Some(_), _) => json!({ "mode": "bearer" }),
            (None, _) => json!({ "mode": "none" }),
        };
    }
    let Some(candidate) = value.and_then(Value::as_object) else {
        return json!({ "mode": "none" });
    };
    match candidate.get("mode").and_then(Value::as_str) {
        Some("bearer") => json!({ "mode": "bearer" }),
        Some("header") => match candidate
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            Some(name) => json!({ "mode": "header", "name": name }),
            None => json!({ "mode": "none" }),
        },
        Some("oauth") => {
            let scopes: Vec<String> = unique_keep_first(
                candidate
                    .get("scopes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|scope| !scope.is_empty())
                    .map(str::to_string),
            );
            let resource = candidate
                .get("resource")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let raw_client = candidate.get("client").and_then(Value::as_object);
            let method = raw_client
                .and_then(|client| client.get("tokenEndpointAuthMethod"))
                .and_then(Value::as_str)
                .filter(|method| {
                    matches!(
                        *method,
                        "none" | "client_secret_basic" | "client_secret_post"
                    )
                });
            let client = raw_client
                .and_then(|client| client.get("clientId").and_then(Value::as_str))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|client_id| {
                    let mut object = Map::new();
                    object.insert("clientId".to_string(), json!(client_id));
                    if let Some(method) = method {
                        object.insert("tokenEndpointAuthMethod".to_string(), json!(method));
                    }
                    Value::Object(object)
                });
            let mut auth = Map::new();
            auth.insert("mode".to_string(), json!("oauth"));
            auth.insert("scopes".to_string(), json!(scopes));
            if let Some(resource) = resource {
                auth.insert("resource".to_string(), json!(resource));
            }
            if let Some(client) = client {
                auth.insert("client".to_string(), client);
            }
            Value::Object(auth)
        }
        _ => json!({ "mode": "none" }),
    }
}

fn string_map(value: Option<&Value>) -> Option<&Map<String, Value>> {
    let object = value?.as_object()?;
    object.values().all(Value::is_string).then_some(object)
}

fn unique_keep_first(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_edited_bad_entry_costs_that_entry_not_the_list() {
        let raw = json!([
            { "id": "no-name", "type": "stdio", "command": "echo" },
            { "id": "good", "type": "stdio", "name": "Good", "command": "run" },
            { "id": "no-url", "type": "http", "name": "Remote" },
        ]);
        let servers = normalize_servers(&raw);
        let ids: Vec<&str> = servers
            .iter()
            .map(|server| server["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["good"]);
    }

    #[test]
    fn a_legacy_header_bearing_server_normalises_with_no_header_key() {
        let raw = json!([{
            "id": "srv1",
            "type": "http",
            "name": "Legacy",
            "url": "https://example.test",
            "headers": { "Authorization": "Bearer secret" },
        }]);
        let server = &normalize_servers(&raw)[0];
        assert_eq!(
            *server,
            json!({
                "id": "srv1",
                "type": "http",
                "name": "Legacy",
                "url": "https://example.test",
                "auth": { "mode": "bearer" },
            })
        );
        assert!(server.get("headers").is_none());
    }

    #[test]
    fn a_legacy_header_map_keeps_its_shape_and_drops_its_values() {
        let one = Map::from_iter([("X-API-Key".to_string(), json!("secret"))]);
        assert_eq!(
            normalize_http_auth(None, Some(&one)),
            json!({ "mode": "header", "name": "X-API-Key" })
        );
        let auth = Map::from_iter([("Authorization".to_string(), json!("Bearer secret"))]);
        assert_eq!(
            normalize_http_auth(None, Some(&auth)),
            json!({ "mode": "bearer" })
        );
        let several = Map::from_iter([
            ("X-A".to_string(), json!("1")),
            ("X-B".to_string(), json!("2")),
        ]);
        assert_eq!(
            normalize_http_auth(None, Some(&several)),
            json!({ "mode": "bearer" })
        );
        assert_eq!(
            normalize_http_auth(Some(&json!({ "mode": "header" })), None),
            json!({ "mode": "none" })
        );
    }
}
