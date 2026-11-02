//! Inbound sources: places outside Hotline (a webhook, a mailbox, a GitHub
//! repository) the person lets leave a teammate asides
//! (`docs/design/thread-awareness.md`, "Inbound sources").
//!
//! A source is a standing grant the person owns, kept on the room stream like
//! a teammate or a tool source. Only its shape exists yet: no transport
//! delivers through one.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

/// What kind of place a source is. A label only, for now: no transport is
/// built for any of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum InboundKind {
    Webhook,
    Mail,
    Github,
    Other,
}

impl InboundKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Mail => "mailbox",
            Self::Github => "GitHub",
            Self::Other => "other",
        }
    }
}

/// One inbound source as the room keeps it (`inbound` on the room stream,
/// folded by id) and `inbound.list` answers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct InboundSource {
    pub id: String,
    /// `type` on the wire, as a tool source's is: on the room stream `kind`
    /// is the event's.
    #[serde(rename = "type")]
    pub kind: InboundKind,
    pub name: String,
    /// The teammates it may leave asides, by id. Nobody else.
    pub teammates: Vec<String>,
    /// The highest level it may use; a higher one is lowered to this.
    #[serde(default)]
    pub max_level: AsideLevel,
    /// Whether it may carry files into a teammate's workspace.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub files_allowed: bool,
    /// Muted, it delivers nothing. The brake mutes it too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub muted: bool,
    /// The most asides it may leave in an hour, across its teammates.
    pub rate: u32,
    /// The name of a secret stored under Settings → Secrets that a transport
    /// for it will use (a webhook's signing key, a mailbox's password): a
    /// reference, never the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// How many times its nudges have woken a teammate since the person last
    /// unmuted it: the brake, counted as a pair's is.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[ts(as = "Option<u32>", optional)]
    pub wakes: u32,
    pub created_at: i64,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the field by reference"
)]
fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// A source as the person adds it (`inbound.add`) or a teammate proposes it:
/// everything but its kind, name and teammates has a default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct InboundDraft {
    #[serde(rename = "type")]
    pub kind: InboundKind,
    pub name: String,
    pub teammates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level: Option<AsideLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}
