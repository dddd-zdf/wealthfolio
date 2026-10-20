//! Web-only currency transitions. The coordinator owns calculation and persistence.
//! Keep old-currency responses out of clients while its windows are being replaced.

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use wealthfolio_core::{
    portfolio::coordinator::{AccountFailure, JobObserver, RetryPolicy},
    quotes::{MarketSyncMode, SyncResult},
    settings::{Settings, SettingsUpdate},
};

use crate::{
    api::shared::{PortfolioJobConfig, ServerJobObserver},
    error::{ApiError, ApiResult},
    events::{ServerEvent, PORTFOLIO_UPDATE_COMPLETE},
    main_lib::AppState,
};

#[derive(Default)]
pub(crate) struct CurrencyTransition {
    settings: tokio::sync::Mutex<()>,
    blocked: AtomicBool,
    revision: AtomicU64,
}

// Publish completion only after the read guard is released, including on retries.
struct CurrencyObserver(ServerJobObserver);

impl JobObserver for CurrencyObserver {
    fn market_sync_started(&self) {
        self.0.market_sync_started();
    }
    fn market_sync_completed(&self, result: &SyncResult) {
        self.0.market_sync_completed(result);
    }
    fn market_sync_failed(&self, message: &str) {
        self.0.market_sync_failed(message);
    }
    fn update_started(&self) {
        self.0.update_started();
    }
    fn update_failed(&self, failure: &AccountFailure) {
        self.0.update_failed(failure);
    }
}

impl CurrencyTransition {
    fn begin(&self) {
        self.blocked.store(true, Ordering::SeqCst);
        self.revision.fetch_add(1, Ordering::SeqCst);
        crate::perf_cache::invalidate();
    }

    fn finish(&self, success: bool) {
        crate::perf_cache::invalidate();
        self.blocked.store(!success, Ordering::SeqCst);
    }

    fn accepts(&self, revision: u64) -> bool {
        !self.blocked.load(Ordering::SeqCst) && self.revision.load(Ordering::SeqCst) == revision
    }
}

fn rebuild_config() -> PortfolioJobConfig {
    PortfolioJobConfig {
        account_ids: None,
        market_sync_mode: MarketSyncMode::BackfillHistory {
            asset_ids: None,
            days: wealthfolio_core::quotes::DEFAULT_HISTORY_DAYS,
        },
        force_full: true,
    }
}

pub(super) async fn rebuild(state: &Arc<AppState>) -> ApiResult<()> {
    let report = state
        .portfolio_coordinator
        .run_job_with_retry(
            rebuild_config().into_request(),
            &CurrencyObserver(ServerJobObserver::new(state)),
            RetryPolicy::default(),
        )
        .await?;
    if !report.failures.is_empty() {
        return Err(ApiError::Internal(
            "Currency history rebuild failed. Review data issues, then save the base currency again to retry."
                .to_string(),
        ));
    }
    Ok(())
}

/// Keep work alive if the browser disconnects or its request times out. Settings
/// success means the complete history is ready, rather than merely queued.
pub(super) async fn update_settings(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<SettingsUpdate>,
) -> ApiResult<Json<Settings>> {
    tokio::spawn(async move {
        let _settings = state.currency_transition.settings.lock().await;
        let previous = state.base_currency()?;
        let retry = state.currency_transition.blocked.load(Ordering::SeqCst);
        let changed = payload
            .base_currency
            .as_ref()
            .is_some_and(|currency| currency != &previous);
        let transition = changed || (retry && payload.base_currency.is_some());
        if transition {
            state.currency_transition.begin();
        }
        let result = super::update_settings(Extension(state.clone()), Json(payload)).await;
        // The upstream handler only rebuilds when the value changed. An earlier
        // failure persisted the preference; saving it again must still rebuild.
        let result = match result {
            Ok(settings) if transition && !changed => rebuild(&state).await.map(|()| settings),
            result => result,
        };
        if transition {
            // Validation can fail before the preference is persisted. In that
            // case the old history remains valid and must stay readable.
            let unchanged = !retry && state.base_currency().is_ok_and(|base| base == previous);
            state
                .currency_transition
                .finish(result.is_ok() || unchanged);
            if result.is_ok() {
                state.health_service.clear_cache().await;
                state.event_bus.publish(ServerEvent::with_payload(
                    PORTFOLIO_UPDATE_COMPLETE,
                    serde_json::json!({ "baseCurrencyChanged": true }),
                ));
            }
        }
        result
    })
    .await
    .map_err(|_| {
        ApiError::Internal("Settings update interrupted; retry the base currency change.".into())
    })?
}

fn financial_path(path: &str) -> bool {
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    [
        "/holdings",
        "/valuations",
        "/performance",
        "/allocations",
        "/net-worth",
        "/income",
        "/goals",
        "/limits",
        "/spending",
    ]
    .iter()
    .any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn unavailable() -> Response {
    (
        StatusCode::CONFLICT,
        [(header::RETRY_AFTER, "2")],
        Json(serde_json::json!({
            "code": 409,
            "message": "Base currency history is rebuilding or needs repair. Retry after completion; if rebuilding failed, review data issues and save the base currency again."
        })),
    )
        .into_response()
}

pub(crate) async fn guard_reads(
    Extension(state): Extension<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if !financial_path(request.uri().path()) {
        return next.run(request).await;
    }
    let revision = state.currency_transition.revision.load(Ordering::SeqCst);
    if !state.currency_transition.accepts(revision) {
        return unavailable();
    }
    let response = next.run(request).await;
    if state.currency_transition.accepts(revision) {
        response
    } else {
        unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_rebuild_covers_every_account_from_genesis() {
        let request = rebuild_config().into_request();
        assert!(request.force_full);
        assert!(request.account_ids.is_none());
        assert!(matches!(
            request.market_sync,
            MarketSyncMode::BackfillHistory {
                asset_ids: None,
                ..
            }
        ));
    }

    #[test]
    fn responses_spanning_a_transition_are_rejected_even_after_success() {
        let transition = CurrencyTransition::default();
        let revision = transition.revision.load(Ordering::SeqCst);
        assert!(transition.accepts(revision));
        transition.begin();
        assert!(!transition.accepts(revision));
        transition.finish(true);
        assert!(!transition.accepts(revision));
        assert!(transition.accepts(transition.revision.load(Ordering::SeqCst)));
    }

    #[test]
    fn failure_keeps_financial_reads_blocked_until_a_successful_retry() {
        let transition = CurrencyTransition::default();
        transition.begin();
        transition.finish(false);
        assert!(!transition.accepts(transition.revision.load(Ordering::SeqCst)));
        transition.begin();
        transition.finish(true);
        assert!(transition.accepts(transition.revision.load(Ordering::SeqCst)));
    }

    #[test]
    fn read_guard_covers_aggregate_and_single_account_routes() {
        for path in [
            "/holdings/query",
            "/api/v1/valuations/history/query",
            "/performance/summary",
            "/spending/summary",
            "/net-worth",
        ] {
            assert!(financial_path(path), "{path}");
        }
        for path in [
            "/settings",
            "/events/stream",
            "/exchange-rates",
            "/health/status",
        ] {
            assert!(!financial_path(path), "{path}");
        }
    }
}
