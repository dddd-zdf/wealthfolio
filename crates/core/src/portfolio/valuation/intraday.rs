//! Intraday portfolio value curve for the dashboard's 1D/1W charts.
//!
//! Wealthfolio only stores one valuation per day, so short ranges draw a
//! straight line. This rebuilds the shape within each day from live intraday
//! bars of the market-priced holdings: every point is that day's stored total
//! plus how far each position's price was from the day's last bar,
//!
//! `value(t) = V(d) + Σ market_value_base × (price(t) / last_price(d) − 1)`.
//!
//! Anchoring on the stored daily totals keeps the curve ending exactly on the
//! official valuation and carries cash, manual assets and deposits as they
//! are. Current position values set each asset's weight, so quantity changes
//! within the window only affect the shape, not the daily levels. Nothing is
//! persisted.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::DailyAccountValuation;
use crate::errors::Result;
use crate::portfolio::holdings::{Holding, HoldingType};
use crate::quotes::{Quote, QuoteServiceTrait};

/// One asset's bars within a day: (time, close), oldest first.
type DayBars = Vec<(DateTime<Utc>, Decimal)>;

/// Window and bar size for an intraday chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayRange {
    /// Last trading session, 5-minute bars.
    Day,
    /// Last five sessions, hourly bars.
    Week,
}

