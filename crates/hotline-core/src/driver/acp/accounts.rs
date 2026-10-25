//! An agent signed in as one of the person's accounts rather than as it is on
//! this computer: Claude Code or Codex with its sign-in, settings and sessions
//! kept in a folder Hotline gives it, so a desk can hold several.
//!
//! An account is a credential like a provider's, with the agent's id as its
//! provider and no secret: the agent signs itself in, in its own folder, the
//! first time a teammate runs on it. A teammate names it in its backend id,
//! `claude-acp#<credential id>`, the way a model names a provider's account
//! (`anthropic#<credential id>/model`); the plain `claude-acp` is the agent as
//! the person signed in to it on this computer. Everything Hotline keeps per
//! agent — the models it reported, whether it asked to be signed in, the
//! sessions a teammate can reopen — is kept by that id, so each account has
//! its own without a second table.

use super::registry::{self, Launch};
use crate::contract::{Credential, CredentialKind};
use crate::log::{Log, StreamId};
use crate::vault::Vault;
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What running one agent under an account takes.
struct Support {
    agent: &'static str,
    /// The variable that points the agent at its folder.
    folder_var: &'static str,
    /// Every variable the agent would sign in with instead of its folder, so
    /// an account's process never inherits one. It ends with `folder_var`,
    /// which the account's folder takes over.
    scrub: &'static [&'static str],
    /// The person's own setup, under their home, when `folder_var` names none.
    home: &'static str,
    /// What a new account starts with from that setup: instructions, skills,
    /// settings and servers. Never a sign-in.
    setup: &'static [&'static str],
    /// Beside `home` in the home, and inside the folder `folder_var` names: a
    /// file of which only the MCP servers are copied, since the rest is the
    /// sign-in's own state.
    servers_file: Option<&'static str>,
    /// The agent signs in only in a terminal, which Hotline runs on Unix
    /// alone: elsewhere an account of it could never be signed in.
    terminal_sign_in: bool,
}

const SUPPORT: &[Support] = &[
    Support {
        agent: "claude-acp",
        folder_var: "CLAUDE_CONFIG_DIR",
        scrub: &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_CUSTOM_HEADERS",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
            "CLAUDE_CODE_OAUTH_SCOPES",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "ANTHROPIC_PROFILE",
            "ANTHROPIC_FEDERATION_RULE_ID",
            "ANTHROPIC_ORGANIZATION_ID",
            "CLAUDE_SECURESTORAGE_CONFIG_DIR",
            "CLAUDE_CODE_SIMPLE",
            "CLAUDECODE",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CONFIG_DIR",
        ],
        home: ".claude",
        setup: &[
            "CLAUDE.md",
            "rules",
            "skills",
            "agents",
            "commands",
            "output-styles",
            CLAUDE_SETTINGS,
        ],
        servers_file: Some(".claude.json"),
        terminal_sign_in: true,
    },
    Support {
        agent: "codex-acp",
        folder_var: "CODEX_HOME",
        scrub: &[
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "OPENAI_ORG_ID",
            "OPENAI_PROJECT_ID",
            "CODEX_API_KEY",
            "CODEX_HOME",
        ],
        home: ".codex",
        setup: &[
            "AGENTS.md",
            "instructions.md",
            "skills",
            "prompts",
            CODEX_CONFIG,
        ],
        servers_file: None,
        terminal_sign_in: false,
    },
];

/// Claude Code's settings, copied without the keys that name a way to sign in.
const CLAUDE_SETTINGS: &str = "settings.json";
const SETTINGS_SIGN_IN: &[&str] = &["apiKeyHelper", "awsAuthRefresh", "awsCredentialExport"];

/// Codex's settings, of which only the MCP servers are copied: the same file
/// says how Codex signs in (a model provider and the key it reads, a bearer
/// token).
const CODEX_CONFIG: &str = "config.toml";

fn support(agent: &str) -> Option<&'static Support> {
    SUPPORT.iter().find(|support| support.agent == agent)
}

/// Whether this agent can run under accounts of the person's own here.
pub fn supported(agent: &str) -> bool {
    support(agent).is_some_and(|support| cfg!(unix) || !support.terminal_sign_in)
}

/// The agent a backend id starts, and the account it names when it names one.
pub fn split(backend_id: &str) -> (&str, Option<&str>) {
    match backend_id.split_once('#') {
        Some((agent, account)) if !agent.is_empty() && !account.is_empty() => {
            (agent, Some(account))
        }
        _ => (backend_id, None),
    }
}

