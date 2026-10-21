# Shared names for the agent-accounts plan (every task uses exactly these)

Spec: C:\Users\kelviny\stellar\hotline\docs\superpowers\specs\2026-10-21-agent-accounts-design.md
Repo: C:\Users\kelviny\stellar\hotline (Windows dev machine, Git Bash, no make)

## hotline-contract — new file crates/hotline-contract/src/accounts.rs (all #[ts(export, export_to = "contract.ts")], serde camelCase unless noted, optional fields skip_serializing_if none + ts optional_fields)

- `pub struct AgentAccount { pub id: String /* uuid v4 */, pub backend_id: String, pub name: String, pub created_at: i64, pub login_at: Option<i64>, pub setup: Option<AccountSetupOrigin> }`
- `pub struct AccountSetupOrigin { pub from: SetupFrom, pub at: i64, pub categories: Vec<String> }`
- `pub enum SetupFrom { SystemDefault, Empty }` (serde rename_all snake_case: "system_default", "empty")
- `pub enum AccountStatusState { NotChecked, Checking, SignedIn, SignedOut, Unavailable }` (snake_case)
- `pub struct AccountStatusView { pub state: AccountStatusState, pub checked_at: Option<i64>, pub email: Option<String>, pub plan: Option<String> }`
- `pub enum LoginAttemptState { Running, Succeeded, Failed }` (snake_case)
- `pub struct LoginAttemptView { pub id: String, pub state: LoginAttemptState, pub error: Option<String> }`
- `pub struct AccountView { pub account: AgentAccount, pub status: AccountStatusView, pub attempt: Option<LoginAttemptView>, pub used_by: Vec<String>, pub folder: Option<String>, pub can_sign_in_here: bool }`
- `pub struct AccountAgent { pub backend_id: String, pub name: String, pub supports_accounts: bool, pub takes_email: bool }`
- `pub struct AccountsList { pub agents: Vec<AccountAgent>, pub accounts: Vec<AccountView> }`
- `pub struct AccountCreated { pub account: AccountView, pub copy: Option<SetupCopyReport> }`
- `pub struct SetupCategoryView { pub id: String, pub count: u32, pub path: Option<String> }`
- `pub struct AccountSetupView { pub setup: Option<AccountSetupOrigin>, pub categories: Vec<SetupCategoryView> }`
- `pub struct SetupCopyReport { pub copied: Vec<String>, pub removed_keys: Vec<String>, pub links_skipped: u32 }`
- `pub enum AccountHold { NotSignedIn, SigningIn }` (snake_case)
- `pub struct SessionAccount { pub running: Option<String>, pub wanted: Option<String>, pub held: Option<AccountHold> }`

