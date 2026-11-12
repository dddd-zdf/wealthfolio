//! Fork: cached valuation history and observed 1D/1W portfolio values.
//!
//! The dashboard reads intraday samples captured by the scheduled refresh.

use std::sync::Arc;

use axum::{Extension, Json};
use serde::Deserialize;
use wealthfolio_core::portfolio::valuation::intraday::{
    observed_intraday_series, IntradayRange, IntradayValuationPoint,
};
use wealthfolio_core::portfolios::AccountScope;

use super::{get_historical_valuations_for_scope, resolve_scope};
use crate::api::{holdings::dto::HistoryFilterBody, shared::holdings_account_ids};
use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
};

#[derive(Deserialize)]
pub struct IntradayValuationBody {
    pub filter: AccountScope,
    /// "1D" or "1W".
    pub range: String,
}

/// POST /valuations/history/query, served from the short-lived cache shared
/// with performance summaries (dropped after every portfolio update).
pub async fn get_cached_historical_valuations_for_scope(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let key = crate::perf_cache::key("valuation-history", Arc::as_ptr(&state) as usize, &body);
    if let Some(hit) = crate::perf_cache::get(key) {
        return Ok(Json(hit));
    }
    let generation = crate::perf_cache::current_generation();
    let parsed: HistoryFilterBody = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("Invalid request body: {error}")))?;
    let Json(vals) = get_historical_valuations_for_scope(Extension(state), Json(parsed)).await?;
    let value = serde_json::to_value(&vals)
        .map_err(|error| ApiError::Internal(format!("Failed to serialize valuations: {error}")))?;
    crate::perf_cache::put(key, generation, value.clone());
    Ok(Json(value))
}

/// POST /valuations/intraday/query - observed values for the selected scope.
pub async fn get_intraday_valuations(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Vec<IntradayValuationPoint>>> {
    let parsed: IntradayValuationBody = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("Invalid request body: {error}")))?;
    let range = IntradayRange::parse(&parsed.range)
        .ok_or_else(|| ApiError::BadRequest(format!("Unsupported range: {}", parsed.range)))?;

    tokio::task::spawn_blocking(move || {
        let scope = resolve_scope(&parsed.filter, &state)?;
        let account_ids = holdings_account_ids(&state, &scope.account_ids)?;
        let since = chrono::Utc::now() - chrono::Duration::days(range.anchor_days());
        let samples = state.valuation_repository.get_intraday_samples(
            &account_ids,
            &scope.base_currency,
            since,
        )?;
        Ok(Json(observed_intraday_series(&account_ids, samples, range)))
    })
    .await
    .map_err(|_| ApiError::Internal("Could not load intraday valuations".into()))?
}
