//! Fork: one asset's live 1D/1W price bars for the asset page chart.
//!
//! Same bars and cache as the dashboard's intraday curve
//! (`api/holdings/handlers/fork_intraday.rs`), for a single asset.

use std::sync::Arc;

use axum::{extract::Query, Extension, Json};
use serde::Deserialize;
use wealthfolio_core::portfolio::valuation::intraday::IntradayRange;

use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntradayQuotesQuery {
    pub asset_id: String,
    /// "1D" or "1W".
    pub range: String,
}

/// Intraday prices move every few minutes; keep each response this long.
const INTRADAY_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// GET /market-data/quotes/intraday - bars oldest first, empty when no
/// provider has intraday data for the asset.
pub async fn get_intraday_quotes(
    Extension(state): Extension<Arc<AppState>>,
    Query(q): Query<IntradayQuotesQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let range = IntradayRange::parse(&q.range)
        .ok_or_else(|| ApiError::BadRequest(format!("Unsupported range: {}", q.range)))?;
    let request = format!("{}\n{}", q.asset_id, q.range);
    let key = crate::perf_cache::key(
        "intraday-quotes",
        Arc::as_ptr(&state) as usize,
        request.as_bytes(),
    );
    if let Some(hit) = crate::perf_cache::get(key) {
        return Ok(Json(hit));
    }
    let generation = crate::perf_cache::current_generation();
    let (interval, window) = range.provider_args();
    let mut bars = state
        .quote_service
        .get_intraday_quotes(std::slice::from_ref(&q.asset_id), interval, window)
        .await?
        .remove(&q.asset_id)
        .unwrap_or_default();
    bars.retain(|bar| bar.close > rust_decimal::Decimal::ZERO);
    bars.sort_by_key(|bar| bar.timestamp);
    let value = serde_json::to_value(&bars)
        .map_err(|error| ApiError::Internal(format!("Failed to serialize quotes: {error}")))?;
    crate::perf_cache::put_with_ttl(key, generation, value.clone(), INTRADAY_CACHE_TTL);
    Ok(Json(value))
}
