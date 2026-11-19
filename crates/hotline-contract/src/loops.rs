//! Living open loops: unfinished work and standing items a teammate carries
//! across chapters.
//!
//! Each loop is kept on the teammate's own stream (`loops/<id>`), written
//! again under the same id each time its state or summary changes: the fold is
//! the latest line of each id.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where an open loop stands in its lifecycle.
///
/// `active` and `waiting` are live work; `stale` was untouched past its window
/// or passed its review moment; `done` finished recently and stays visible
/// briefly so the next chapter knows it is settled; `archived` left the active
/// view and stays searchable in the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum LoopState {
    #[default]
    Active,
    Waiting,
    Stale,
    Done,
    Archived,
}

impl LoopState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::Stale => "stale",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Self::Active | Self::Waiting | Self::Stale)
    }
}

/// How an open loop entered the teammate's working memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum LoopSource {
    /// Extracted when a conversation chapter closed.
    #[default]
    Chapter,
    /// Declared explicitly by a turn via `outcome(type="open")`.
    Outcome,
}

/// One open loop carried by a teammate across chapters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct OpenLoop {
    pub id: String,
    /// Short stable slug (e.g. `larder`, `ca-filing`) used for reconciliation
    /// and display in the thread index.
    pub key: String,
    /// One plain-English line describing what is open.
    pub summary: String,
    pub state: LoopState,
    pub source: LoopSource,
    /// When the loop was first opened, milliseconds since epoch.
    pub created_at: i64,
    /// When a human or teammate last touched or updated the loop, milliseconds
    /// since epoch. Automated aging to `stale` leaves this alone so age stays
    /// truthful.
    pub updated_at: i64,
    /// Optional explicit deadline or review moment, milliseconds since epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_by: Option<i64>,
    /// Scheduled job id this loop tracks; closes as `done` when the job ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Chapter id where this loop was last touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter_id: Option<String>,
    /// When the loop became `done` or `archived`, milliseconds since epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,
    /// Fields written by a newer Hotline that this build does not read yet,
    /// preserved on round-trip.
    #[serde(flatten)]
    #[ts(skip)]
    pub newer: serde_json::Map<String, serde_json::Value>,
}
