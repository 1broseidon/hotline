//! Credentials, providers and logins.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// A provider credential, as the room knows one: everything except the secret.
///
/// The secret is never an event. It lives in the vault — a `0600` file in a
/// `0700` directory on this machine — and the room stream carries only this,
/// so a stream can be read, copied or shipped without a key going with it.
/// A `credential` event is these fields under that `kind`; a deletion is the
/// same id with `deleted: true` and nothing else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Credential {
    pub id: String,
    pub provider_id: String,
    /// The event's own `kind` names the event, so a credential's kind is
    /// spelled differently here: two fields called `kind` would be one field.
    pub credential_kind: CredentialKind,
    /// The chosen Ollama or custom server. Other providers use their fixed endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Custom connections keep their protocol and model ids with their identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomProvider>,
    /// What the user called it, so a list of keys is a list they recognise.
    pub label: String,
    /// Revoked. Set once and never unset — revocation is a fact, not a toggle.
    pub revoked: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A provider Hotline Agent can hold a credential for, as the key form offers
/// them. Which ones there are is `models::WIRING`; the name and the doc link
/// come from the model catalogue. A provider can offer several ways to connect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub credential_kinds: Vec<CredentialKind>,
    /// Whether Rig can discover models using this provider connection.
    pub model_discovery: bool,
    /// What a connection to it can do, in the order Settings names them:
    /// `chat`, `images`, `voice`. Absent from a desk older than the field.
    #[serde(default)]
    #[ts(as = "Option<Vec<ProviderJob>>", optional)]
    pub does: Vec<ProviderJob>,
}

/// A job a provider can do once connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum ProviderJob {
    Chat,
    Images,
    Voice,
}

/// How this connection was established: a pasted key, a provider sign-in,
/// or a keyless server URL. OAuth does not imply subscription billing.
/// `agent` is an account of an agent that keeps its own sign-in — Claude
/// Code, Codex — in a folder of its own: its provider id is the agent's
/// backend id, and Hotline holds no secret for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum CredentialKind {
    ApiKey,
    Oauth,
    Local,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum OpenAiApi {
    Responses,
    ChatCompletions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct CustomProvider {
    pub api: OpenAiApi,
    pub models: Vec<String>,
}

/// Input only. The key is never copied into credential metadata or a stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CustomProviderDraft {
    pub name: String,
    pub base_url: String,
    pub api: OpenAiApi,
    pub models: Vec<String>,
    /// Omitted keeps an existing key; an empty string removes it.
    pub secret: Option<String>,
}

/// Instructions for provider sign-in. Browser callback flows leave `user_code`
/// empty; device flows supply the code to enter at `verification_uri`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct LoginPrompt {
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
}

/// How far a provider login has got. A finished one stays queryable until
/// the process exits, so a window that missed the moment can still read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct LoginStatus {
    pub state: LoginState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<Credential>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "contract.ts")]
pub enum LoginState {
    Pending,
    Done,
    Failed,
}