/// The backend id a teammate runs an agent's account under.
pub fn backend_id(account: &Credential) -> String {
    format!("{}#{}", account.provider_id, account.id)
}

/// An account's folder. Only an id Hotline could have made names one, so
/// nothing read from the room stream becomes a path outside the folder.
fn folder(root: &Path, account: &str) -> Option<PathBuf> {
    let made_here = account
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-');
    (made_here && !account.is_empty())
        .then(|| hotline_support::paths::agent_accounts_dir(root).join(account))
}

/// Why a teammate on this backend id cannot start on its account: it names
/// one the room no longer holds, or whose folder is no longer there. `None`
/// for the agent as it is on this computer, and for an account that is here.
pub fn gone(root: &Path, backend_id: &str) -> Option<String> {
    let (agent, account) = split(backend_id);
    let account = account?;
    let here = support(agent).is_some()
        && held(root, agent, account)
        && folder(root, account)
            .and_then(|folder| fs::symlink_metadata(folder).ok())
            .is_some_and(|folder| folder.is_dir());
    if here {
        return None;
    }
    let name = registry::name_of(root, agent).unwrap_or_else(|| agent.to_string());
    Some(format!(
        "This teammate's {name} account is no longer on this desk. Pick another account in its model menu; it won't move to one on its own."
    ))
}

/// Whether the room still holds this account of this agent. The room says
/// an account exists, not its folder: an agent still running on a removed
/// one can write the folder back.
fn held(root: &Path, agent: &str, account: &str) -> bool {
    Vault::credentials(&Log::open(root).load(&StreamId::Room))
        .iter()
        .any(|credential| {
            credential.id == account
                && credential.provider_id == agent
                && credential.credential_kind == CredentialKind::Agent
                && !credential.revoked
        })
}

/// Points a launch at the account its backend id names: every inherited way
/// to sign in is taken away, and the account's folder is set last, so nothing
/// but the account's own sign-in is left for the agent to find. A launch for
/// the agent as it is on this computer is left as it was.
pub(super) fn sign_in_as(root: &Path, backend_id: &str, launch: &mut Launch) -> Result<(), String> {
    let (agent, Some(account)) = split(backend_id) else {
        return Ok(());
    };
    if let Some(gone) = gone(root, backend_id) {
        return Err(gone);
    }
    let (Some(support), Some(folder)) = (support(agent), folder(root, account)) else {
        unreachable!("gone() answered None, so the agent has accounts and the id is a folder");
    };
    let folder = std::path::absolute(folder).map_err(|error| error.to_string())?;
    // Windows ignores the case of a variable's name, so a lowercase spelling
    // of a scrubbed one would otherwise bring it back.
    let scrubbed = |name: &str| {
        support
            .scrub
            .iter()
            .any(|one| one.eq_ignore_ascii_case(name))
    };
    launch.env.retain(|(name, _)| !scrubbed(name));
    launch.unset = support
        .scrub
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    launch.env.push((
        support.folder_var.to_string(),
        folder.to_string_lossy().into_owned(),
    ));
    Ok(())
}

