//! Fork-owned, read-only market quote lookup. Keep the upstream sync path intact.
//!
//! Tracked listings use persisted Yahoo quotes without fetching. Other listings
//! use the configured Yahoo provider's shared cache/policy and a separate small
//! agent allowance. No assets, quotes, or transactions are persisted here.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use wealthfolio_market_data::YahooProvider;

use crate::assets::{AssetRepositoryTrait, QuoteMode};
use crate::errors::{Error, Result};

use super::{MarketDataClient, MarketDataError, QuoteStore, DATA_SOURCE_YAHOO};

const AGENT_MISSES_PER_HOUR: usize = 30;
static AGENT_ALLOWANCE: LazyLock<Mutex<VecDeque<Instant>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketQuoteSnapshot {
    pub symbol: String,
    pub price: String,
    pub currency: String,
    pub source: String,
    /// Provider quote/bar timestamp, never the time this tool was called.
    pub quote_time: DateTime<Utc>,
    /// Unknown for persisted quotes: created_at is not a reliable last-fetch time.
    pub fetched_at: Option<DateTime<Utc>>,
    pub served_at: DateTime<Utc>,
    pub cache_status: String,
    pub quote_age_seconds: i64,
    pub note: String,
}

impl MarketQuoteSnapshot {
    fn new(
        symbol: &str,
        price: Decimal,
        currency: &str,
        timestamp: DateTime<Utc>,
        fetched_at: Option<DateTime<Utc>>,
        cache_status: &str,
        reason: &str,
    ) -> Result<Self> {
        if price <= Decimal::ZERO || currency.trim().is_empty() {
            return Err(Error::Unexpected(
                "Yahoo quote has no usable price or currency".into(),
            ));
        }
        let now = Utc::now();
        Ok(Self {
            symbol: symbol.to_string(),
            price: price.normalize().to_string(),
            currency: currency.to_string(),
            source: DATA_SOURCE_YAHOO.to_string(),
            quote_time: timestamp,
            fetched_at,
            served_at: now,
            cache_status: cache_status.to_string(),
            quote_age_seconds: (now - timestamp).num_seconds().max(0),
            note: format!("{reason} Inspect quoteTime: quotes can be delayed or from an earlier market session; a recent fetch does not mean a recent trade. Currency units follow the provider (GBp/GBX means British pence)."),
        })
    }
}

fn reserve_miss(allowance: &mut VecDeque<Instant>, now: Instant) -> bool {
    while allowance
        .front()
        .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(3600))
    {
        allowance.pop_front();
    }
    if allowance.len() >= AGENT_MISSES_PER_HOUR {
        return false;
    }
    allowance.push_back(now);
    true
}

pub(crate) async fn get_market_quote<Q: QuoteStore, A: AssetRepositoryTrait>(
    store: &Q,
    assets: &A,
    client: &MarketDataClient,
    symbol: &str,
) -> Result<MarketQuoteSnapshot> {
    // Exact provider resolution avoids confusing the US and Canadian listings.
    let ids: Vec<String> = assets
        .list()?
        .into_iter()
        .filter(|asset| asset.quote_mode == QuoteMode::Market)
        .filter(|asset| {
            client
                .yahoo_symbol(asset)
                .is_some_and(|listing| listing.eq_ignore_ascii_case(symbol))
        })
        .map(|asset| asset.id)
        .collect();
    if !ids.is_empty() {
        if let Some(quote) = store
            .get_latest_quotes(&ids)?
            .into_values()
            .filter(|quote| {
                quote.data_source == DATA_SOURCE_YAHOO
                    && quote.close > Decimal::ZERO
                    && !quote.currency.trim().is_empty()
            })
            .max_by_key(|quote| quote.timestamp)
        {
            return MarketQuoteSnapshot::new(
                symbol,
                quote.close,
                &quote.currency,
                quote.timestamp,
                None,
                "stored",
                "Stored Wealthfolio quote; this lookup did not request a refresh.",
            );
        }
    }

    let cached = YahooProvider::cached_quote(symbol);
    if let Some(cached) = cached.as_ref().filter(|cached| !cached.is_stale) {
        return MarketQuoteSnapshot::new(
            symbol,
            cached.quote.close,
            &cached.quote.currency,
            cached.quote.timestamp,
            Some(cached.fetched_at),
            "cached",
            "Shared Yahoo cache; no new provider request.",
        );
    }

    let allowed = {
        let mut allowance = AGENT_ALLOWANCE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reserve_miss(&mut allowance, Instant::now())
    };
    let result = if allowed {
        client.fetch_yahoo_quote(symbol).await.map_err(|error| {
            // Provider transport errors can contain URLs with Yahoo crumbs.
            // Expose a useful category without leaking transport credentials.
            let message = match error {
                Error::MarketData(MarketDataError::RateLimitExceeded(_)) => {
                    "Yahoo requests are paused by the shared rate-limit policy; try later"
                }
                Error::MarketData(MarketDataError::NoProvidersAvailable) => {
                    "Yahoo is not enabled in the configured market-data providers"
                }
                _ => "Yahoo quote is unavailable; check the symbol or try later",
            };
            Error::Unexpected(message.into())
        })
    } else {
        Err(Error::Unexpected("Agent quote allowance exhausted (30 cache misses per hour); use cached quotes or try later".into()))
    };
    match result {
        Ok(quote) if quote.source == DATA_SOURCE_YAHOO => {
            // Provider caching records the actual fetch time, even for a coalesced call.
            let fetched_at = YahooProvider::cached_quote(symbol).map(|cached| cached.fetched_at);
            MarketQuoteSnapshot::new(
                symbol,
                quote.close,
                &quote.currency,
                quote.timestamp,
                fetched_at,
                "fetched",
                "Shared Yahoo lookup; no portfolio records were changed.",
            )
        }
        Ok(_) => Err(Error::Unexpected("Quote did not come from Yahoo".into())),
        Err(error) => match cached {
            Some(cached) => MarketQuoteSnapshot::new(
                symbol,
                cached.quote.close,
                &cached.quote.currency,
                cached.quote.timestamp,
                Some(cached.fetched_at),
                "stale",
                &format!("Serving an older cached quote because refresh was unavailable: {error}."),
            ),
            None => Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_allowance_is_bounded_and_expires_without_an_hour_boundary_burst() {
        let now = Instant::now();
        let mut allowance = VecDeque::new();
        for _ in 0..AGENT_MISSES_PER_HOUR {
            assert!(reserve_miss(&mut allowance, now));
        }
        assert!(!reserve_miss(
            &mut allowance,
            now + Duration::from_secs(3599)
        ));
        assert!(reserve_miss(
            &mut allowance,
            now + Duration::from_secs(3600)
        ));
        assert_eq!(allowance.len(), 1);
    }

    #[test]
    fn quote_timestamp_and_currency_units_are_preserved_and_fetch_time_can_be_unknown() {
        let quote_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let result = MarketQuoteSnapshot::new(
            "VOD.L",
            Decimal::new(12345, 3),
            "GBp",
            quote_time,
            None,
            "stored",
            "Stored quote.",
        )
        .unwrap();
        assert_eq!(result.price, "12.345");
        assert_eq!(result.currency, "GBp");
        assert_eq!(result.quote_time, quote_time);
        assert!(result.fetched_at.is_none());
        assert!(result.served_at > result.quote_time);
    }
}
