//! The table of what each agent needs to run under an account: one row per
//! agent that can, found by the agent's id rather than through a trait. The
//! row says how the agent is pointed at a folder, which credentials it must not
//! inherit, how to ask it who is signed in and how to sign it in, and what of
//! its user-level setup an account folder holds.
//!
//! Nothing outside a row and its own tests names the agent it describes;
//! giving another agent accounts is a row, a parser for its status answer and
//! the tests of both.

use crate::contract::AccountStatusView;
use std::path::Path;

/// What running one agent under an account takes. Fields are data; the parser
/// is the one piece of code, because each agent words its answer its own way.
#[derive(Debug)]
pub(crate) struct AccountSupport {
    /// The provider whose page lists the agent's accounts.
    pub(crate) provider_id: &'static str,
    /// The environment variable that points the agent at its account folder.
    pub(crate) env_var: &'static str,
    /// The arguments, after the launch's own, that sign the agent in.
    pub(crate) login_args: &'static [&'static str],
    /// The flag that hands the sign-in an email, for an agent that takes one.
    pub(crate) email_flag: Option<&'static str>,
    /// The arguments, after the launch's own, that make the agent say who is
    /// signed in.
    pub(crate) status_args: &'static [&'static str],
    /// Reads the answer to `status_args` as what it says about the folder it
    /// was run for. Whatever it cannot vouch for, for that folder, is unavailable.
    pub(crate) parse_status: fn(&[u8], &Path) -> AccountStatusView,
    /// Every variable an account's process must not inherit, because it would
    /// replace or change which sign-in the agent uses. It ends with `env_var`,
    /// which the account's folder takes over.
    pub(crate) scrub: &'static [&'static str],
    /// How the agent's out-of-usage messages begin, so a failed turn that says
    /// one of them can be told from any other failure.
    pub(crate) usage_limit_prefixes: &'static [&'static str],
    pub(crate) home_dir: &'static str,
    /// Beside `home_dir` in the home, but inside the folder `env_var` names.
    /// A category's `json_keys` name it by this spelling.
    pub(crate) state_file: Option<&'static str>,
    /// The user-level setup a folder can hold, which a new account may start from.
    pub(crate) setup: &'static [SetupCategory],
    /// Keys of the agent's settings file that name a way to sign in, which the
    /// copy of a setup leaves behind.
    pub(crate) settings_auth_keys: &'static [&'static str],
}

/// One kind of setup an account folder holds: what is copied for it, and what
/// "Open" shows.
#[derive(Debug)]
pub(crate) struct SetupCategory {
    pub(crate) id: &'static str,
    /// Files and folders, relative to the setup's source and to the account folder.
    pub(crate) paths: &'static [&'static str],
    /// Keys merged out of a JSON file, one `(file, keys)` pair per file.
    pub(crate) json_keys: &'static [(&'static str, &'static [&'static str])],
    /// What "Open" shows, relative to the account folder: a file, or a folder
    /// when it ends in `/`.
    pub(crate) open: &'static str,
}

pub(crate) fn account_support(backend_id: &str) -> Option<&'static AccountSupport> {
    ROWS.iter()
        .find(|(id, _)| *id == backend_id)
        .map(|(_, row)| *row)
}

static ROWS: [(&str, &AccountSupport); 1] = [("claude-acp", &CLAUDE)];

/// The `open` values of every row's setup. The desk shell is handed a path and
/// not an agent, so it checks a request against all of them; a folder ends in `/`.
pub fn account_setup_items() -> Vec<&'static str> {
    ROWS.iter()
        .flat_map(|(_, row)| row.setup.iter().map(|category| category.open))
        .collect()
}

