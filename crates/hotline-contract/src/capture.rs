//! Credential values cross only the authenticated capture command, never an event.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum CaptureKind {
    Login,
    Passkey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum CredentialStatus {
    Pending,
    Stored,
    Declined,
    Expired,
    Withdrawn,
}

/// Write-only fields. Debug output deliberately cannot print any typed value.
#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CredentialValues {
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp: Option<String>,
}
impl std::fmt::Debug for CredentialValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialValues([redacted])")
    }
}

/// No site or teammate is accepted here: both are bound by the live request.
/// No name means decline. A name without values grants a matching existing
/// key, or starts a passkey arming. Values are accepted only for a new login.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct CredentialAnswer {
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<CredentialValues>,
    #[serde(default)]
    pub ask: bool,
}
