//! Fork: the router `main` serves, with the default profile started at boot.

use super::{app_router_with_profiles, Config, Router};

/// Like [`super::app_router_from_config`], but starts the default profile before
/// any request is served, so background work begins at boot without racing a
/// visitor's own startup of the same profile.
pub async fn serving_router_from_config(config: &Config) -> anyhow::Result<Router> {
    let profiles = crate::profiles::WebProfiles::open(config).await?;
    profiles.start_default_profile().await;
    let auth = profiles.auth_state();
    app_router_with_profiles(profiles, auth, config)
}