impl IntradayRange {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "1D" => Some(Self::Day),
            "1W" => Some(Self::Week),
            _ => None,
        }
    }

    /// Provider (interval, range) arguments.
    pub fn provider_args(self) -> (&'static str, &'static str) {
        match self {
            Self::Day => ("5m", "1d"),
            Self::Week => ("60m", "5d"),
        }
    }

    /// Calendar days of daily valuations needed as anchors.
    pub fn anchor_days(self) -> i64 {
        match self {
            Self::Day => 5,
            Self::Week => 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntradayValuationPoint {
    pub timestamp: DateTime<Utc>,
    pub total_value_base: Decimal,
    pub net_contribution_base: Decimal,
}

/// Weight of one market-priced asset in the portfolio, in base currency.
#[derive(Debug, Clone)]
pub struct IntradayExposure {
    pub asset_id: String,
    pub market_value_base: Decimal,
}

/// Stored daily total the intraday shape is attached to.
#[derive(Debug, Clone)]
pub struct IntradayAnchor {
    pub date: NaiveDate,
    pub total_value_base: Decimal,
    pub net_contribution_base: Decimal,
}

/// Market-priced security holdings, summed per asset.
pub fn intraday_exposures(holdings: &[Holding]) -> Vec<IntradayExposure> {
    let mut by_asset: BTreeMap<String, Decimal> = BTreeMap::new();
    for holding in holdings {
        if holding.holding_type != HoldingType::Security || holding.is_closed {
            continue;
        }
        let Some(instrument) = holding.instrument.as_ref() else {
            continue;
        };
        if instrument.pricing_mode != "MARKET" || holding.market_value.base.is_zero() {
            continue;
        }
        *by_asset.entry(instrument.id.clone()).or_default() += holding.market_value.base;
    }
    by_asset
        .into_iter()
        .map(|(asset_id, market_value_base)| IntradayExposure {
            asset_id,
            market_value_base,
        })
        .collect()
}

/// Daily totals by date.
pub fn intraday_anchors(daily: &[DailyAccountValuation]) -> Vec<IntradayAnchor> {
    daily
        .iter()
        .map(|v| IntradayAnchor {
            date: v.valuation_date,
            total_value_base: v.total_value_base,
            net_contribution_base: v.net_contribution_base,
        })
        .collect()
}

/// Build the intraday curve. Days without a stored anchor are skipped.
pub fn build_intraday_series(
    anchors: &[IntradayAnchor],
    exposures: &[IntradayExposure],
    bars: &HashMap<String, Vec<Quote>>,
    tz: Tz,
) -> Vec<IntradayValuationPoint> {
    let anchors: HashMap<NaiveDate, &IntradayAnchor> =
        anchors.iter().map(|a| (a.date, a)).collect();

    // asset -> local day -> bars sorted by time
    let mut per_asset: HashMap<&str, BTreeMap<NaiveDate, DayBars>> = HashMap::new();
    for exposure in exposures {
        let Some(quotes) = bars.get(&exposure.asset_id) else {
            continue;
        };
        let days = per_asset.entry(exposure.asset_id.as_str()).or_default();
        for q in quotes.iter().filter(|q| q.close > Decimal::ZERO) {
            let day = q.timestamp.with_timezone(&tz).date_naive();
            days.entry(day).or_default().push((q.timestamp, q.close));
        }
        for day_bars in days.values_mut() {
            day_bars.sort_by_key(|(ts, _)| *ts);
        }
    }

    let days: BTreeSet<NaiveDate> = per_asset
        .values()
        .flat_map(|days| days.keys().copied())
        .filter(|day| anchors.contains_key(day))
        .collect();

    let mut points = Vec::new();
    for day in days {
        let anchor = anchors[&day];
        let legs: Vec<(Decimal, &DayBars)> = exposures
            .iter()
            .filter_map(|e| {
                let day_bars = per_asset.get(e.asset_id.as_str())?.get(&day)?;
                Some((e.market_value_base, day_bars))
            })
            .collect();
        let timestamps: BTreeSet<DateTime<Utc>> = legs
            .iter()
            .flat_map(|(_, day_bars)| day_bars.iter().map(|(ts, _)| *ts))
            .collect();

        for ts in timestamps {
            let mut delta = Decimal::ZERO;
            for (weight, day_bars) in &legs {
                let last = day_bars[day_bars.len() - 1].1;
                // Price as of `ts`: latest bar at or before it, else the first bar.
                let price = day_bars
                    .iter()
                    .rev()
                    .find(|(bar_ts, _)| *bar_ts <= ts)
                    .unwrap_or(&day_bars[0])
                    .1;
                delta += *weight * (price / last - Decimal::ONE);
            }
            points.push(IntradayValuationPoint {
                timestamp: ts,
                total_value_base: (anchor.total_value_base + delta).round_dp(2),
                net_contribution_base: anchor.net_contribution_base,
            });
        }
    }
    points
}

/// Fetch bars for the holdings and build the curve.
pub async fn intraday_valuations(
    holdings: &[Holding],
    daily: &[DailyAccountValuation],
    quote_service: &dyn QuoteServiceTrait,
    range: IntradayRange,
    tz: Tz,
) -> Result<Vec<IntradayValuationPoint>> {
    let exposures = intraday_exposures(holdings);
    if exposures.is_empty() {
        return Ok(Vec::new());
    }
    let asset_ids: Vec<String> = exposures.iter().map(|e| e.asset_id.clone()).collect();
    let (interval, window) = range.provider_args();
    let bars = quote_service
        .get_intraday_quotes(&asset_ids, interval, window)
        .await?;
    Ok(build_intraday_series(
        &intraday_anchors(daily),
        &exposures,
        &bars,
        tz,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn bar(asset: &str, ts: DateTime<Utc>, close: Decimal) -> Quote {
        Quote {
            id: String::new(),
            asset_id: asset.to_string(),
            timestamp: ts,
            open: close,
            high: close,
            low: close,
            close,
            adjclose: close,
            volume: Decimal::ZERO,
            currency: "CAD".to_string(),
            data_source: "YAHOO".to_string(),
            created_at: ts,
            notes: None,
        }
    }

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        // 2026-10-02 in UTC; Vancouver local date is the same during market hours.
        Utc.with_ymd_and_hms(2026, 10, 2, h, m, 0).unwrap()
    }

    #[test]
    fn curve_ends_on_the_daily_total_and_moves_with_prices() {
        let tz: Tz = "America/Vancouver".parse().unwrap();
        let anchors = vec![IntradayAnchor {
            date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            total_value_base: dec!(10000),
            net_contribution_base: dec!(8000),
        }];
        // 6000 of XEQT and 2000 of VFV; cash etc. make up the rest.
        let exposures = vec![
            IntradayExposure {
                asset_id: "xeqt".into(),
                market_value_base: dec!(6000),
            },
            IntradayExposure {
                asset_id: "vfv".into(),
                market_value_base: dec!(2000),
            },
        ];
        let mut bars = HashMap::new();
        bars.insert(
            "xeqt".to_string(),
            vec![
                bar("xeqt", at(14, 30), dec!(29)),
                bar("xeqt", at(20, 0), dec!(30)),
            ],
        );
        bars.insert(
            "vfv".to_string(),
            vec![
                bar("vfv", at(14, 35), dec!(100)),
                bar("vfv", at(20, 0), dec!(100)),
            ],
        );

        let points = build_intraday_series(&anchors, &exposures, &bars, tz);

        assert_eq!(points.len(), 3);
        // 14:30: XEQT 29/30 -> -200; VFV has no bar yet -> its first bar (no move).
        assert_eq!(points[0].total_value_base, dec!(9800));
        assert_eq!(points[1].total_value_base, dec!(9800));
        // Last bar equals the stored daily total.
        assert_eq!(points[2].total_value_base, dec!(10000));
        assert!(points.iter().all(|p| p.net_contribution_base == dec!(8000)));
    }

    #[test]
    fn days_without_a_stored_total_are_skipped() {
        let tz: Tz = "America/Vancouver".parse().unwrap();
        let exposures = vec![IntradayExposure {
            asset_id: "xeqt".into(),
            market_value_base: dec!(6000),
        }];
        let mut bars = HashMap::new();
        bars.insert("xeqt".to_string(), vec![bar("xeqt", at(15, 0), dec!(30))]);
        assert!(build_intraday_series(&[], &exposures, &bars, tz).is_empty());
    }

    #[test]
    fn range_parsing() {
        assert_eq!(IntradayRange::parse("1D"), Some(IntradayRange::Day));
        assert_eq!(IntradayRange::parse("1W"), Some(IntradayRange::Week));
        assert_eq!(IntradayRange::parse("1M"), None);
    }
}
