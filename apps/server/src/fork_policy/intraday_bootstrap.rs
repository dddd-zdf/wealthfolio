//! Fill the short-range dashboard after the profile's first history build.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use chrono::{Duration, Utc};
use tokio::sync::broadcast::error::RecvError;
use wealthfolio_core::accounts::{
    account_supports_portfolio_scope, AccountPurpose, AccountServiceTrait,
};
use wealthfolio_core::portfolio::valuation::intraday::{
    intraday_exposures, reconstruct_intraday_samples, IntradayRange,
};
use wealthfolio_core::portfolio::valuation::{ValuationRepositoryTrait, ValuationStatus};
use wealthfolio_core::utils::time_utils::parse_user_timezone_or_default;

use crate::error::{ApiError, ApiResult};
use crate::events::{ServerEvent, PORTFOLIO_UPDATE_COMPLETE};
use crate::main_lib::AppState;

/// Use the existing history completion event to handle profiles that start
/// before their first daily projection, including fresh imports and upgrades.
pub(super) async fn bootstrap_intraday_history(state: &Arc<AppState>) {
    let mut events = state.event_bus.subscribe();
    loop {
        match seed_intraday_history(state).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) => {
                tracing::warn!("Could not seed intraday portfolio history");
                return;
            }
        }
        loop {
            match events.recv().await {
                Ok(event) if event.name == PORTFOLIO_UPDATE_COMPLETE => break,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => break,
                Err(RecvError::Closed) => return,
            }
        }
    }
}

/// Returns false while the profile has no usable history to seed.
async fn seed_intraday_history(state: &Arc<AppState>) -> ApiResult<bool> {
    let base = state.base_currency()?;
    let timezone = state.timezone()?;
    let tz = parse_user_timezone_or_default(&timezone);
    let now = Utc::now();
    let since = now - Duration::days(IntradayRange::Week.anchor_days());
    let first_date = since.with_timezone(&tz).date_naive();
    let today = now.with_timezone(&tz).date_naive();
    let read_state = state.clone();
    let read_base = base.clone();
    let Some((account_ids, has_history, existing)) =
        tokio::task::spawn_blocking(move || -> ApiResult<_> {
            let repo = &read_state.valuation_repository;
            let existing = repo.intraday_sample_days(&read_base, since)?;
            if existing.len() >= 5 {
                return Ok(None);
            }
            let account_ids: Vec<_> = read_state
                .account_service
                .get_non_archived_accounts()?
                .into_iter()
                .filter(|account| {
                    account_supports_portfolio_scope(account, AccountPurpose::Holdings)
                })
                .map(|account| account.id)
                .collect();
            let daily = repo.get_historical_valuations_for_accounts(
                &account_ids,
                Some(first_date),
                Some(today),
            )?;
            let has_history = daily.iter().any(|row| {
                row.base_currency == read_base
                    && row.value_status == ValuationStatus::Complete
                    && row.valuation_date < today
            });
            Ok(Some((account_ids, has_history, existing)))
        })
        .await
        .map_err(|_| {
            ApiError::Internal("Could not read daily history for intraday bootstrap".into())
        })??
    else {
        return Ok(true);
    };
    // An account created today may have no historical activity yet. Keep
    // listening so a later import can seed its first completed sessions.
    if !has_history {
        return Ok(false);
    }
    let scope = state
        .holdings_service
        .get_holdings_by_account_scope(&account_ids, &base)
        .await?;
    if !scope.incomplete_accounts.is_empty() {
        return Ok(false);
    }
    let mut holdings_by_account = HashMap::<_, Vec<_>>::new();
    for holding in scope.holdings {
        holdings_by_account
            .entry(holding.account_id.clone())
            .or_default()
            .push(holding);
    }
    let exposures: HashMap<_, _> = holdings_by_account
        .into_iter()
        .map(|(account_id, holdings)| (account_id, intraday_exposures(&holdings)))
        .collect();
    let assets: BTreeSet<_> = exposures
        .values()
        .flatten()
        .map(|exposure| exposure.asset_id.clone())
        .collect();
    if assets.is_empty() {
        return Ok(false);
    }
    let bars = state
        .quote_service
        // The provider's five-day window can include a session still in progress.
        .get_intraday_quotes(&assets.into_iter().collect::<Vec<_>>(), "5m", "1mo")
        .await?;
    let repository = state.valuation_repository.clone();
    let read_base = base.clone();
    let (daily, samples) = tokio::task::spawn_blocking(move || -> ApiResult<_> {
        // Read the anchors after the provider call so the startup refresh can
        // finish replacing its daily projections while the bars are loading.
        let mut daily = repository.get_historical_valuations_for_accounts(
            &account_ids,
            Some(first_date),
            Some(today),
        )?;
        daily.retain(|row| row.base_currency == read_base);
        // The scheduled refresh may observe the closing print until 16:15.
        let mut samples = reconstruct_intraday_samples(
            &daily,
            &exposures,
            &bars,
            tz,
            now - Duration::minutes(15),
        );
        samples.retain(|row| !existing.contains(&row.sample.point.timestamp.date_naive()));
        Ok((daily, samples))
    })
    .await
    .map_err(|_| ApiError::Internal("Could not reconstruct intraday history".into()))??;
    if state.base_currency()? != base || state.timezone()? != timezone {
        return Ok(false);
    }
    if samples.is_empty() {
        return Ok(true);
    }
    let inserted = state
        .valuation_repository
        .seed_intraday_samples(daily, samples, base, since)
        .await?;
    if inserted > 0 {
        state.event_bus.publish(ServerEvent::with_payload(
            PORTFOLIO_UPDATE_COMPLETE,
            serde_json::json!({ "background": true, "intradayHistoryReady": true }),
        ));
    }
    Ok(inserted > 0)
}
