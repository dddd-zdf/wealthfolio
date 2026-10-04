//! Background schedulers for the Docker/Web server: periodic broker sync
//! (fixed 4-hour interval) and price/valuation updates.

use std::sync::Arc;

#[cfg(feature = "connect-sync")]
use tokio::time::{interval, Duration};
#[cfg(not(feature = "connect-sync"))]
use tracing::info;
#[cfg(all(feature = "device-sync", not(feature = "connect-sync")))]
use tracing::warn;
#[cfg(feature = "connect-sync")]
use tracing::{debug, info, warn};

#[cfg(feature = "connect-sync")]
use crate::api::connect::{has_broker_sync, perform_broker_sync};
use crate::api::shared::{process_background_portfolio_job, PortfolioJobConfig};
use crate::main_lib::AppState;
#[cfg(feature = "connect-sync")]
use wealthfolio_connect::CLOUD_REFRESH_TOKEN_KEY;
use wealthfolio_core::{
    portfolio::{snapshot::SnapshotRecalcMode, valuation::ValuationRecalcMode},
    quotes::MarketSyncMode,
    utils::time_utils::parse_user_timezone,
};

/// Sync interval: 4 hours (not user-configurable to prevent API abuse)
#[cfg(feature = "connect-sync")]
const SYNC_INTERVAL_SECS: u64 = 4 * 60 * 60;

/// Initial delay before first sync (60 seconds to let server fully start)
#[cfg(feature = "connect-sync")]
const INITIAL_DELAY_SECS: u64 = 60;

/// Starts the background broker sync scheduler.
#[cfg(feature = "connect-sync")]
pub fn start_broker_sync_scheduler(state: Arc<AppState>) {
    let runtime = state.clone();
    let worker = tokio::spawn(async move {
        info!("Broker sync scheduler started (4-hour interval)");

        // Initial delay before first sync
        tokio::time::sleep(Duration::from_secs(INITIAL_DELAY_SECS)).await;

        // Set up periodic sync - first tick is immediate, subsequent ticks are 4h apart
        let mut sync_interval = interval(Duration::from_secs(SYNC_INTERVAL_SECS));

        loop {
            sync_interval.tick().await;
            run_scheduled_sync(&runtime).await;
        }
    });
    state.workers.lock().unwrap().push(worker);
}

/// Starts the background broker sync scheduler.
#[cfg(not(feature = "connect-sync"))]
pub fn start_broker_sync_scheduler(_state: Arc<AppState>) {
    info!("Broker sync scheduler disabled: connect-sync feature is not compiled");
}

/// Runs a single scheduled sync operation.
#[cfg(feature = "connect-sync")]
async fn run_scheduled_sync(state: &Arc<AppState>) {
    info!("Running scheduled broker sync...");

    // Check if user has a refresh token configured (indicates they've logged in)
    let has_token = state
        .secret_store
        .get_secret(CLOUD_REFRESH_TOKEN_KEY)
        .map(|t| t.is_some())
        .unwrap_or(false);

    if !has_token {
        debug!("Scheduled sync skipped: no refresh token configured");
        return;
    }

    // Check if user's plan includes broker sync
    match has_broker_sync(state).await {
        Ok(true) => {}
        Ok(false) => {
            debug!("Scheduled sync skipped: plan does not include broker sync");
            return;
        }
        Err(e) => {
            debug!(
                "Scheduled sync skipped: could not verify broker sync access ({})",
                e
            );
            return;
        }
    }

    // Perform the sync using the shared perform_broker_sync from api::connect
    // This uses SyncOrchestrator which:
    // - Emits broker:sync-start, broker:sync-complete, broker:sync-error events via SSE
    // - Handles subscription validation internally
    // - Syncs connections, accounts, activities, and holdings
    match perform_broker_sync(state).await {
        Ok(result) => {
            let activities_count = result
                .activities_synced
                .as_ref()
                .map(|a| a.activities_upserted)
                .unwrap_or(0);
            info!(
                "Scheduled broker sync completed: {} activities synced",
                activities_count
            );
        }
        Err(e) => {
            // Check if this is an auth error (expected when user isn't logged in)
            if e.contains("No refresh token")
                || e.contains("not authenticated")
                || e.contains("Session expired")
                || e.contains("Broker sync already running")
            {
                debug!("Scheduled sync skipped: {}", e);
            } else {
                warn!("Scheduled broker sync failed: {}", e);
            }
        }
    }
}

#[cfg(feature = "device-sync")]
fn is_expected_startup_token_warmup_error(err: &crate::error::ApiError) -> bool {
    match err {
        crate::error::ApiError::Unauthorized(_) | crate::error::ApiError::Forbidden(_) => true,
        crate::error::ApiError::Internal(message) => {
            message.contains("No refresh token configured")
                || message.contains("Auth refresh configuration is missing")
                || message.contains("CONNECT_AUTH_URL or CONNECT_AUTH_PUBLISHABLE_KEY")
        }
        _ => false,
    }
}

