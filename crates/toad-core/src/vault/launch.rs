//! MCP launch arguments and environment values never become room events.
use super::*;
use crate::mcp::{McpServer, McpTransport};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct Launch {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
}

impl Vault {
    fn launch_file(&self, id: &str, command: &str, reference: &str) -> io::Result<CredentialFile> {
        let reference = uuid::Uuid::parse_str(reference)
            .map_err(|_| io::Error::other("Invalid MCP credential reference."))?;
        let command = format!("{:x}", Sha256::digest(command.as_bytes()));
        Ok(self
            .files
            .file(self.directory().join("launch").join(format!(
                "{}-{command}-{reference}.json",
                mcp_path_component(id)
            ))))
    }

    pub(crate) fn protect_mcp_settings(&self, value: &Value) -> io::Result<Value> {
        if value.as_array().is_some_and(|servers| {
            servers
                .iter()
                .any(|server| server["urlNeedsRepair"] == true)
        }) {
            return Err(io::Error::other(
                "Re-enter the tool-source URL without credentials, a query, or a fragment. Use the authentication fields for secrets.",
            ));
        }
        if value.as_array().is_some_and(|servers| {
            servers
                .iter()
                .any(|server| server["launchValuesPending"] == true)
        }) {
            return Err(io::Error::other(
                "Some launch values still need credential migration. Unlock the OS credential store and retry the source, or replace its complete launch values.",
            ));
        }
        let mut servers = crate::mcp::normalize_servers(value);
        for server in &mut servers {
            if server["type"] == "http" {
                crate::mcp::validate_http_url(server["url"].as_str().unwrap_or_default())?;
                continue;
            }
            let id = server["id"]
                .as_str()
                .ok_or_else(|| io::Error::other("An MCP server needs an id."))?;
            let command = server["command"].as_str().unwrap_or_default();
            let args: Vec<String> = serde_json::from_value(server["args"].clone())
                .map_err(|_| io::Error::other("MCP arguments must be strings."))?;
            let env: HashMap<String, String> = match server.get("env") {
                Some(value) => serde_json::from_value(value.clone())
                    .map_err(|_| io::Error::other("MCP environment values must be strings."))?,
                None => HashMap::new(),
            };
            if let Some(reference) = server.get("credentialRef") {
                if !args.is_empty() || !env.is_empty() {
                    return Err(io::Error::other(
                        "Replace saved MCP launch values by submitting new arguments and environment without the credential reference.",
                    ));
                }
                let reference = reference
                    .as_str()
                    .ok_or_else(|| io::Error::other("Invalid MCP credential reference."))?;
                let file = self.launch_file(id, command, reference)?;
                crate::credentials::check_private_path(file.path()).map_err(|_| io::Error::other("The saved launch values do not belong to this command and server. Enter them again."))?;
                continue;
            }
            if args.is_empty() && env.is_empty() {
                continue;
            }
            let reference = uuid::Uuid::new_v4().to_string();
            let file = self.launch_file(id, command, &reference)?;
            make_private_directory(file.path().parent().expect("launch path has a parent"))?;
            file.write(
                &serde_json::to_vec(&Launch {
                    command: command.into(),
                    args,
                    env,
                })
                .map_err(io::Error::other)?,
            )?;
            server["credentialRef"] = json!(reference);
            server["args"] = json!([]);
            server
                .as_object_mut()
                .expect("normalized server")
                .remove("env");
        }
        Ok(Value::Array(servers))
    }

    pub(crate) fn migrate_mcp_settings(&self) -> io::Result<()> {
        self.log
            .migrate_mcp_settings(|value| self.protect_mcp_settings(value))
    }

