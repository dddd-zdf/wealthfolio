//! Fork: profile startup that doesn't wait for, or get cancelled by, a visitor.

use std::sync::Arc;

use uuid::Uuid;

use super::{failure, AppState, Result, WebProfiles};

impl WebProfiles {
    /// Starts the unlocked default profile so its background price updates
    /// run without waiting for a first visit. Failures are left for that
    /// visit to retry and report.
    pub async fn start_default_profile(&self) {
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

    /// [`WebProfiles::runtime`], detached so a client timeout doesn't cancel a
    /// slow cold start.
    pub(super) async fn runtime_detached(self: &Arc<Self>, id: Uuid) -> Result<Arc<AppState>> {
        let root = self.clone();
        tokio::spawn(async move { root.runtime(id).await })
            .await
            .map_err(failure)?
    }
}
