//! The Updates screen's harnesses: where each stands, a check, an update
//! (`driver::acp::updates`). The person's action, on the desk's own
//! authority: a companion seat reaches none of it.

use super::*;
use crate::contract::{HarnessUpdate, HarnessUpdates};

impl Room {
    /// Where each harness stands, as last checked. A check six hours old, or
    /// none yet, starts one in the background; this answers at once.
    pub fn harness_updates(self: &Arc<Self>) -> HarnessUpdates {
        if self.harness_updates.due(now_ms()) {
            let room = self.clone();
            tokio::spawn(async move {
                room.harness_updates_check().await;
            });
        }
        self.harness_updates.view()
    }

    /// Checks every harness now.
    pub async fn harness_updates_check(&self) -> HarnessUpdates {
        self.harness_updates
            .check(self.log.root(), &self.backends_in_use(), now_ms())
            .await
    }

    /// Updates one harness. A running session keeps the version it started
    /// with; its teammate gets the new one at its next start.
    pub async fn harness_update(&self, backend_id: &str) -> Result<HarnessUpdate, String> {
        self.harness_updates
            .update(self.log.root(), backend_id)
            .await
    }

    /// The harnesses the roster's teammates are set to.
    fn backends_in_use(&self) -> Vec<String> {
        let mut used: Vec<String> = room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.backend_id)
            .collect();
        used.sort();
        used.dedup();
        used
    }

    /// Checks and updates harnesses on a scripted machine.
    #[cfg(test)]
    pub(crate) fn harness_world(&self, world: Arc<dyn crate::driver::acp::updates::World>) {
        self.harness_updates.use_world(world);
    }
}