    pub(crate) fn resolve_mcp_server(&self, server: &McpServer) -> io::Result<McpServer> {
        if let McpTransport::Http { url, .. } = &server.transport {
            crate::mcp::validate_http_url(url)?;
        }
        let McpTransport::Stdio { command, .. } = &server.transport else {
            return Ok(server.clone());
        };
        self.migrate_mcp_settings()?;
        let settings = crate::room::settings(&self.log);
        let saved = settings
            .get("mcpServers")
            .and_then(Value::as_array)
            .and_then(|servers| servers.iter().find(|saved| saved["id"] == server.id));
        let Some(saved) = saved else {
            return Ok(server.clone());
        };
        if saved["command"] != *command {
            return Err(io::Error::other(
                "The MCP command changed; reconnect this tool source.",
            ));
        }
        let Some(reference) = saved.get("credentialRef").and_then(Value::as_str) else {
            let mut resolved = server.clone();
            resolved.transport = McpTransport::Stdio {
                command: command.clone(),
                args: Vec::new(),
                env: HashMap::new(),
            };
            return Ok(resolved);
        };
        let bytes = self
            .launch_file(&server.id, command, reference)?
            .read()?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "The MCP launch credentials are missing. Enter them again.",
                )
            })?;
        let launch: Launch = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::other("The MCP launch credential record is unreadable."))?;
        if launch.command != *command {
            return Err(io::Error::other(
                "MCP launch credentials belong to a different command.",
            ));
        }
        let mut resolved = server.clone();
        resolved.transport = McpTransport::Stdio {
            command: command.clone(),
            args: launch.args,
            env: launch.env,
        };
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::tests::MemoryStore;

    #[test]
    fn legacy_urls_with_credentials_are_refused_before_connecting() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        let vault =
            Vault::open_with_store(root.path(), log, Arc::new(MemoryStore::default())).unwrap();
        for url in [
            "https://user:secret@example.com/mcp",
            "https://example.com/mcp?token=secret",
            "https://example.com/mcp#secret",
        ] {
            let value = json!([{"id":"remote", "name":"Remote", "type":"http", "url":url}]);
            assert!(vault.protect_mcp_settings(&value).is_err());
            let settings = serde_json::from_value(json!({"mcpServers":value})).unwrap();
            let server = crate::mcp::servers(&settings).remove(0);
            assert!(server.refuse.is_some());
            assert!(vault.resolve_mcp_server(&server).is_err());
        }
    }

    #[test]
    fn migrating_launch_values_removes_superseded_secrets_and_preserves_resolution() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        for token in ["superseded-secret", "current-secret"] {
            log.append(
                &StreamId::Room,
                &json!({"kind":"setting", "id":"mcpServers", "value":[{
                    "id":"example", "name":"Example", "type":"stdio", "command":"example-server",
                    "args":["--key", token], "env":{"API_KEY":token}
                }]}),
            )
            .unwrap();
        }
        let vault =
            Vault::open_with_store(root.path(), log.clone(), Arc::new(MemoryStore::default()))
                .unwrap();
        let disk = fs::read_to_string(root.path().join("room.jsonl")).unwrap();
        assert!(!disk.contains("superseded-secret"));
        assert!(!disk.contains("current-secret"));
        assert!(disk.contains("credentialRef"));
        let server = crate::mcp::servers(&crate::room::settings(&log)).remove(0);
        let resolved = vault.resolve_mcp_server(&server).unwrap();
        let McpTransport::Stdio { args, env, .. } = resolved.transport else {
            panic!("stdio")
        };
        assert_eq!(args, ["--key", "current-secret"]);
        assert_eq!(env["API_KEY"], "current-secret");
        let renamed = vault.protect_mcp_settings(&json!([{
            "id":"example", "name":"Renamed", "type":"stdio", "command":"example-server", "args":[],
            "credentialRef": crate::room::settings(&log)["mcpServers"][0]["credentialRef"]
        }])).unwrap();
        assert_eq!(renamed[0]["name"], "Renamed");
        let mut swapped = renamed;
        swapped[0]["command"] = json!("another-server");
        assert!(vault.protect_mcp_settings(&swapped).is_err());
    }

    struct Locked;
    impl SecretStore for Locked {
        fn get(&self, _: &str) -> io::Result<Option<Vec<u8>>> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
        }
        fn set(&self, _: &str, _: &[u8]) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
        }
        fn delete(&self, _: &str) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
        }
    }

    #[test]
    fn failed_migration_keeps_the_original_room_and_refuses_plaintext_launch() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::open(root.path());
        log.append(
            &StreamId::Room,
            &json!({"kind":"setting", "id":"mcpServers", "value":[{
                "id":"example", "name":"Example", "type":"stdio", "command":"example-server",
                "args":["--key", "keep-until-migrated"]
            }]}),
        )
        .unwrap();
        let before = fs::read(root.path().join("room.jsonl")).unwrap();
        let vault = Vault::open_with_store(root.path(), log.clone(), Arc::new(Locked)).unwrap();
        assert_eq!(before, fs::read(root.path().join("room.jsonl")).unwrap());
        let server = crate::mcp::servers(&crate::room::settings(&log)).remove(0);
        assert_eq!(
            vault.resolve_mcp_server(&server).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
