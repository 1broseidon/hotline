//! Updates to the agent harnesses a desk runs: one row per harness, however
//! many parts it is made of (its CLI, the adapter that speaks ACP for it, a
//! release Hotline fetched).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where one harness stands. The parts behind it are the desk's business:
/// the person sees one harness, behind or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub enum HarnessUpdateState {
    /// Being looked at now.
    Checking,
    UpToDate,
    /// A newer version of some part is published, and Hotline can fetch it.
    Available,
    /// Not every part could be checked; none is known to be behind.
    Unknown,
    /// An update is running.
    Updating,
    /// The last update finished. A teammate on the harness gets it at its
    /// next start.
    Updated,
    /// The last check or update failed; `detail` says why.
    Failed,
}

/// One harness on the Updates screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct HarnessUpdate {
    pub backend_id: String,
    pub name: String,
    pub state: HarnessUpdateState,
    /// The version installed, as the person knows the harness by: its CLI's,
    /// else its adapter's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The newest version of that same part, when it is newer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Whether the Update button runs anything: a harness Hotline can only
    /// check, not install, is shown without one.
    pub can_update: bool,
    /// One sentence: why a check or an update failed, or what an update
    /// changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Every harness this desk runs, with when they were last checked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct HarnessUpdates {
    pub harnesses: Vec<HarnessUpdate>,
    /// When the last full check finished, ms since the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
}
