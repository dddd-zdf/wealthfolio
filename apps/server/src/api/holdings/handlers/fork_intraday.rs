//! Fork: cached valuation history and the 1D/1W intraday portfolio curve.
//!
//! Both responses live in `perf_cache`, so a portfolio update drops them.
//! The upstream valuation-history handler stays unchanged; these wrappers
//! call it directly.

use std::sync::Arc;

use axum::{Extension, Json};
use serde::Deserialize;
use wealthfolio_core::portfolio::holdings::Holding;
use wealthfolio_core::portfolio::valuation::intraday::{intraday_valuations, IntradayRange};
use wealthfolio_core::portfolios::AccountScope;
use wealthfolio_core::utils::time_utils::{parse_user_timezone_or_default, user_today};

use super::{get_historical_valuations_for_scope, load_holdings_scope_for_filter};
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

/// Intraday bars do not include a matching intra-session FX series. A curve
/// built from them would therefore misstate any open, nonzero foreign holding,
/// including cash or manually priced assets. Returning no intraday points lets
/// the dashboard use the stored daily values for the whole scope.
fn has_unsupported_intraday_inputs(holdings: &[Holding]) -> bool {
    holdings.iter().any(|holding| {
        !holding.is_closed
            && (holding.market_value.base.is_none()
                || ((!holding.quantity.is_zero()
                    || holding
                        .market_value
                        .local
                        .is_some_and(|value| !value.is_zero())
                    || holding
                        .market_value
                        .base
                        .is_some_and(|value| !value.is_zero()))
                    && !holding
                        .local_currency
                        .eq_ignore_ascii_case(&holding.base_currency)))
    })
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
    // The 3.13 holdings read can return rows alongside accounts whose
    // accounting inputs are pending or unavailable. A curve built from that
    // partial scope would silently omit part of the portfolio, so fall back
    // to the daily chart until the scope is complete.
    let holdings_scope =
        load_holdings_scope_for_filter(state.as_ref(), &parsed.filter, false).await?;
    let holdings = holdings_scope.holdings;
    if !holdings_scope.incomplete_accounts.is_empty() || has_unsupported_intraday_inputs(&holdings)
    {
        let value = serde_json::json!([]);
        crate::perf_cache::put_with_ttl(key, generation, value.clone(), INTRADAY_CACHE_TTL);
        return Ok(Json(value));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use wealthfolio_core::portfolio::holdings::{HoldingType, Instrument, MonetaryValue};

    fn holding(holding_type: HoldingType, local: &str, base: &str, value: Decimal) -> Holding {
        Holding {
            id: "holding".to_string(),
            account_id: "account".to_string(),
            holding_type,
            is_closed: false,
            instrument: None::<Instrument>,
            asset_kind: None,
            quantity: value,
            open_date: None,
            lots: None,
            contract_multiplier: Decimal::ONE,
            local_currency: local.to_string(),
            base_currency: base.to_string(),
            fx_rate: None,
            market_value: MonetaryValue {
                local: Some(value),
                base: Some(value),
            },
            cost_basis: None,
            translate_observed_basis: false,
            price: None,
            purchase_price: None,
            unrealized_gain: None,
            unrealized_gain_pct: None,
            realized_gain: None,
            realized_gain_pct: None,
            total_gain: None,
            total_gain_pct: None,
            income: None,
            total_return: None,
            total_return_pct: None,
            return_basis: None,
            day_change: None,
            day_change_pct: None,
            prev_close_value: None,
            weight: Some(Decimal::ZERO),
            as_of_date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            metadata: None,
            source_account_ids: Vec::new(),
        }
    }

    #[test]
    fn foreign_open_nonzero_holding_disables_curve() {
        assert!(has_unsupported_intraday_inputs(&[holding(
            HoldingType::Security,
            "USD",
            "CAD",
            Decimal::new(100, 0),
        )]));
        assert!(has_unsupported_intraday_inputs(&[holding(
            HoldingType::Cash,
            "USD",
            "CAD",
            Decimal::new(100, 0),
        )]));
        assert!(!has_unsupported_intraday_inputs(&[holding(
            HoldingType::Security,
            "CAD",
            "CAD",
            Decimal::new(100, 0),
        )]));
    }

    #[test]
    fn foreign_cash_disables_mixed_scope_with_domestic_security() {
        let holdings = vec![
            holding(HoldingType::Security, "CAD", "CAD", Decimal::new(100, 0)),
            holding(HoldingType::Cash, "USD", "CAD", Decimal::new(100, 0)),
        ];
        assert!(has_unsupported_intraday_inputs(&holdings));
    }

    #[test]
    fn zero_or_closed_foreign_holding_does_not_disable_curve() {
        let mut zero = holding(HoldingType::Cash, "USD", "CAD", Decimal::ZERO);
        let mut closed = holding(HoldingType::Cash, "USD", "CAD", Decimal::new(100, 0));
        closed.is_closed = true;
        assert!(!has_unsupported_intraday_inputs(std::slice::from_ref(
            &zero
        )));
        assert!(!has_unsupported_intraday_inputs(&[closed]));
        zero.market_value.base = Some(Decimal::new(1, 0));
        assert!(has_unsupported_intraday_inputs(&[zero]));
    }

    #[test]
    fn unpriced_foreign_position_still_disables_the_whole_scope_curve() {
        let mut unpriced = holding(HoldingType::Security, "USD", "CAD", Decimal::ZERO);
        unpriced.quantity = Decimal::ONE;
        assert!(has_unsupported_intraday_inputs(&[unpriced]));
    }

    #[test]
    fn missing_base_value_disables_the_whole_scope_curve() {
        let mut incomplete = holding(HoldingType::Security, "CAD", "CAD", Decimal::ONE);
        incomplete.market_value.base = None;
        assert!(has_unsupported_intraday_inputs(&[incomplete]));
    }
}
