//! Small deployment policies kept outside the upstream portfolio coordinator.
//!
//! The coordinator remains the only calculation path. This module only adapts
//! its observer events for unattended web-server jobs and selects the fork's
//! refresh cadence for the always-on ARM64 deployment.

mod intraday_bootstrap;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use serde_json::json;
use wealthfolio_core::{
    goals::GoalServiceTrait,
    health::HealthServiceTrait,
    portfolio::coordinator::{AccountFailure, JobObserver},
    quotes::{MarketSyncMode, SyncResult},
    utils::time_utils::{parse_user_timezone, parse_user_timezone_or_default},
};

use crate::{
    events::{
        EventBus, MarketSyncResult, ServerEvent, MARKET_SYNC_COMPLETE, MARKET_SYNC_ERROR,
        MARKET_SYNC_START, PORTFOLIO_UPDATE_COMPLETE, PORTFOLIO_UPDATE_ERROR,
        PORTFOLIO_UPDATE_START,
    },
    main_lib::AppState,
};

/// How coordinator progress is presented to connected web clients.
pub(crate) struct ServerJobObserver {
    event_bus: EventBus,
    health_service: Arc<dyn HealthServiceTrait + Send + Sync>,
    goal_service: Arc<dyn GoalServiceTrait + Send + Sync>,
    background: bool,
    warm_after_update: bool,
    /// Set when this job's market sync stored a new or changed close.
    prices_changed: AtomicBool,
}

impl ServerJobObserver {
    /// User-initiated and startup jobs publish ordinary progress events.
    pub(crate) fn new(state: &AppState) -> Self {
        Self::with_mode(
            state.event_bus.clone(),
            state.health_service.clone(),
            state.goal_service.clone(),
            false,
            false,
        )
    }

    /// Domain-event jobs follow an edit and should warm the dashboard cache.
    pub(crate) fn with_parts(
        event_bus: EventBus,
        health_service: Arc<dyn HealthServiceTrait + Send + Sync>,
        goal_service: Arc<dyn GoalServiceTrait + Send + Sync>,
    ) -> Self {
        Self::with_mode(event_bus, health_service, goal_service, false, true)
    }

    fn background(state: &AppState) -> Self {
        Self::with_mode(
            state.event_bus.clone(),
            state.health_service.clone(),
            state.goal_service.clone(),
            true,
            false,
        )
    }

    fn with_mode(
        event_bus: EventBus,
        health_service: Arc<dyn HealthServiceTrait + Send + Sync>,
        goal_service: Arc<dyn GoalServiceTrait + Send + Sync>,
        background: bool,
        warm_after_update: bool,
    ) -> Self {
        Self {
            event_bus,
            health_service,
            goal_service,
            background,
            warm_after_update,
            prices_changed: AtomicBool::new(false),
        }
    }

    fn progress(&self, name: &'static str) -> ServerEvent {
        if self.background {
            ServerEvent::with_payload(name, json!({ "background": true }))
        } else {
            ServerEvent::new(name)
        }
    }

    fn clear_health_cache(&self) {
        let health_service = self.health_service.clone();
        tokio::spawn(async move {
            health_service.clear_cache().await;
        });
    }
}

#[async_trait::async_trait]
impl JobObserver for ServerJobObserver {
    fn market_sync_started(&self) {
        self.event_bus.publish(self.progress(MARKET_SYNC_START));
    }

    fn market_sync_completed(&self, result: &SyncResult) {
        self.clear_health_cache();
        if result.quotes_synced > 0 {
            self.prices_changed.store(true, Ordering::SeqCst);
        }
        let skipped_reasons: Vec<(String, String)> = result
            .skipped_reasons
            .iter()
            .map(|(asset_id, reason)| (asset_id.clone(), reason.to_string()))
            .collect();
        let mut payload = json!(MarketSyncResult {
            failed_syncs: result.failures.clone(),
            skipped_reasons,
            show_skipped_reasons: false,
        });
        if self.background {
            payload["background"] = json!(true);
        }
        self.event_bus
            .publish(ServerEvent::with_payload(MARKET_SYNC_COMPLETE, payload));
    }

    fn market_sync_failed(&self, message: &str) {
        if !self.background {
            self.event_bus
                .publish(ServerEvent::with_payload(MARKET_SYNC_ERROR, json!(message)));
        }
    }

    fn update_started(&self) {
        self.event_bus
            .publish(self.progress(PORTFOLIO_UPDATE_START));
    }

    fn update_failed(&self, failure: &AccountFailure) {
        if failure.code == "INVALID_SNAPSHOT_DATE" {
            self.clear_health_cache();
        }
        if self.background {
            return;
        }
        let payload = if failure.account_id.is_empty() {
            json!(failure.message)
        } else {
            json!(failure)
        };
        self.event_bus
            .publish(ServerEvent::with_payload(PORTFOLIO_UPDATE_ERROR, payload));
    }

    async fn update_completed(&self) {
        self.clear_health_cache();
        match cache_action(
            self.background,
            self.warm_after_update,
            self.prices_changed.load(Ordering::SeqCst),
        ) {
            CacheAction::InvalidateAndWarm => crate::perf_cache::invalidate_and_warm(),
            CacheAction::Invalidate => crate::perf_cache::invalidate(),
            CacheAction::Keep => {}
        }
        self.event_bus
            .publish(self.progress(PORTFOLIO_UPDATE_COMPLETE));
        match self.goal_service.refresh_all_goal_summaries().await {
            Ok(goals) => {
                crate::api::shared::emit_goal_summaries_updated(&self.event_bus, &goals, false)
            }
            Err(error) => tracing::warn!(
                "Goal summary batch refresh failed ({})",
                wealthfolio_core::goals::summary_refresh_error_kind(&error)
            ),
        }
    }
}