Additive fields on existing types (all Option, skip when None, absent = System default / legacy):
- `Persona.agent_account_id: Option<String>` (persona.rs, next to harness_override)
- `PersonaDraft.agent_account_id: Option<String>`
- `SessionCheckpoint.account_id: Option<String>` (#[serde(default, skip_serializing_if = "Option::is_none")])
- `SessionInfo.account: Option<SessionAccount>` (session.rs, like awaiting_subagents/processes style)
- `TranscriptEvent::Chapter` and `TranscriptEvent::Side`: `account_id: Option<String>` (transcript.rs; additive tape field)

Command variants (command.rs, after SecretsPasskeyCancel; doc comment ends "Owner or local desk only."):
- `#[serde(rename = "accounts.list")] AccountsList {}` → AccountsList
- `#[serde(rename = "accounts.create")] AccountsCreate { backend_id: String, name: String, start_with: SetupFrom, #[serde(default, skip_serializing_if...)] categories: Option<Vec<String>> }` → AccountCreated
- `#[serde(rename = "accounts.rename")] AccountsRename { account_id: String, name: String }` → AccountView
- `#[serde(rename = "accounts.remove")] AccountsRemove { account_id: String }` → {} (ok)
- `#[serde(rename = "accounts.status")] AccountsStatus { account_id: String }` → AccountStatusView
- `#[serde(rename = "accounts.login.start")] AccountsLoginStart { account_id: String, email: Option<String> }` → `{ "id": String }`
- `#[serde(rename = "accounts.login.poll")] AccountsLoginPoll { account_id: String, id: String }` → AuthStatus `{state, output, error?}` (existing hotline-room AuthStatus)
- `#[serde(rename = "accounts.login.input")] AccountsLoginInput { account_id: String, id: String, input: String }` → {}
- `#[serde(rename = "accounts.login.cancel")] AccountsLoginCancel { account_id: String, id: String }` → {}
- `#[serde(rename = "accounts.setup.list")] AccountsSetupList { account_id: String }` → AccountSetupView
- `#[serde(rename = "accounts.setup.copy")] AccountsSetupCopy { account_id: String, categories: Vec<String> }` → SetupCopyReport
- `#[serde(rename = "persona.account")] PersonaAccount { persona_id: String, account_id: Option<String> /* null = System default; use present_option if needed */ }` → SessionAccount

## hotline-support
- `paths::agent_accounts_dir(root: &Path) -> PathBuf` = root/"agent-accounts"
- `paths::agent_account_dir(root: &Path, id: &str) -> Option<PathBuf>` (None unless uuid::Uuid::parse_str(id) ok)
- `private_dir::make_private_directory(path: &Path) -> io::Result<()>` moved from crates/hotline-core/src/vault.rs into hotline-support (new module `private_dir`), vault calls it from there
- `private_dir::real_directory(path: &Path, expected: &Path) -> io::Result<PathBuf>`: canonical (dunce) path, Err if symlink/junction/reparse point or canonical != canonical(expected)
- `process_windows::prepare_visible(command: &mut tokio::process::Command)` = CREATE_SUSPENDED | CREATE_NEW_CONSOLE + kill_on_drop(false)
- `process_windows::Job::attach_without_kill_on_close(pid) -> io::Result<Job>` and `Job::terminate(&self)` (exists)

## hotline-room
- `room::AGENT_ACCOUNTS: &str = "agentAccounts"` setting key; default []
- `room::agent_accounts(log: &Log) -> Vec<AgentAccount>` (lenient per-entry filter: uuid id, non-empty backend, name 1..=64 trimmed, unique id, unique (backend, lowercase name))
- `room::write_agent_accounts(log: &Log, accounts: &[AgentAccount]) -> io::Result<()>` (one setting event)
- `settings::apply_update` refuses AGENT_ACCOUNTS ("Agent accounts are changed with accounts.*.")
- `room::checkpoint_session(log, persona_id, backend_id, account_id: Option<&str>, session_id)` and `room::clear_checkpoint(log, persona_id, backend_id, account_id: Option<&str>)` match on (backend, account)
- `room::clear_account_checkpoints(log, account_id: &str)` (every persona) and `room::clear_backend_checkpoints(log, persona_id, backend_id)` (every account of that backend)
- persona-record lock: `room::persona_write_lock() -> MutexGuard<'static, ()>` (std Mutex, short) taken by with_checkpoints and by persona.account's append
- hotline-store link.rs: `AgentBinding { backend_id, session_id, account_id: Option<String> }` (+ Link::line / read_link carry `accountId`)

## hotline-core — new module crates/hotline-core/src/accounts.rs with children accounts/{support,env,resolve,status,login,setup,switch}.rs
- support.rs: `pub(crate) struct AccountSupport { env_var, login_args, email_flag: Option<&str>, status_args, parse_status: fn(&[u8], &Path) -> AccountStatusView, scrub: &[&str], usage_limit_prefixes: &[&str], setup: &[SetupCategory], settings_auth_keys: &[&str] }`, `pub(crate) struct SetupCategory { id, paths: &[&str], json_keys: &[(&str, &[&str])], open: &str }`, `pub(crate) fn account_support(backend_id: &str) -> Option<&'static AccountSupport>`, `pub(crate) const CLAUDE: AccountSupport`
- env.rs: `pub(crate) struct EnvPlan { pub remove: Vec<OsString>, pub set: Vec<(OsString, OsString)> }`; `pub(crate) fn env_plan(catalogue: &[(String, String)], method: &[(String, String)], account: Option<&ResolvedAccount>) -> EnvPlan`; `impl EnvPlan { pub fn apply(&self, cmd: &mut tokio::process::Command); pub fn apply_pty(&self, cmd: &mut portable_pty::CommandBuilder) }`. Order: catalogue → method → scrub (remove) → env_var=dir last. account None ⇒ remove empty, set = catalogue+method only (legacy).
- resolve.rs: `pub(crate) struct ResolvedAccount { pub id: String, pub name: String, pub dir: PathBuf, pub support: &'static AccountSupport, pub login_at: Option<i64> }`; `pub(crate) enum AccountRefusal { UnknownAccount { account_id: String }, FolderUnavailable { account_id: String, name: String, path: PathBuf, kind: std::io::ErrorKind } }` with `fn private_sentence(&self, teammate: &str, agent: &str) -> String` and `fn public_sentence(&self, teammate: &str, agent: &str) -> String`; `pub(crate) fn resolve(root: &Path, log: &Log, persona: &Persona) -> Result<Option<ResolvedAccount>, AccountRefusal>`
- status.rs: `pub(crate) fn parse_claude_status(stdout: &[u8], dir: &Path) -> AccountStatusView`; `pub(crate) struct AccountStatuses` (in-memory cache + single-flight per account) with `async fn check(&self, root: &Path, account: &ResolvedAccount) -> AccountStatusView`, `fn cached(&self, id) -> AccountStatusView`
- login.rs: `pub(crate) struct AccountLogins` (one attempt per account) with `async fn start(self: &Arc<Self>, root, account: &ResolvedAccount, email: Option<String>, owner: CancellationToken, seat_is_desk: bool) -> Result<String, String>`, `fn poll(&self, account_id, id) -> Result<AuthStatus, String>`, `fn input(..)`, `fn cancel(..)`
- setup.rs: `pub(crate) fn system_default_source() -> Option<SetupSource { dir: PathBuf, claude_json: PathBuf }>`; `pub(crate) fn copy_setup(source, account_dir: &Path, support, categories: &[String]) -> io::Result<SetupCopyReport>`; `pub(crate) fn setup_view(account_dir, support, origin, with_paths: bool) -> AccountSetupView`
- switch.rs: apply/hold/pending logic on Room (`Room::assign_account(persona_id, Option<String>) -> Result<SessionAccount, String>`, `Room::apply_account_when_idle(persona_id)`)
- Driver: `ChildAgent::with_account(self, Option<ResolvedAccount>) -> Self`; `Driver` failure kinds `Kind::UsageLimit` and `Kind::AccountUnavailable` in driver/failure.rs; `Failure` gains `account_id: Option<String>` serialized in the hotlineFailure JSON as `accountId`.
- Session: `Session { account_id: Option<String>, account_login_at: Option<i64>, .. }`

## UI (ui/src)
- `components/settings/AgentAccounts.tsx` exports `AgentAccountsSection({ roster, onBack })`
- `components/teammate/AccountRow.tsx` exports `AccountRow({ persona, session, disabled })`
- `components/accounts/AddAccountForm.tsx` exports `AddAccountForm({ backendId, agentName, takesEmail, compact, onAdded })` shared by Settings, NewTeammate, Teammate
- `components/AgentSignIn.tsx` exports `SignInTerminal({ job, onDone })` (extracted from SignInAttempt) where `job = { start(): Promise<{id:string}>; poll(id): Promise<AuthStatus>; input(id, text): Promise<void>; cancel(id): Promise<void> }`
- `Settings.tsx` section id "accounts", title "Agent accounts", placed after "providers"
- wire.ts Results rows for every new command

## Sentences (exact copy from spec)
- private unknown: "Pick a {agent} account for {teammate}: the one it was set to is not on this desk."
- private folder: "Can't open {account}'s folder ({path}): {kind}. Fix the folder or pick another account."
- public: "{teammate} can't start: its {agent} account needs attention at the desk."
- remove in use: "{account} can't be removed while {names} use it."
- agent.auth refusal: "Sign in {account} from its account instead."
- owner windows login: "Sign in from the desk itself."
- login closed: "Claude Code closed before it was signed in."
