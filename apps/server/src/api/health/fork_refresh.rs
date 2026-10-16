//! Fork: `/health/status` serves the cached status, even once it has aged
//! out, and refreshes it in the background instead of making the request
//! wait for a full check. Data changes clear the cache outright (and trigger a
//! warm-up), so an aged entry only means time passed, not that data changed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::{http::HeaderMap, Extension, Json};
use wealthfolio_core::health::HealthStatus;

use crate::{error::ApiResult, main_lib::AppState};

/// GET /health/status
pub(super) async fn get_health_status(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<HealthStatus>> {
    if let Some(status) = state.health_service.get_cached_status().await {
        if status.is_stale {
            spawn_health_refresh(state.clone());
        }
        return Ok(Json(status));
    }
    super::get_health_status(Extension(state), headers).await
}

static HEALTH_REFRESHING: AtomicBool = AtomicBool::new(false);

fn spawn_health_refresh(state: Arc<AppState>) {
    if HEALTH_REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        warm_health_status(&state).await;
        HEALTH_REFRESHING.store(false, Ordering::SeqCst);
    });
}

/// Recompute and cache the health status ahead of the next page view.
pub(crate) async fn warm_health_status(state: &Arc<AppState>) {
    let base_currency = match state.base_currency() {
        Ok(base_currency) => base_currency,
        Err(error) => {
            tracing::debug!("Background health check skipped: {error}");
            return;
        }
    };
    if let Err(error) = super::run_health_checks_internal(state, &base_currency, None).await {
        tracing::debug!("Background health check failed: {error}");
    }
}
