//! Small deployment policies kept outside the upstream portfolio coordinator.
//!
//! The coordinator remains the only calculation path. This module only adapts
//! its observer events for unattended web-server jobs and selects the fork's
//! refresh cadence for the always-on ARM64 deployment.

use std::sync::Arc;

use serde_json::json;
use wealthfolio_core::{
    health::HealthServiceTrait,
    portfolio::coordinator::{AccountFailure, JobObserver},
    quotes::{MarketSyncMode, SyncResult},
    utils::time_utils::parse_user_timezone,
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
    background: bool,
    warm_after_update: bool,
}

impl ServerJobObserver {
    /// User-initiated and startup jobs publish ordinary progress events.
    pub(crate) fn new(state: &AppState) -> Self {
        Self::with_mode(
            state.event_bus.clone(),
            state.health_service.clone(),
            false,
            false,
        )
    }

    /// Domain-event jobs follow an edit and should warm the dashboard cache.
    pub(crate) fn with_parts(
        event_bus: EventBus,
        health_service: Arc<dyn HealthServiceTrait + Send + Sync>,
    ) -> Self {
        Self::with_mode(event_bus, health_service, false, true)
    }

    fn background(state: &AppState) -> Self {
        Self::with_mode(
            state.event_bus.clone(),
            state.health_service.clone(),
            true,
            false,
        )
    }

    fn with_mode(
        event_bus: EventBus,
        health_service: Arc<dyn HealthServiceTrait + Send + Sync>,
        background: bool,
        warm_after_update: bool,
    ) -> Self {
        Self {
            event_bus,
            health_service,
            background,
            warm_after_update,
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

impl JobObserver for ServerJobObserver {
    fn market_sync_started(&self) {
        self.event_bus.publish(self.progress(MARKET_SYNC_START));
    }

    fn market_sync_completed(&self, result: &SyncResult) {
        self.clear_health_cache();
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

    fn update_completed(&self) {
        self.clear_health_cache();
        if self.warm_after_update {
            crate::perf_cache::invalidate_and_warm();
        } else {
            crate::perf_cache::invalidate();
        }
        self.event_bus
            .publish(self.progress(PORTFOLIO_UPDATE_COMPLETE));
    }
}

/// Keep prices fresh every five minutes during North American trading hours,
/// hourly otherwise, with the last pre-open wait ending at 09:30 New York.
pub(crate) fn background_update_interval(
    now: chrono::DateTime<chrono::Utc>,
) -> std::time::Duration {
    use chrono::{Datelike, Timelike, Weekday};

    let Ok(new_york) = parse_user_timezone("America/New_York") else {
        return std::time::Duration::from_secs(3600);
    };
    const OPEN_SECS: u32 = 9 * 3600 + 30 * 60;
    const CLOSE_SECS: u32 = 16 * 3600 + 15 * 60;

    let local = now.with_timezone(&new_york);
    let seconds = local.num_seconds_from_midnight();
    let weekday = !matches!(local.weekday(), Weekday::Sat | Weekday::Sun);
    if weekday && (OPEN_SECS..CLOSE_SECS).contains(&seconds) {
        std::time::Duration::from_secs(5 * 60)
    } else if weekday && seconds < OPEN_SECS {
        std::time::Duration::from_secs(u64::from((OPEN_SECS - seconds).min(3600)))
    } else {
        std::time::Duration::from_secs(3600)
    }
}

/// One periodic coordinator loop for the profile. Startup refresh remains a
/// separate one-shot job in `main_lib`.
pub(crate) fn spawn_periodic_update(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        tracing::info!("Periodic portfolio update started with adaptive market-hours cadence");
        loop {
            let observer = ServerJobObserver::background(&state);
            let report = state
                .portfolio_coordinator
                .update_all(MarketSyncMode::Incremental { asset_ids: None }, &observer)
                .await;
            tracing::info!(
                "Periodic portfolio update: {} account(s) rebuilt, {} failure(s)",
                report.account_ids.len(),
                report.failures.len()
            );
            tokio::time::sleep(background_update_interval(chrono::Utc::now())).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
    fn last_pre_open_wait_ends_at_open() {
        assert_eq!(
            background_update_interval(utc(2026, 10, 5, 13, 20)),
            std::time::Duration::from_secs(600)
        );
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