/// Start background jobs after server construction succeeds.
pub fn start_background_workers(state: Arc<AppState>) {
    #[cfg(feature = "device-sync")]
    #[allow(clippy::collapsible_if)]
    if crate::features::device_sync_enabled() {
        let startup_state = state.clone();
        let worker = tokio::spawn(async move {
            match crate::api::connect::mint_access_token(&startup_state).await {
                Ok(token) => {
                    if startup_state
                        .device_enroll_service
                        .get_sync_state(&token)
                        .await
                        .map(|sync_state| {
                            sync_state.state == wealthfolio_device_sync::SyncState::Ready
                        })
                        .unwrap_or(false)
                    {
                        if let Err(err) =
                            crate::api::device_sync_engine::ensure_background_engine_started(
                                startup_state.clone(),
                            )
                            .await
                        {
                            warn!(
                                "Failed to auto-start device sync background engine: {}",
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    if is_expected_startup_token_warmup_error(&err) {
                        info!(
                            "Skipping startup device sync token warmup (expected state): {}",
                            err
                        );
                    } else {
                        warn!("Device sync token warmup failed during startup: {}", err);
                    }
                }
            }
        });
        state.workers.lock().unwrap().push(worker);
    }

    // Start background broker sync scheduler (4-hour interval)
    start_broker_sync_scheduler(state.clone());

    // Keep prices and valuations current in the background, so opening the
    // app doesn't have to (2min initial delay).
    let job_state = state.clone();
    let worker = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        loop {
            let config = PortfolioJobConfig {
                account_ids: None,
                market_sync_mode: MarketSyncMode::Incremental { asset_ids: None },
                snapshot_mode: SnapshotRecalcMode::IncrementalFromLast,
                valuation_mode: ValuationRecalcMode::IncrementalFromLast,
                since_date: None,
            };
            if let Err(err) = process_background_portfolio_job(job_state.clone(), config).await {
                tracing::error!("Background portfolio update failed: {}", err);
            }
            tokio::time::sleep(background_update_interval(chrono::Utc::now())).await;
        }
    });
    state.workers.lock().unwrap().push(worker);
}

/// Every 5 minutes while North American markets are open (weekdays
/// 9:30-16:00 New York time, plus a few minutes for closing prices), hourly
/// otherwise, cut short so the first run of the day lands at the open.
/// Off-hours runs find no price changes and skip the recalculation.
fn background_update_interval(now: chrono::DateTime<chrono::Utc>) -> std::time::Duration {
    use chrono::{Datelike, Timelike, Weekday};

    let Ok(new_york) = parse_user_timezone("America/New_York") else {
        return std::time::Duration::from_secs(3600);
    };
    const OPEN_SECS: u32 = 9 * 3600 + 30 * 60;
    const CLOSE_SECS: u32 = 16 * 3600 + 15 * 60;

    let local = now.with_timezone(&new_york);
    let secs = local.num_seconds_from_midnight();
    let weekday = !matches!(local.weekday(), Weekday::Sat | Weekday::Sun);
    if weekday && (OPEN_SECS..CLOSE_SECS).contains(&secs) {
        std::time::Duration::from_secs(5 * 60)
    } else if weekday && secs < OPEN_SECS {
        std::time::Duration::from_secs(u64::from((OPEN_SECS - secs).min(3600)))
    } else {
        std::time::Duration::from_secs(3600)
    }
}

#[cfg(all(test, feature = "device-sync"))]
mod tests {
    use super::*;
    use crate::error::ApiError;

    #[test]
    fn startup_token_warmup_treats_unauthorized_as_expected() {
        let err = ApiError::Forbidden("No refresh token configured".to_string());
        assert!(is_expected_startup_token_warmup_error(&err));
    }

    #[test]
    fn startup_token_warmup_treats_missing_config_as_expected() {
        let err = ApiError::Internal(
            "CONNECT_AUTH_URL or CONNECT_AUTH_PUBLISHABLE_KEY is not configured".to_string(),
        );
        assert!(is_expected_startup_token_warmup_error(&err));
    }

    #[test]
    fn startup_token_warmup_treats_unexpected_internal_as_warning_candidate() {
        let err = ApiError::Internal("Upstream refresh timeout".to_string());
        assert!(!is_expected_startup_token_warmup_error(&err));
    }
}

#[cfg(test)]
mod background_update_tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn updates_every_five_minutes_while_markets_are_open() {
        // Fri 2026-10-02 11:00 New York (EDT, UTC-4).
        assert_eq!(
            background_update_interval(utc(2026, 10, 2, 15, 0)),
            std::time::Duration::from_secs(300)
        );
        // 16:10 New York still catches closing prices.
        assert_eq!(
            background_update_interval(utc(2026, 10, 2, 20, 10)),
            std::time::Duration::from_secs(300)
        );
    }

    #[test]
    fn the_last_wait_before_the_open_ends_at_the_open() {
        // Mon 2026-10-05 09:20 New York: next run at 09:30, not 10:20.
        assert_eq!(
            background_update_interval(utc(2026, 10, 5, 13, 20)),
            std::time::Duration::from_secs(600)
        );
    }

    #[test]
    fn updates_hourly_outside_market_hours() {
        // Fri 08:00 New York (over an hour before the open), Sat noon, Fri 17:00.
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
