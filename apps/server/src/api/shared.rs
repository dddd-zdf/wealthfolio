use std::sync::Arc;

use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
};
use anyhow::anyhow;
use chrono::NaiveDate;
use wealthfolio_core::{
    accounts::{account_supports_portfolio_scope, AccountPurpose, AccountServiceTrait},
    portfolio::coordinator::{PortfolioJobRequest, RetryPolicy},
    quotes::MarketSyncMode,
};

// ============================================================================
// Date Parsing Utilities
// ============================================================================

/// Parse a required date string in YYYY-MM-DD format.
pub fn parse_date(date_str: &str, field_name: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("Invalid {}: {}", field_name, e)))
}

/// Parse an optional date string in YYYY-MM-DD format.
pub fn parse_date_optional(
    date_str: Option<String>,
    field_name: &str,
) -> Result<Option<NaiveDate>, ApiError> {
    date_str.map(|s| parse_date(&s, field_name)).transpose()
}

pub fn holdings_account_ids(state: &AppState, account_ids: &[String]) -> ApiResult<Vec<String>> {
    Ok(state
        .account_service
        .get_accounts_by_ids(account_ids)?
        .into_iter()
        .filter(|account| account_supports_portfolio_scope(account, AccountPurpose::Holdings))
        .map(|account| account.id)
        .collect())
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRequestBody {
    pub account_ids: Option<Vec<String>>,
    #[serde(default)]
    pub market_sync_mode: MarketSyncMode,
    /// Rebuild from the first activity even when the accounts are fresh.
    #[serde(default)]
    pub force_full: bool,
}

impl PortfolioRequestBody {
    pub fn into_config(self) -> PortfolioJobConfig {
        PortfolioJobConfig {
            account_ids: self.account_ids,
            market_sync_mode: self.market_sync_mode,
            force_full: self.force_full,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PortfolioJobConfig {
    pub account_ids: Option<Vec<String>>,
    pub market_sync_mode: MarketSyncMode,
    pub force_full: bool,
}

/// Enqueue a background portfolio job that will publish SSE events as it runs.
pub fn enqueue_portfolio_job(state: Arc<AppState>, config: PortfolioJobConfig) {
    tokio::spawn(async move {
        if let Err(err) = process_portfolio_job(state, config).await {
            tracing::error!("Portfolio job failed: {}", err);
        }
    });
}

/// Trigger a lightweight portfolio update (no full recalculation) similar to Tauri defaults.
/// Uses MarketSyncMode::None - no market sync, just recalculation.
pub fn trigger_lightweight_portfolio_update(state: Arc<AppState>) {
    enqueue_portfolio_job(
        state,
        PortfolioJobConfig {
            account_ids: None,
            market_sync_mode: MarketSyncMode::None,
            force_full: false,
        },
    );
}

/// Trigger a full portfolio recalculation impacting every account.
/// Uses MarketSyncMode::None - no market sync, just recalculation.
pub fn trigger_full_portfolio_recalc(state: Arc<AppState>) {
    enqueue_portfolio_job(
        state,
        PortfolioJobConfig {
            account_ids: None,
            market_sync_mode: MarketSyncMode::None,
            force_full: false,
        },
    );
}

/// Trigger a full portfolio recalculation that also syncs the given assets'
/// market data. Used when a provider-backed FX pair is added so its real rate
/// is fetched immediately instead of waiting for the periodic sync (#1143).
pub fn trigger_portfolio_recalc_with_asset_sync(state: Arc<AppState>, asset_ids: Vec<String>) {
    enqueue_portfolio_job(
        state,
        PortfolioJobConfig {
            account_ids: None,
            market_sync_mode: MarketSyncMode::Incremental {
                asset_ids: Some(asset_ids),
            },
            ..PortfolioJobConfig::default()
        },
    );
}

pub(crate) use super::goals::emit_goal_summaries_updated;
pub(crate) use crate::fork_policy::ServerJobObserver;

impl PortfolioJobConfig {
    /// The coordinator request this config describes.
    pub fn into_request(self) -> PortfolioJobRequest {
        PortfolioJobRequest {
            account_ids: self.account_ids,
            market_sync: self.market_sync_mode,
            force_full: self.force_full,
        }
    }
}

pub async fn process_portfolio_job(
    state: Arc<AppState>,
    config: PortfolioJobConfig,
) -> ApiResult<()> {
    let observer = ServerJobObserver::new(&state);
    let report = state
        .portfolio_coordinator
        .run_job_with_retry(config.into_request(), &observer, RetryPolicy::default())
        .await
        .map_err(|err| crate::error::ApiError::Anyhow(anyhow!(err.to_string())))?;
    if let Some(Err(message)) = &report.market_sync {
        tracing::error!("Market data sync failed before the rebuild: {message}");
    }
    Ok(())
}

/// Portfolio update at server start (architecture §3.3): sync market data,
/// then bring whatever is stale up to date, so a web client finds it current.
pub fn spawn_portfolio_update(state: Arc<AppState>) {
    tokio::spawn(async move {
        let observer = ServerJobObserver::new(&state);
        state
            .portfolio_coordinator
            .update_all(MarketSyncMode::Incremental { asset_ids: None }, &observer)
            .await;
    });
}
