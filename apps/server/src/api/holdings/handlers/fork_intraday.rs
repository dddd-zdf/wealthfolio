//! Fork: cached valuation history and the 1D/1W intraday portfolio curve.
//!
//! Both responses live in `perf_cache`, so a portfolio update drops them.
//! The upstream valuation-history handler stays unchanged; these wrappers
//! call it directly.

use std::sync::Arc;

use axum::{Extension, Json};
use serde::Deserialize;
use wealthfolio_core::portfolio::valuation::intraday::{intraday_valuations, IntradayRange};
use wealthfolio_core::portfolios::AccountScope;
use wealthfolio_core::utils::time_utils::{parse_user_timezone_or_default, user_today};

use super::{get_historical_valuations_for_scope, load_holdings_for_filter};
use crate::api::holdings::dto::HistoryFilterBody;
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

/// Intraday prices move every few minutes; keep each curve this long. Like
/// the other cached results, a portfolio update drops it.
const INTRADAY_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

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

/// POST /valuations/intraday/query - 1D/1W portfolio curve from live
/// intraday bars, anchored to the stored daily totals.
pub async fn get_intraday_valuations(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let key = crate::perf_cache::key("intraday", Arc::as_ptr(&state) as usize, &body);
    if let Some(hit) = crate::perf_cache::get(key) {
        return Ok(Json(hit));
    }
    let generation = crate::perf_cache::current_generation();
    let parsed: IntradayValuationBody = serde_json::from_slice(&body)
        .map_err(|error| ApiError::BadRequest(format!("Invalid request body: {error}")))?;
    let range = IntradayRange::parse(&parsed.range)
        .ok_or_else(|| ApiError::BadRequest(format!("Unsupported range: {}", parsed.range)))?;

    let tz = parse_user_timezone_or_default(&state.timezone()?);
    let today = user_today(tz);
    let holdings = load_holdings_for_filter(state.as_ref(), &parsed.filter, false).await?;
    let Json(daily) = get_historical_valuations_for_scope(
        Extension(state.clone()),
        Json(HistoryFilterBody {
            filter: parsed.filter,
            start_date: Some(
                (today - chrono::Duration::days(range.anchor_days()))
                    .format("%Y-%m-%d")
                    .to_string(),
            ),
            end_date: Some(today.format("%Y-%m-%d").to_string()),
        }),
    )
    .await?;
    let points =
        intraday_valuations(&holdings, &daily, state.quote_service.as_ref(), range, tz).await?;
    let value = serde_json::to_value(&points).map_err(|error| {
        ApiError::Internal(format!("Failed to serialize intraday values: {error}"))
    })?;
    crate::perf_cache::put_with_ttl(key, generation, value.clone(), INTRADAY_CACHE_TTL);
    Ok(Json(value))
}