#[derive(Debug, PartialEq)]
enum CacheAction {
    Keep,
    Invalidate,
    InvalidateAndWarm,
}

/// What a finished job does to the performance cache. Edits warm it again.
/// A background refresh whose price sync stored no new or changed close
/// leaves valuations as they were, so the cached summaries stay valid.
fn cache_action(background: bool, warm_after_update: bool, prices_changed: bool) -> CacheAction {
    if warm_after_update {
        CacheAction::InvalidateAndWarm
    } else if background && !prices_changed {
        CacheAction::Keep
    } else {
        CacheAction::Invalidate
    }
}

/// Keep prices fresh every five minutes during North American trading hours,
/// hourly otherwise, with the last pre-open wait ending at 09:30:30 New York
/// to give providers time to publish opening prices.
pub(crate) fn background_update_interval(now: DateTime<Utc>) -> std::time::Duration {
    match market_clock(now) {
        Some(seconds) if (OPEN_SECS..CLOSE_SECS).contains(&seconds) => {
            std::time::Duration::from_secs(5 * 60)
        }
        Some(seconds) if seconds < OPEN_SECS => {
            std::time::Duration::from_secs(u64::from((OPEN_SECS - seconds).min(3600)))
        }
        _ => std::time::Duration::from_secs(3600),
    }
}

const OPEN_SECS: u32 = 9 * 3600 + 30 * 60 + 30;
const CLOSE_SECS: u32 = 16 * 3600 + 15 * 60;
const INTRADAY_RETENTION_DAYS: i64 = 14;

fn market_clock(now: DateTime<Utc>) -> Option<u32> {
    let new_york = parse_user_timezone("America/New_York").ok()?;
    let local = now.with_timezone(&new_york);
    (!matches!(local.weekday(), Weekday::Sat | Weekday::Sun))
        .then(|| local.num_seconds_from_midnight())
}

async fn record_intraday_sample(
    state: &AppState,
    timestamp: DateTime<Utc>,
) -> crate::error::ApiResult<()> {
    let tz = parse_user_timezone_or_default(&state.timezone()?);
    state
        .valuation_repository
        .record_intraday_sample(
            timestamp,
            timestamp.with_timezone(&tz).date_naive(),
            state.base_currency()?,
            timestamp - chrono::Duration::days(INTRADAY_RETENTION_DAYS),
        )
        .await?;
    Ok(())
}

/// One periodic coordinator loop for the profile. Startup refresh remains a
/// separate one-shot job in `main_lib`.
pub(crate) fn spawn_periodic_update(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::join!(
            intraday_bootstrap::bootstrap_intraday_history(&state),
            run_periodic_update(&state),
        );
    })
}

async fn run_periodic_update(state: &AppState) {
    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
    tracing::info!("Periodic portfolio update started with adaptive market-hours cadence");
    loop {
        let session_open = market_clock(Utc::now())
            .is_some_and(|seconds| (OPEN_SECS..CLOSE_SECS).contains(&seconds));
        let observer = ServerJobObserver::background(state);
        let report = state
            .portfolio_coordinator
            .update_all(MarketSyncMode::Incremental { asset_ids: None }, &observer)
            .await;
        if session_open
            && report.failures.is_empty()
            && report.market_sync.is_some_and(|sync| sync.is_ok())
            && record_intraday_sample(state, Utc::now()).await.is_err()
        {
            tracing::warn!("Could not record intraday portfolio sample");
        }
        tracing::info!(
            "Periodic portfolio update: {} account(s) rebuilt, {} failure(s)",
            report.account_ids.len(),
            report.failures.len()
        );
        tokio::time::sleep(background_update_interval(chrono::Utc::now())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn background_refresh_without_price_changes_keeps_the_cache() {
        assert_eq!(cache_action(true, false, false), CacheAction::Keep);
        assert_eq!(cache_action(true, false, true), CacheAction::Invalidate);
        assert_eq!(cache_action(false, false, false), CacheAction::Invalidate);
        assert_eq!(
            cache_action(false, true, false),
            CacheAction::InvalidateAndWarm
        );
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn refreshes_every_five_minutes_during_market_hours() {
        assert_eq!(
            background_update_interval(utc(2026, 10, 2, 15, 0)),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            background_update_interval(utc(2026, 10, 2, 20, 10)),
            std::time::Duration::from_secs(300)
        );
    }

    #[test]
    fn last_pre_open_wait_ends_thirty_seconds_after_open() {
        assert_eq!(
            background_update_interval(utc(2026, 10, 5, 13, 20)),
            std::time::Duration::from_secs(630)
        );
        // Both daylight-saving and standard time use 09:30:30 New York.
        for opening in [utc(2026, 10, 5, 13, 30), utc(2026, 11, 5, 14, 30)] {
            for (elapsed, expected) in [(0, 30), (29, 1), (30, 300)] {
                assert_eq!(
                    background_update_interval(opening + chrono::Duration::seconds(elapsed)),
                    std::time::Duration::from_secs(expected)
                );
            }
        }
    }

    #[test]
    fn refreshes_hourly_outside_market_hours() {
        for now in [
            utc(2026, 10, 2, 12, 0),
            utc(2026, 10, 3, 16, 0),
            utc(2026, 10, 2, 21, 0),
        ] {
            assert_eq!(
                background_update_interval(now),
                std::time::Duration::from_secs(3600)
            );
        }
    }
}
