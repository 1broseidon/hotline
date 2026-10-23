//! Agent accounts: the named sign-ins a teammate's agent can run under, and
//! what the desk shows of them.
//!
//! Only [`AgentAccount`] is kept (in the room's `agentAccounts` setting).
//! Everything else here is a view built when asked: a status, a sign-in
//! attempt and the setup a folder holds live in memory or on disk and are
//! never written to a stream. Absent from a teammate, an account means
//! This computer: the agent's own sign-in.

use crate::Credential;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One sign-in the operator keeps for an agent, as stored. No secret and no
/// path is kept: the folder it signs in under is derived from `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AgentAccount {
    /// A v4 UUID, stable for the account's life and the name of its folder.
    pub id: String,
    pub backend_id: String,
    pub name: String,
    pub created_at: i64,
    /// When a sign-in last finished here, so a session can tell its login
    /// from a newer one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_at: Option<i64>,
    /// Where the folder's setup came from. Absent on an account with no record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<AccountSetupOrigin>,
}

/// Where an account folder's setup came from, written on creation and again
/// on each "Copy again".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountSetupOrigin {
    pub from: SetupFrom,
    pub at: i64,
    pub categories: Vec<String>,
}

/// What a new account's folder starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum SetupFrom {
    /// A copy of the setup the agent has when run without an account.
    SystemDefault,
    Empty,
}

/// Whether an account is signed in, as last found out. Never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum AccountStatusState {
    NotChecked,
    Checking,
    SignedIn,
    SignedOut,
    /// The agent could not be asked, or did not answer.
    Unavailable,
}

/// What the agent said about an account's sign-in. Kept in memory only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountStatusView {
    pub state: AccountStatusState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum LoginAttemptState {
    Running,
    Succeeded,
    Failed,
}

/// The one sign-in attempt an account may have going. Kept in memory only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct LoginAttemptView {
    pub id: String,
    pub state: LoginAttemptState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// An account as the desk lists it: the stored record with its sign-in state
/// and who uses it. Answered only to the desk and an owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountView {
    pub account: AgentAccount,
    pub status: AccountStatusView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<LoginAttemptView>,
    pub used_by: Vec<String>,
    /// Where the account's folder is. Desk and owner only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Whether this seat may start a sign-in. On Windows only the desk can,
    /// because the agent opens its own console window.
    pub can_sign_in_here: bool,
}

/// One agent the desk can name an account for, and what it can do with one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountAgent {
    pub backend_id: String,
    pub name: String,
    pub supports_accounts: bool,
    /// Whether its sign-in takes an email to pre-fill.
    pub takes_email: bool,
    /// The provider whose page lists this agent's accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

/// A Hotline Agent credential as the desk lists it. Answered only to the
/// desk and an owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CredentialAccount {
    pub credential: Credential,
    pub section: AccountSection,
    /// The teammates it serves: those set to it, and on its provider's
    /// default those whose choice does not name another of its accounts.
    pub used_by: Vec<String>,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountsList {
    pub agents: Vec<AccountAgent>,
    pub accounts: Vec<AccountView>,
    pub credentials: Vec<CredentialAccount>,
}

/// What `accounts.create` answers: the new account, and what was copied into
/// its folder when it started from This computer's setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountCreated {
    pub account: AccountView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<SetupCopyReport>,
}

/// One kind of setup an agent keeps, as found in an account's folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SetupCategoryView {
    pub id: String,
    /// How many items the folder holds in it.
    pub count: u32,
    /// What "Open" opens. Desk and owner only; only the desk's window offers Open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What `accounts.setup.list` answers: where the setup came from, and what
/// the folder holds now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct AccountSetupView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<AccountSetupOrigin>,
    pub categories: Vec<SetupCategoryView>,
}

/// What a copy of This computer's setup did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SetupCopyReport {
    pub copied: Vec<String>,
    /// Sign-in settings left out of the copied settings, by key.
    pub removed_keys: Vec<String>,
    /// Links in the source that were skipped rather than followed.
    pub links_skipped: u32,
}

/// Where an account is listed on its provider's page: by what it bills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum AccountSection {
    Subscription,
    ApiKey,
    Server,
}

/// Why a chosen account is not yet the one running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum AccountHold {
    NotSignedIn,
    SigningIn,
}

/// Which account a teammate's session runs on and which it will move to.
/// Ids only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct SessionAccount {
    /// The account the live session started with. Absent is the default:
    /// This computer, or on Hotline Agent its model's provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
    /// The account the teammate is set to: the running one, unless a switch
    /// to another is still pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wanted: Option<String>,
    /// Why the switch to `wanted` has not happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<AccountHold>,
}
