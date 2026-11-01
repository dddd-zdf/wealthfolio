//! Fork: profile startup that doesn't wait for, or get cancelled by, a visitor.

use std::sync::Arc;

use super::WebProfiles;

impl WebProfiles {
    /// Starts the unlocked default profile so its background price updates
    /// run without waiting for a first visit. Failures are left for that
    /// visit to retry and report.
    pub async fn start_default_profile(self: &Arc<Self>) {
        let Ok(id) = self.registry.default_id() else {
            return;
        };
        let unlocked = self
            .registry
            .profile(id)
            .is_ok_and(|profile| !profile.lock_enabled);
        if unlocked {
            if let Err((status, _)) = self.runtime(id).await {
                tracing::warn!(%status, "Default profile startup deferred to first request");
            }
        }
    }
}