/// A `static` and not a `const`, so that every `&CLAUDE` is the one row, and a
/// resolved account can be checked against the table by address.
pub(crate) static CLAUDE: AccountSupport = AccountSupport {
    provider_id: "anthropic",
    env_var: "CLAUDE_CONFIG_DIR",
    login_args: &["--cli", "auth", "login", "--claudeai"],
    email_flag: Some("--email"),
    status_args: &["--cli", "auth", "status", "--json"],
    parse_status: super::status::parse_claude_status,
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
    // `USAGE_LIMIT_ERROR_PREFIXES` of `@anthropic-ai/claude-agent-sdk` 0.3.270,
    // the copy bundled by `@agentclientprotocol/claude-agent-acp` 0.78.0
    // (driver/acp/registry.rs), in the SDK's order.
    usage_limit_prefixes: &[
        "You've hit your",
        "You've reached your",
        "You're out of usage credits",
        "Your org is out of usage \u{b7} add funds to continue",
        "Your org is out of usage \u{b7} contact your admin",
        "Your seat type doesn't include usage credits",
        "Your seat type doesn't include usage",
        "Your usage allocation has been disabled by your admin",
        "Your group's usage limit is set to $0",
        "Fable 5 requires usage credits",
        "You're out of extra usage",
        "Your seat type doesn't include extra usage",
    ],
    home_dir: ".claude",
    state_file: Some(".claude.json"),
    setup: &[
        SetupCategory {
            id: "instructions",
            paths: &["CLAUDE.md", "rules"],
            json_keys: &[],
            open: "CLAUDE.md",
        },
        SetupCategory {
            id: "skills",
            paths: &["skills", "agents", "commands", "output-styles"],
            json_keys: &[],
            open: "skills/",
        },
        SetupCategory {
            id: "plugins",
            paths: &["plugins"],
            json_keys: &[(
                "settings.json",
                &["enabledPlugins", "extraKnownMarketplaces"],
            )],
            open: "plugins/",
        },
        SetupCategory {
            id: "mcp",
            paths: &[],
            json_keys: &[(".claude.json", &["mcpServers"])],
            open: ".claude.json",
        },
        SetupCategory {
            id: "settings",
            paths: &["settings.json"],
            json_keys: &[],
            open: "settings.json",
        },
    ],
    settings_auth_keys: &["apiKeyHelper", "awsAuthRefresh", "awsCredentialExport"],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AccountStatusState;

    #[test]
    fn only_claude_code_has_a_row_and_it_carries_the_spec_values() {
        for none in ["codex-acp", "hotline", ""] {
            assert!(account_support(none).is_none());
        }
        let row = account_support("claude-acp").expect("the one row");
        assert_eq!(
            (row.env_var, row.email_flag),
            ("CLAUDE_CONFIG_DIR", Some("--email"))
        );
        assert_eq!(row.login_args, ["--cli", "auth", "login", "--claudeai"]);
        assert_eq!(row.status_args, ["--cli", "auth", "status", "--json"]);
        assert_eq!(
            row.scrub,
            [
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
            ]
        );
        assert_eq!(row.usage_limit_prefixes.len(), 12);
        assert!(
            row.usage_limit_prefixes
                .contains(&"Your org is out of usage \u{b7} add funds to continue")
        );
        assert_eq!(
            row.setup.iter().map(|c| c.id).collect::<Vec<_>>(),
            ["instructions", "skills", "plugins", "mcp", "settings"]
        );
        assert_eq!(
            row.setup.iter().map(|c| c.open).collect::<Vec<_>>(),
            [
                "CLAUDE.md",
                "skills/",
                "plugins/",
                ".claude.json",
                "settings.json"
            ]
        );
        assert_eq!(
            (row.home_dir, row.state_file),
            (".claude", Some(".claude.json"))
        );
        assert_eq!(row.setup[0].paths, ["CLAUDE.md", "rules"]);
        assert_eq!(
            row.setup[2].json_keys,
            [(
                "settings.json",
                &["enabledPlugins", "extraKnownMarketplaces"][..]
            )]
        );
        assert_eq!(
            row.setup[3].json_keys,
            [(".claude.json", &["mcpServers"][..])]
        );
        assert_eq!(
            row.settings_auth_keys,
            ["apiKeyHelper", "awsAuthRefresh", "awsCredentialExport"]
        );
        assert_eq!(
            (row.parse_status)(br#"{"loggedIn":false}"#, Path::new(".")).state,
            AccountStatusState::SignedOut
        );
    }

    #[test]
    fn open_may_show_only_what_a_row_names() {
        assert_eq!(
            account_setup_items(),
            [
                "CLAUDE.md",
                "skills/",
                "plugins/",
                ".claude.json",
                "settings.json"
            ]
        );
    }
}
