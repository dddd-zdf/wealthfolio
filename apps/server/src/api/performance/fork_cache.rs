//! Fork: dashboard performance responses served from `perf_cache`.
//!
//! Common dashboard ranges use one cached set per reporting scope and day.
//! Custom ranges and other callers use the ordinary single-request cache.

use std::{sync::Arc, time::Instant};

use super::{
    parse_date_optional, AccountScope, PerfBody, PerformancePeriod, PerformanceSummariesBody,
    PerformanceSummaryProfile, ScopedPerformance,
};
use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
};
use axum::{Extension, Json};
use chrono::{Datelike, Days, Months, NaiveDate};
use wealthfolio_core::utils::time_utils::{parse_user_timezone_or_default, user_today};

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
    if let Some(filter) = dashboard_preset_scope(&parsed) {
        let today = dashboard_today(&state);
        let periods = dashboard_period_ranges(today);
        let requested = PerformancePeriod {
            start_date: parse_date_optional(parsed.start_date.clone(), "startDate")?,
            end_date: parse_date_optional(parsed.end_date.clone(), "endDate")?,
        };
        if let Some(index) = periods.iter().position(|period| period == &requested) {
            let cached = dashboard_summaries(&state, filter, today, periods)
                .await
                .ok()
                .and_then(|mut results| results.get_mut(index).map(serde_json::Value::take));
            if let Some(value) = cached {
                return Ok(Json(value));
            }
            // A failed preparation can still leave this particular window readable.
            tracing::debug!("Dashboard preset preparation skipped for a single-period read");
        }
    }
    let key = summary_cache_key(&state, &parsed);
    crate::perf_cache::get_or_calculate(key, async {
        let Json(result) =
            super::calculate_performance_summary(Extension(state), Json(parsed)).await?;
        serde_json::to_value(&result)
            .map_err(|error| ApiError::Internal(format!("Failed to serialize summary: {error}")))
    })
    .await
    .map(Json)
}

/// POST /performance/summaries
pub(super) async fn get_performance_summaries(
    Extension(state): Extension<Arc<AppState>>,
    body: axum::body::Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let key = crate::perf_cache::key("summaries", Arc::as_ptr(&state) as usize, &body);
    crate::perf_cache::get_or_calculate(key, async {
        let parsed: PerformanceSummariesBody = parse_cached_body(&body)?;
        let Json(results) =
            super::get_performance_summaries(Extension(state), Json(parsed)).await?;
        serde_json::to_value(&results)
            .map_err(|error| ApiError::Internal(format!("Failed to serialize summaries: {error}")))
    })
    .await
    .map(Json)
}

fn dashboard_preset_scope(body: &PerfBody) -> Option<&AccountScope> {
    if body.item_type != "account" || body.profile != Some(PerformanceSummaryProfile::Dashboard) {
        return None;
    }
    match body.filter.as_ref()? {
        scope @ (AccountScope::All | AccountScope::Portfolio { .. }) => Some(scope),
        _ => None,
    }
}

fn dashboard_period_ranges(today: NaiveDate) -> Vec<PerformancePeriod> {
    let months = |count| today.checked_sub_months(Months::new(count));
    [
        today.checked_sub_days(Days::new(7)),
        months(1),
        months(3),
        months(6),
        today.with_ordinal(1),
        months(12),
        months(60),
    ]
    .into_iter()
    .flatten()
    .map(|start| PerformancePeriod {
        start_date: Some(start),
        end_date: Some(today),
    })
    .chain([PerformancePeriod {
        start_date: None,
        end_date: None,
    }])
    .collect()
}

fn dashboard_today(state: &AppState) -> NaiveDate {
    let timezone = state
        .timezone
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    user_today(parse_user_timezone_or_default(&timezone))
}

async fn dashboard_summaries(
    state: &Arc<AppState>,
    filter: &AccountScope,
    today: NaiveDate,
    periods: Vec<PerformancePeriod>,
) -> ApiResult<serde_json::Value> {
    let profile_id = state.profile_binding.get().map(|(_, id)| *id);
    let timezone = state
        .timezone
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let generation = crate::perf_cache::current_generation();
    let key = crate::perf_cache::key(
        "dashboard-presets",
        Arc::as_ptr(state) as usize,
        serde_json::json!([profile_id, today, timezone, state.base_currency()?, filter])
            .to_string()
            .as_bytes(),
    );
    let results = crate::perf_cache::get_or_calculate(key, async {
        let started = Instant::now();
        let scope = ScopedPerformance::resolve(state, filter)?;
        let mut results = if scope.account_ids.is_empty() {
            periods.iter().map(|period| scope.empty(period)).collect()
        } else {
            state
                .performance_service
                .calculate_dashboard_summaries_for_accounts(
                    scope.resolved.scope_id.clone(),
                    scope.account_ids.clone(),
                    periods,
                )
                .await?
        };
        if !scope.account_ids.is_empty() {
            for result in &mut results {
                scope.add_exclusion_warning(result);
            }
        }
        tracing::debug!(
            presets = results.len(),
            elapsed_ms = started.elapsed().as_millis(),
            "Prepared dashboard performance presets"
        );
        serde_json::to_value(results).map_err(|error| {
            ApiError::Internal(format!("Failed to serialize dashboard presets: {error}"))
        })
    })
    .await?;
    if generation != crate::perf_cache::current_generation() {
        return Err(ApiError::Internal(
            "Portfolio changed while preparing dashboard presets".into(),
        ));
    }
    Ok(results)
}

pub(crate) async fn warm_dashboard_summaries(state: &Arc<AppState>) {
    let today = dashboard_today(state);
    let started = Instant::now();
    let generation = crate::perf_cache::current_generation();
    let mut filters = vec![AccountScope::All];
    match state.portfolio_service.list_portfolios() {
        Ok(portfolios) => {
            filters.extend(
                portfolios
                    .into_iter()
                    .map(|portfolio| AccountScope::Portfolio {
                        portfolio_id: portfolio.id,
                    }),
            )
        }
        Err(_) => tracing::debug!("Saved portfolios unavailable during dashboard warm-up"),
    }
    for filter in filters {
        if generation != crate::perf_cache::current_generation() {
            break;
        }
        if dashboard_summaries(state, &filter, today, dashboard_period_ranges(today))
            .await
            .is_err()
        {
            tracing::debug!("Dashboard warm-up skipped a scope");
        }
    }
    tracing::info!(
        "Warmed dashboard performance summaries in {:?}",
        started.elapsed()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_warm_up_covers_every_interval_the_dashboard_offers() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 16).expect("valid date");
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d);
        assert_eq!(
            dashboard_period_ranges(today)
                .into_iter()
                .map(|period| (period.start_date, period.end_date))
                .collect::<Vec<_>>(),
            vec![
                (day(2026, 10, 9), Some(today)),
                (day(2026, 9, 16), Some(today)),
                (day(2026, 7, 16), Some(today)),
                (day(2026, 4, 16), Some(today)),
                (day(2026, 1, 1), Some(today)),
                (day(2025, 10, 16), Some(today)),
                (day(2021, 10, 16), Some(today)),
                (None, None),
            ]
        );
    }
}
