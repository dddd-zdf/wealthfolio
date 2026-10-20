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
    accounts::AccountServiceTrait,
    portfolio::coordinator::{AccountFailure, JobObserver, RetryPolicy},
    portfolio::projection::{MarkerScope, ProjectionStoreTrait},
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

/// Reuse the ordinary startup job slot after an interrupted policy rebuild.
pub(crate) async fn spawn_startup_update(
    state: Arc<AppState>,
    projections: Arc<dyn ProjectionStoreTrait>,
) -> ApiResult<()> {
    let policy_pending = projections
        .pending_markers()?
        .iter()
        .any(|marker| marker.scope == MarkerScope::All);
    let base = state.base_currency()?;
    let accounts: Vec<_> = state
        .account_service
        .get_non_archived_accounts()?
        .into_iter()
        .map(|account| account.id)
        .collect();
    // A fresh profile has no historical amounts to protect. Its initial policy
    // marker must not make empty-scope reads depend on background job timing.
    if accounts.is_empty() {
        crate::api::shared::spawn_portfolio_update(state);
        return Ok(());
    }
    let valuations = state.valuation_service.clone();
    // A per-account failure can consume @all. Check older windows as well as
    // the latest row so a partial rebuild cannot hide mixed currencies.
    let mismatched =
        tokio::task::spawn_blocking(move || -> wealthfolio_core::errors::Result<bool> {
            for account in accounts {
                let rows = valuations.get_historical_valuations(&account, None, None)?;
                if rows.iter().any(|row| row.base_currency != base) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await
        .map_err(|_| {
            ApiError::Internal("Could not check historical currencies at startup.".into())
        })??;
    if !policy_pending && !mismatched {
        crate::api::shared::spawn_portfolio_update(state);
        return Ok(());
    }
    state.currency_transition.begin();
    tokio::spawn(async move {
        let _settings = state.currency_transition.settings.lock().await;
        state.currency_transition.begin();
        let result = rebuild(&state).await;
        state.currency_transition.finish(result.is_ok());
        if result.is_ok() {
            publish_completion(&state).await;
        }
    });
    Ok(())
}

async fn publish_completion(state: &AppState) {
    state.health_service.clear_cache().await;
    state.event_bus.publish(ServerEvent::with_payload(
        PORTFOLIO_UPDATE_COMPLETE,
        serde_json::json!({ "baseCurrencyChanged": true }),
    ));
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
                publish_completion(&state).await;
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
        "/snapshots/holdings",
        "/utilities/export",
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
    guard_transition(&state.currency_transition, request, next).await
}

async fn guard_transition(
    transition: &CurrencyTransition,
    request: Request,
    next: Next,
) -> Response {
    let path = request
        .uri()
        .path()
        .strip_prefix("/api/v1")
        .unwrap_or(request.uri().path());
    let settings_read = path == "/settings" && request.method() == axum::http::Method::GET;
    if !settings_read && !financial_path(path) {
        return next.run(request).await;
    }
    let revision = transition.revision.load(Ordering::SeqCst);
    if !transition.accepts(revision) {
        return unavailable();
    }
    let response = next.run(request).await;
    if transition.accepts(revision) {
        response
    } else {
        unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware, routing::get, Router};
    use tower::ServiceExt;

    fn test_router(transition: Arc<CurrencyTransition>) -> Router {
        Router::new()
            .route(
                "/valuations/history/query",
                get(|| async { "old-currency amounts" }),
            )
            .route(
                "/settings",
                get(|| async { "settings" }).put(|| async { "retry accepted" }),
            )
            .layer(middleware::from_fn(move |request, next| {
                let transition = transition.clone();
                async move { guard_transition(&transition, request, next).await }
            }))
    }

    #[tokio::test]
    async fn actual_read_routes_block_during_rebuild_but_settings_retry_stays_available() {
        let transition = Arc::new(CurrencyTransition::default());
        let router = test_router(transition.clone());
        transition.begin();
        for path in ["/valuations/history/query", "/settings"] {
            let response = router
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
            assert_eq!(response.headers()[header::RETRY_AFTER], "2");
        }
        let retry = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry.status(), StatusCode::OK);
        transition.finish(true);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/valuations/history/query")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

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
            "/snapshots/holdings",
            "/utilities/export/holdings/csv",
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