/// Adds an account of an agent: its folder, private like the vault and
/// started from the person's own setup, then the credential that names it.
/// The agent signs in there itself, the first time a teammate runs on it.
pub fn create(root: &Path, vault: &Vault, agent: &str, label: &str) -> Result<Credential, String> {
    let support = support(agent)
        .filter(|_| supported(agent))
        .ok_or_else(|| format!("{agent} has no accounts of its own here."))?;
    let label = label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err("An account's name is 1 to 80 characters.".to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let folder = folder(root, &id).expect("a uuid names a folder");
    let made = crate::vault::make_private_directory(&folder)
        .and_then(|()| copy_setup(support, &folder))
        .and_then(|()| vault.record_account(&id, agent, CredentialKind::Agent, label));
    made.map_err(|error| {
        let _ = fs::remove_dir_all(&folder);
        format!("The account could not be made: {error}")
    })
}

/// Takes an account's folder away, and its sign-in with it: before the
/// credential is revoked or deleted, so a crash between the two never leaves
/// a usable folder behind a row that says it is gone.
pub fn forget(root: &Path, account: &Credential) -> io::Result<()> {
    let Some(folder) = folder(root, &account.id) else {
        return Ok(());
    };
    match fs::remove_dir_all(folder) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn copy_setup(support: &Support, folder: &Path) -> io::Result<()> {
    let named = std::env::var_os(support.folder_var).filter(|named| !named.is_empty());
    let (source, servers_beside) = match (named, super::home()) {
        (Some(named), _) => (PathBuf::from(&named), PathBuf::from(named)),
        (None, Some(home)) => (home.join(support.home), home),
        (None, None) => return Ok(()),
    };
    copy_from(support, &source, &servers_beside, folder)
}

/// The copy itself, from a setup folder and the folder its servers file is
/// in, which the tests hand in.
fn copy_from(
    support: &Support,
    source: &Path,
    servers_beside: &Path,
    folder: &Path,
) -> io::Result<()> {
    for item in support.setup {
        let (from, to) = (source.join(item), folder.join(item));
        match *item {
            CLAUDE_SETTINGS => copy_settings(support, &from, &to)?,
            CODEX_CONFIG => copy_codex_servers(&from, &to)?,
            _ => copy_tree(&from, &to)?,
        }
    }
    if let Some(file) = support.servers_file
        && let Some(servers) =
            read_object(&servers_beside.join(file)).and_then(|mut state| state.remove("mcpServers"))
    {
        let only_servers = Value::Object(Map::from_iter([("mcpServers".to_string(), servers)]));
        fs::write(folder.join(file), only_servers.to_string())?;
    }
    Ok(())
}

/// A file or a folder, never through a link: one met on the way is left
/// behind, so nothing outside the setup comes with it. What is not there is
/// nothing to copy.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let Ok(found) = fs::symlink_metadata(from) else {
        return Ok(());
    };
    if found.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if found.is_file() {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// The settings without a way to sign in: the helper keys, and any scrubbed
/// variable its `env` would hand the agent. Settings that do not read as an
/// object are not copied, since what they hold cannot be checked.
fn copy_settings(support: &Support, from: &Path, to: &Path) -> io::Result<()> {
    let Some(mut settings) = read_object(from) else {
        return Ok(());
    };
    for key in SETTINGS_SIGN_IN {
        settings.remove(*key);
    }
    if let Some(Value::Object(env)) = settings.get_mut("env") {
        env.retain(|name, _| {
            !support
                .scrub
                .iter()
                .any(|one| one.eq_ignore_ascii_case(name))
        });
    }
    fs::write(to, Value::Object(settings).to_string())
}

/// Codex's `[mcp_servers]` and nothing else of its config.
fn copy_codex_servers(from: &Path, to: &Path) -> io::Result<()> {
    let Some(mut config) = read_plain(from)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.parse::<toml::Table>().ok())
    else {
        return Ok(());
    };
    let Some(servers) = config.remove("mcp_servers") else {
        return Ok(());
    };
    let only_servers = toml::Table::from_iter([("mcp_servers".to_string(), servers)]);
    fs::write(
        to,
        toml::to_string(&only_servers).map_err(io::Error::other)?,
    )
}

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    match serde_json::from_slice(&read_plain(path)?).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

/// A file's bytes, unless it is a link or not there.
fn read_plain(path: &Path) -> Option<Vec<u8>> {
    fs::symlink_metadata(path)
        .ok()
        .filter(fs::Metadata::is_file)?;
    fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch_with(env: &[(&str, &str)]) -> Launch {
        Launch {
            command: "/bin/claude-agent-acp".into(),
            args: Vec::new(),
            env: env
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            unset: Vec::new(),
        }
    }

    /// An account of `agent` as `create` leaves it, without the person's setup.
    fn held_account(root: &Path, agent: &str) -> Credential {
        let vault = Vault::open(root, Log::open(root)).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        fs::create_dir_all(folder(root, &id).unwrap()).unwrap();
        vault
            .record_account(&id, agent, CredentialKind::Agent, "Work")
            .unwrap()
    }

    #[test]
    fn a_backend_id_names_its_account_only_when_it_has_one() {
        assert_eq!(split("claude-acp"), ("claude-acp", None));
        assert_eq!(split("claude-acp#a-1"), ("claude-acp", Some("a-1")));
        // A stray '#' is not an account.
        assert_eq!(split("claude-acp#"), ("claude-acp#", None));
        assert_eq!(split("#a-1"), ("#a-1", None));
        assert!(supported("codex-acp"));
        // Claude Code signs in only in a terminal, which Hotline runs on Unix.
        assert_eq!(supported("claude-acp"), cfg!(unix));
        assert!(!supported("cursor") && !supported("hotline") && !supported("codex-acp#a-1"));
    }

    /// An account's agent finds nothing to sign in with but its own folder:
    /// the person's key is taken out of what it inherits and out of what its
    /// launch sets, whatever the case of its name, and the folder is set last.
    #[test]
    fn an_accounts_launch_signs_in_with_its_folder_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let account = held_account(root.path(), "claude-acp");
        let mut launch =
            launch_with(&[("anthropic_api_key", "sk-person"), ("NODE_OPTIONS", "--x")]);
        sign_in_as(root.path(), &backend_id(&account), &mut launch).unwrap();
        assert_eq!(launch.unset, support("claude-acp").unwrap().scrub);
        let folder = std::path::absolute(folder(root.path(), &account.id).unwrap()).unwrap();
        assert_eq!(
            launch.env,
            [
                ("NODE_OPTIONS".to_string(), "--x".to_string()),
                (
                    "CLAUDE_CONFIG_DIR".to_string(),
                    folder.to_string_lossy().into_owned()
                ),
            ]
        );

        // The agent as it is on this computer runs as it always did.
        let mut own = launch_with(&[("ANTHROPIC_API_KEY", "sk-person")]);
        sign_in_as(root.path(), "claude-acp", &mut own).unwrap();
        assert_eq!(own, launch_with(&[("ANTHROPIC_API_KEY", "sk-person")]));
    }

    /// The room says an account exists, not its folder: a folder with no
    /// account behind it, an account of another agent, an id Hotline could
    /// not have made and a link in the folder's place never start, and never
    /// on the agent's own sign-in instead.
    #[test]
    fn an_account_that_is_not_here_refuses_to_start() {
        let root = tempfile::tempdir().unwrap();
        let codex = held_account(root.path(), "codex-acp");
        fs::create_dir_all(folder(root.path(), "a-1").unwrap()).unwrap();
        for id in [
            "claude-acp#a-1".to_string(),
            format!("claude-acp#{}", codex.id),
            "claude-acp#../../vault".to_string(),
            "cursor#a-1".to_string(),
        ] {
            let mut launch = launch_with(&[]);
            let refused = sign_in_as(root.path(), &id, &mut launch).unwrap_err();
            assert!(refused.contains("no longer on this desk"), "{refused}");
            assert_eq!(gone(root.path(), &id), Some(refused));
        }
        assert_eq!(gone(root.path(), &backend_id(&codex)), None);
        assert_eq!(gone(root.path(), "claude-acp"), None);
        #[cfg(unix)]
        {
            let linked = held_account(root.path(), "codex-acp");
            let at = folder(root.path(), &linked.id).unwrap();
            fs::remove_dir(&at).unwrap();
            let elsewhere = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(elsewhere.path(), at).unwrap();
            assert!(gone(root.path(), &backend_id(&linked)).is_some());
        }
    }

    /// A new account starts from the person's instructions, skills, settings
    /// and servers, and never from anything that signs in: no helper, no key
    /// in the settings' env, nothing of the sign-in's own state, nothing of
    /// Codex's config but its servers, and nothing a link points at.
    #[test]
    fn a_new_account_starts_from_the_setup_but_never_its_sign_in() {
        let home = tempfile::tempdir().unwrap();
        let setup = home.path().join(".claude");
        fs::create_dir_all(setup.join("skills/review")).unwrap();
        fs::write(setup.join("CLAUDE.md"), "Be brief.").unwrap();
        fs::write(
            setup.join("skills/review/SKILL.md"),
            "---\nname: review\n---",
        )
        .unwrap();
        fs::write(setup.join(".credentials.json"), "{\"token\":\"secret\"}").unwrap();
        fs::write(
            setup.join("settings.json"),
            r#"{"apiKeyHelper":"vault-cli get","model":"opus","env":{"ANTHROPIC_API_KEY":"sk-person","DEBUG":"1"}}"#,
        )
        .unwrap();
        fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"me@example.com"},"mcpServers":{"docs":{"command":"docs-mcp"}}}"#,
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(home.path(), setup.join("skills/outside")).unwrap();

        let folder = tempfile::tempdir().unwrap();
        let claude = support("claude-acp").unwrap();
        copy_from(claude, &setup, home.path(), folder.path()).unwrap();
        let copied = folder.path();
        assert_eq!(
            fs::read_to_string(copied.join("CLAUDE.md")).unwrap(),
            "Be brief."
        );
        assert!(copied.join("skills/review/SKILL.md").is_file());
        assert!(!copied.join("skills/outside").exists());
        assert!(!copied.join(".credentials.json").exists());
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(copied.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            settings,
            serde_json::json!({"model": "opus", "env": {"DEBUG": "1"}})
        );
        let state: Value =
            serde_json::from_str(&fs::read_to_string(copied.join(".claude.json")).unwrap())
                .unwrap();
        assert_eq!(
            state,
            serde_json::json!({"mcpServers": {"docs": {"command": "docs-mcp"}}})
        );

        // Codex's servers come out of its config.toml, and nothing else of
        // it: not its model provider, the key that reads, or a bearer token.
        // Its sign-in, auth.json, stays.
        let codex = home.path().join(".codex");
        fs::create_dir_all(&codex).unwrap();
        fs::write(
            codex.join("config.toml"),
            "model_provider = \"azure\"\nexperimental_bearer_token = \"sk-person\"\n\n[model_providers.azure]\nenv_key = \"AZURE_OPENAI_API_KEY\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n",
        )
        .unwrap();
        fs::write(codex.join("auth.json"), "{}").unwrap();
        let folder = tempfile::tempdir().unwrap();
        copy_from(
            support("codex-acp").unwrap(),
            &codex,
            home.path(),
            folder.path(),
        )
        .unwrap();
        let config: toml::Table = fs::read_to_string(folder.path().join("config.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            config,
            "[mcp_servers.docs]\ncommand = \"docs-mcp\"\n"
                .parse::<toml::Table>()
                .unwrap()
        );
        assert!(!folder.path().join("auth.json").exists());
    }

    /// Adding an account makes its private folder and names it on the room
    /// stream as the agent's, with no secret; forgetting it takes the folder,
    /// and with it the sign-in, so a teammate on it no longer starts.
    #[test]
    fn an_account_is_a_private_folder_until_it_is_forgotten() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::open(root.path(), Log::open(root.path())).unwrap();
        assert!(create(root.path(), &vault, "cursor", "Work").is_err());
        assert!(create(root.path(), &vault, "codex-acp", "  ").is_err());
        let account = create(root.path(), &vault, "codex-acp", " Work ").unwrap();
        assert_eq!(
            (
                account.provider_id.as_str(),
                account.label.as_str(),
                account.credential_kind
            ),
            ("codex-acp", "Work", CredentialKind::Agent)
        );
        assert_eq!(vault.list(), std::slice::from_ref(&account));
        let id = backend_id(&account);
        assert_eq!(gone(root.path(), &id), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let folder = folder(root.path(), &account.id).unwrap();
            let mode = fs::metadata(folder).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        forget(root.path(), &account).unwrap();
        assert!(gone(root.path(), &id).is_some());
        // Forgetting twice changes nothing.
        forget(root.path(), &account).unwrap();
    }

    /// A teammate on a gone account comes up failed without starting
    /// anything, on the agent's own sign-in or any other, and its card names
    /// the agent and the model it ran, so the person can move it to another
    /// account.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_teammate_on_a_gone_account_comes_up_on_the_card_that_offers_others() {
        use crate::driver::Driver;
        use crate::driver::acp::{ChildAgent, lock, tests};
        use crate::mcp::server::TeammateTools;
        let root = tempfile::tempdir().unwrap();
        // A fresh, empty catalogue: the start asks the network for nothing.
        fs::create_dir_all(root.path().join("cache")).unwrap();
        fs::write(
            hotline_support::paths::acp_registry_path(root.path()),
            serde_json::json!({"format": 2, "fetchedAt": chrono::Utc::now().timestamp_millis(), "agents": []})
                .to_string(),
        )
        .unwrap();
        let held = tests::room("gone-account");
        let mut ada = tests::persona(&tests::scratch_cwd(), Vec::new());
        ada.backend_id = "claude-acp#gone".into();
        ada.model_id = Some("opus".into());
        let driver = ChildAgent::new(
            root.path().to_path_buf(),
            ada.backend_id.clone(),
            "briefing".into(),
            TeammateTools::new(&held, "ada"),
        );
        driver.start(&ada).await.unwrap();
        let notice = driver.startup_failure().expect("the card is up");
        for said in [
            r#""kind":"account_gone""#,
            r#""backend":"claude-acp#gone""#,
            r#""model":"opus""#,
            "no longer on this desk",
        ] {
            assert!(notice.contains(said), "{notice}");
        }
        assert!(lock(&driver.child).is_none(), "nothing was started");
    }
}
