//! Fork: dashboard performance responses served from `perf_cache`.
//!
//! The wrappers parse the raw body, serve a cached response when the cache
//! generation still matches, and otherwise call the unchanged upstream
//! handler. `warm_dashboard_summaries` precomputes the dashboard's periods
//! after edits (see `perf_cache::register_warm_target`).

use std::{sync::Arc, time::Instant};

use super::{AccountScope, PerfBody, PerformanceSummariesBody, PerformanceSummaryProfile};
use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
};
use axum::{Extension, Json};
use chrono::{Datelike, Days, Months, NaiveDate};

fn parse_cached_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    serde_json::from_slice(body)
        .map_err(|error| ApiError::BadRequest(format!("Invalid request body: {error}")))
}

fn summary_cache_key(state: &Arc<AppState>, body: &PerfBody) -> u64 {
    let canonical = serde_json::json!({
        "itemType": body.item_type,
        "itemId": body.item_id,
        "startDate": body.start_date,
        "endDate": body.end_date,
        "trackingMode": body.tracking_mode,
        "filter": body.filter,
        "profile": body.profile,
    })
    .to_string();
    crate::perf_cache::key("summary", Arc::as_ptr(state) as usize, canonical.as_bytes())
}

/// POST /performance/summary
pub(super) async fn calculate_performance_summary(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let parsed: PerfBody = parse_cached_body(&body)?;
    let key = summary_cache_key(&state, &parsed);
    if let Some(hit) = crate::perf_cache::get(key) {
        return Ok(Json(hit));
    }
    let generation = crate::perf_cache::current_generation();
    let Json(result) = super::calculate_performance_summary(Extension(state), Json(parsed)).await?;
    let value = serde_json::to_value(&result)
        .map_err(|error| ApiError::Internal(format!("Failed to serialize summary: {error}")))?;
    crate::perf_cache::put(key, generation, value.clone());
    Ok(Json(value))
}

/// POST /performance/summaries
pub(super) async fn get_performance_summaries(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let key = crate::perf_cache::key("summaries", Arc::as_ptr(&state) as usize, &body);
    if let Some(hit) = crate::perf_cache::get(key) {
        return Ok(Json(hit));
    }
    let generation = crate::perf_cache::current_generation();
    let parsed: PerformanceSummariesBody = parse_cached_body(&body)?;
    let Json(results) = super::get_performance_summaries(Extension(state), Json(parsed)).await?;
    let value = serde_json::to_value(&results)
        .map_err(|error| ApiError::Internal(format!("Failed to serialize summaries: {error}")))?;
    crate::perf_cache::put(key, generation, value.clone());
    Ok(Json(value))
}

fn dashboard_period_ranges(today: NaiveDate) -> Vec<(Option<NaiveDate>, Option<NaiveDate>)> {
    let months = |count| today.checked_sub_months(Months::new(count));
    vec![
        (today.checked_sub_days(Days::new(1)), Some(today)),
        (today.checked_sub_days(Days::new(7)), Some(today)),
        (months(1), Some(today)),
        (months(3), Some(today)),
        (months(6), Some(today)),
        (today.with_ordinal(1), Some(today)),
        (months(12), Some(today)),
        (months(60), Some(today)),
        (None, None),
    ]
}

pub(crate) async fn warm_dashboard_summaries(state: &Arc<AppState>) {
    let today = {
        let timezone = state
            .timezone
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        wealthfolio_core::utils::time_utils::user_today(
            wealthfolio_core::utils::time_utils::parse_user_timezone_or_default(&timezone),
        )
    };
    let started = Instant::now();
    for (start, end) in dashboard_period_ranges(today) {
        let body = PerfBody {
            item_type: "account".to_string(),
            item_id: "portfolio:all".to_string(),
            start_date: start.map(|date| date.format("%Y-%m-%d").to_string()),
            end_date: end.map(|date| date.format("%Y-%m-%d").to_string()),
            tracking_mode: None,
            filter: Some(AccountScope::All),
            profile: Some(PerformanceSummaryProfile::Dashboard),
        };
        let key = summary_cache_key(state, &body);
        if crate::perf_cache::get(key).is_some() {
            continue;
        }
        let generation = crate::perf_cache::current_generation();
        match super::calculate_performance_summary(Extension(state.clone()), Json(body)).await {
            Ok(Json(result)) => {
                if let Ok(value) = serde_json::to_value(&result) {
                    crate::perf_cache::put(key, generation, value);
                }
            }
            Err(error) => tracing::debug!("Dashboard summary warm-up skipped a period: {error}"),
        }
    }
    tracing::info!(
        "Warmed dashboard performance summaries in {:?}",
        started.elapsed()
    );
}
