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

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::{America, Tz};
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

/// An observed intraday account valuation. Storage filters these by the
/// reporting currency before the selected account scope is assembled.
#[derive(Debug, Clone)]
pub struct IntradayAccountSample {
    pub account_id: String,
    pub point: IntradayValuationPoint,
}

/// A reconstructed observation together with its daily valuation anchor date.
#[derive(Debug, Clone)]
pub struct ReconstructedIntradaySample {
    pub anchor_date: NaiveDate,
    pub sample: IntradayAccountSample,
}

/// Sum only observations that include every account in the selected scope.
/// The latest observed session supplies 1D; up to five sessions supply 1W.
pub fn observed_intraday_series(
    account_ids: &[String],
    samples: Vec<IntradayAccountSample>,
    range: IntradayRange,
) -> Vec<IntradayValuationPoint> {
    let mut by_time = BTreeMap::<_, Vec<_>>::new();
    for sample in samples {
        by_time
            .entry(sample.point.timestamp)
            .or_default()
            .push(sample);
    }

    let mut points = Vec::new();
    let mut last_session = None;
    let mut sessions = 0;
    let max_sessions = match range {
        IntradayRange::Day => 1,
        IntradayRange::Week => 5,
    };
    let expected: BTreeSet<_> = account_ids.iter().map(String::as_str).collect();
    let selected_day = if range == IntradayRange::Day {
        let mut complete_points_by_day = BTreeMap::<NaiveDate, usize>::new();
        for (timestamp, samples) in &by_time {
            let observed: BTreeSet<_> = samples
                .iter()
                .map(|sample| sample.account_id.as_str())
                .collect();
            if observed == expected && samples.len() == expected.len() {
                let day = timestamp
                    .with_timezone(&chrono_tz::America::New_York)
                    .date_naive();
                *complete_points_by_day.entry(day).or_default() += 1;
            }
        }
        let latest = complete_points_by_day.keys().next_back().copied();
        latest.and_then(|latest| {
            if complete_points_by_day[&latest] >= 2 {
                Some(latest)
            } else {
                complete_points_by_day
                    .iter()
                    .rev()
                    .find_map(|(day, count)| (*count >= 2).then_some(*day))
                    .or(Some(latest))
            }
        })
    } else {
        None
    };
    for (timestamp, samples) in by_time.into_iter().rev() {
        let observed: BTreeSet<_> = samples
            .iter()
            .map(|sample| sample.account_id.as_str())
            .collect();
        if observed != expected || samples.len() != expected.len() {
            continue;
        }
        let session = timestamp
            .with_timezone(&chrono_tz::America::New_York)
            .date_naive();
        if let Some(selected_day) = selected_day {
            if session != selected_day {
                continue;
            }
        }
        if last_session != Some(session) {
            if sessions == max_sessions {
                break;
            }
            last_session = Some(session);
            sessions += 1;
        }
        points.push(IntradayValuationPoint {
            timestamp,
            total_value_base: samples
                .iter()
                .map(|sample| sample.point.total_value_base)
                .sum(),
            net_contribution_base: samples
                .iter()
                .map(|sample| sample.point.net_contribution_base)
                .sum(),
        });
    }
    points.reverse();
    points
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
        let Some(market_value_base) = holding.market_value.base else {
            continue;
        };
        if instrument.pricing_mode != "MARKET" || market_value_base.is_zero() {
            continue;
        }
        *by_asset.entry(instrument.id.clone()).or_default() += market_value_base;
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
        // Unavailable rows carry zero or partial base values and must never
        // become chart anchors. Partial-unpriced rows remain valid anchors,
        // matching the upstream chart's valueStatus guard.
        .filter(|v| !v.value_status.is_unavailable_for_returns())
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
            points.push(IntradayValuationPoint {
                timestamp: ts,
                total_value_base: intraday_total_at(anchor.total_value_base, &legs, ts),
                net_contribution_base: anchor.net_contribution_base,
            });
        }
    }
    points
}

/// Apply the current position weights to one point in a day's price series.
fn intraday_total_at(
    anchor_total: Decimal,
    legs: &[(Decimal, &DayBars)],
    timestamp: DateTime<Utc>,
) -> Decimal {
    let mut delta = Decimal::ZERO;
    for (weight, day_bars) in legs {
        let last = day_bars[day_bars.len() - 1].1;
        // Price as of `timestamp`: latest bar at or before it, else the first bar.
        let position = day_bars.partition_point(|(bar_ts, _)| *bar_ts <= timestamp);
        let price = if position == 0 {
            day_bars[0].1
        } else {
            day_bars[position - 1].1
        };
        delta += *weight * (price / last - Decimal::ONE);
    }
    (anchor_total + delta).round_dp(2)
}

fn ny_session_close(day: NaiveDate) -> Option<DateTime<Utc>> {
    America::New_York
        .from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(16, 0, 0)?))
        .single()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

/// Reconstruct intraday observations after a cold profile start.
///
/// Provider bars are five-minute intervals whose timestamps denote their
/// starts. Reconstructed observations use interval ends, so the final regular
/// session bar is observed at 16:00 New York time. A single grid is shared by
/// every account; accounts without usable bars retain their daily total.
pub fn reconstruct_intraday_samples(
    daily: &[DailyAccountValuation],
    exposures: &HashMap<String, Vec<IntradayExposure>>,
    bars: &HashMap<String, Vec<Quote>>,
    tz: Tz,
    now: DateTime<Utc>,
) -> Vec<ReconstructedIntradaySample> {
    let open = NaiveTime::from_hms_opt(9, 30, 0).expect("valid NYSE open");
    let close = NaiveTime::from_hms_opt(16, 0, 0).expect("valid NYSE close");

    // Keep only bars belonging to an exposed asset so unrelated provider
    // history cannot create a grid with no usable portfolio market history.
    let asset_ids: BTreeSet<&str> = exposures
        .values()
        .flat_map(|account_exposures| account_exposures.iter().map(|e| e.asset_id.as_str()))
        .collect();
    let mut per_asset: HashMap<&str, BTreeMap<NaiveDate, DayBars>> = HashMap::new();
    let mut grid: BTreeMap<NaiveDate, BTreeSet<DateTime<Utc>>> = BTreeMap::new();

    for asset_id in asset_ids {
        let Some(quotes) = bars.get(asset_id) else {
            continue;
        };
        for quote in quotes.iter().filter(|quote| quote.close > Decimal::ZERO) {
            let ny_timestamp = quote.timestamp.with_timezone(&America::New_York);
            let ny_day = ny_timestamp.date_naive();
            if matches!(ny_day.weekday(), Weekday::Sat | Weekday::Sun)
                || ny_timestamp.time() < open
                || ny_timestamp.time() >= close
            {
                continue;
            }
            let Some(session_close) = ny_session_close(ny_day) else {
                continue;
            };
            if now < session_close {
                continue;
            }

            // The profile timezone determines the daily-anchor date. This is
            // the NY session date for North American profiles and correctly
            // handles profiles configured east of New York as well.
            let anchor_day = session_close.with_timezone(&tz).date_naive();
            let endpoint = (quote.timestamp + Duration::minutes(5)).min(session_close);
            per_asset
                .entry(asset_id)
                .or_default()
                .entry(anchor_day)
                .or_default()
                .push((endpoint, quote.close));
            grid.entry(anchor_day).or_default().insert(endpoint);
        }
    }

    for day_bars in per_asset.values_mut().flat_map(BTreeMap::values_mut) {
        day_bars.sort_by_key(|(timestamp, _)| *timestamp);
    }

    // A cold start can receive more than five sessions from a provider. Keep
    // the newest five completed weekdays, then render them chronologically.
    let mut sessions: Vec<NaiveDate> = grid.keys().rev().take(5).copied().collect();
    sessions.sort_unstable();

    let anchors: HashMap<(&str, NaiveDate), &DailyAccountValuation> = daily
        .iter()
        .filter(|valuation| valuation.value_status.is_complete())
        .map(|valuation| {
            (
                (valuation.account_id.as_str(), valuation.valuation_date),
                valuation,
            )
        })
        .collect();
    let account_ids: BTreeSet<&str> = anchors.keys().map(|(account_id, _)| *account_id).collect();

    let mut samples = Vec::new();
    for day in sessions {
        let Some(timestamps) = grid.get(&day) else {
            continue;
        };
        let account_legs: Vec<_> = account_ids
            .iter()
            .filter_map(|account_id| {
                let anchor = anchors.get(&(*account_id, day))?;
                let account_exposures =
                    exposures.get(*account_id).map(Vec::as_slice).unwrap_or(&[]);
                let legs = account_exposures
                    .iter()
                    .filter_map(|exposure| {
                        per_asset
                            .get(exposure.asset_id.as_str())?
                            .get(&day)
                            .map(|day_bars| (exposure.market_value_base, day_bars))
                    })
                    .collect::<Vec<_>>();
                Some((*account_id, *anchor, legs))
            })
            .collect();
        for &timestamp in timestamps {
            for (account_id, anchor, legs) in &account_legs {
                samples.push(ReconstructedIntradaySample {
                    anchor_date: day,
                    sample: IntradayAccountSample {
                        account_id: (*account_id).to_string(),
                        point: IntradayValuationPoint {
                            timestamp,
                            total_value_base: intraday_total_at(
                                anchor.total_value_base,
                                legs,
                                timestamp,
                            ),
                            net_contribution_base: anchor.net_contribution_base,
                        },
                    },
                });
            }
        }
    }
    samples
}

/// Fetch bars for the holdings and build the curve.
pub async fn intraday_valuations(
    holdings: &[Holding],
    daily: &[DailyAccountValuation],
    quote_service: &dyn QuoteServiceTrait,
    range: IntradayRange,
    tz: Tz,
) -> Result<Vec<IntradayValuationPoint>> {
    let anchors = intraday_anchors(daily);
    let exposures = intraday_exposures(holdings);
    if anchors.is_empty() || exposures.is_empty() {
        return Ok(Vec::new());
    }
    let asset_ids: Vec<String> = exposures.iter().map(|e| e.asset_id.clone()).collect();
    let (interval, window) = range.provider_args();
    let bars = quote_service
        .get_intraday_quotes(&asset_ids, interval, window)
        .await?;
    Ok(build_intraday_series(&anchors, &exposures, &bars, tz))
}

/// One asset's live bars for the asset page chart, oldest first. Empty when
/// no provider has intraday data for it.
pub async fn asset_intraday_bars(
    quote_service: &dyn QuoteServiceTrait,
    asset_id: &str,
    range: IntradayRange,
) -> Result<Vec<Quote>> {
    let (interval, window) = range.provider_args();
    let mut bars = quote_service
        .get_intraday_quotes(&[asset_id.to_string()], interval, window)
        .await?
        .remove(asset_id)
        .unwrap_or_default();
    bars.retain(|bar| bar.close > Decimal::ZERO);
    bars.sort_by_key(|bar| bar.timestamp);
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::valuation::ValuationStatus;
    use chrono::{Datelike, TimeZone};
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
        at_day(2, h, m)
    }

    fn at_day(day: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, h, m, 0).unwrap()
    }

    fn ny(day: u32, h: u32, m: u32) -> DateTime<Utc> {
        chrono_tz::America::New_York
            .with_ymd_and_hms(2026, 10, day, h, m, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn daily_anchor(
        day: u32,
        status: ValuationStatus,
        total_value_base: Decimal,
    ) -> DailyAccountValuation {
        let date = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
        DailyAccountValuation {
            id: format!("valuation-{day}"),
            account_id: "account-1".to_string(),
            valuation_date: date,
            account_currency: "CAD".to_string(),
            base_currency: "CAD".to_string(),
            fx_rate_to_base: dec!(1),
            cash_balance: dec!(0),
            investment_market_value: total_value_base,
            total_value: total_value_base,
            cost_basis: dec!(0),
            book_basis: dec!(0),
            net_contribution: dec!(0),
            cash_balance_base: dec!(0),
            investment_market_value_base: total_value_base,
            total_value_base,
            cost_basis_base: dec!(0),
            book_basis_base: dec!(0),
            net_contribution_base: dec!(0),
            external_inflow_base: dec!(0),
            external_outflow_base: dec!(0),
            external_flow_source: Default::default(),
            performance_eligible_value_base: total_value_base,
            value_status: status,
            basis_status: Default::default(),
            calculated_at: at_day(day, 0, 0),
        }
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
    fn unavailable_daily_anchors_are_skipped_but_partial_anchors_remain_usable() {
        let tz: Tz = "America/Vancouver".parse().unwrap();
        let daily = vec![
            daily_anchor(1, ValuationStatus::Unavailable, dec!(9000)),
            daily_anchor(2, ValuationStatus::PartialUnpriced, dec!(10000)),
            daily_anchor(3, ValuationStatus::Complete, dec!(11000)),
        ];
        let anchors = intraday_anchors(&daily);
        assert_eq!(
            anchors.iter().map(|anchor| anchor.date).collect::<Vec<_>>(),
            vec![
                NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(),
            ]
        );

        let exposures = vec![IntradayExposure {
            asset_id: "xeqt".into(),
            market_value_base: dec!(6000),
        }];
        let mut bars = HashMap::new();
        bars.insert(
            "xeqt".to_string(),
            vec![
                bar("xeqt", at_day(1, 14, 30), dec!(27)),
                bar("xeqt", at_day(1, 20, 0), dec!(28)),
                bar("xeqt", at_day(2, 14, 30), dec!(29)),
                bar("xeqt", at_day(2, 20, 0), dec!(30)),
                bar("xeqt", at_day(3, 14, 30), dec!(31)),
                bar("xeqt", at_day(3, 20, 0), dec!(32)),
            ],
        );

        let points = build_intraday_series(&anchors, &exposures, &bars, tz);
        assert_eq!(points.len(), 4);
        assert!(points
            .iter()
            .all(|point| point.timestamp.date_naive().day() != 1));
        assert!(points.iter().any(|point| {
            point.timestamp.date_naive().day() == 2 && point.total_value_base == dec!(10000)
        }));
        assert!(points.iter().any(|point| {
            point.timestamp.date_naive().day() == 3 && point.total_value_base == dec!(11000)
        }));
    }

    #[test]
    fn range_parsing() {
        assert_eq!(IntradayRange::parse("1D"), Some(IntradayRange::Day));
        assert_eq!(IntradayRange::parse("1W"), Some(IntradayRange::Week));
        assert_eq!(IntradayRange::parse("1M"), None);
    }

    #[test]
    fn observed_series_requires_the_whole_scope_and_sorts_by_session() {
        let accounts = vec!["a".to_string(), "b".to_string()];
        let sample = |account_id: &str, timestamp, value| IntradayAccountSample {
            account_id: account_id.to_string(),
            point: IntradayValuationPoint {
                timestamp,
                total_value_base: value,
                net_contribution_base: dec!(25),
            },
        };
        let mut samples = Vec::new();
        for day in 1..=6 {
            let ts = at_day(day, 14, 30);
            samples.push(sample("b", ts, dec!(60)));
            samples.push(sample("a", ts, dec!(45)));
        }
        let latest = at_day(6, 14, 35);
        samples.push(sample("a", latest, dec!(40)));
        samples.push(sample("b", latest, dec!(55)));
        samples.push(sample("a", at_day(7, 14, 30), dec!(90)));

        let day = observed_intraday_series(&accounts, samples.clone(), IntradayRange::Day);
        assert_eq!(day.len(), 2);
        assert_eq!(day[0].total_value_base, dec!(105));
        assert_eq!(day[1].timestamp, latest);
        assert_eq!(day[1].total_value_base, dec!(95));
        assert_eq!(day[1].net_contribution_base, dec!(50));

        let week = observed_intraday_series(&accounts, samples, IntradayRange::Week);
        assert_eq!(week.len(), 6);
        assert_eq!(week[0].timestamp, at_day(2, 14, 30));
        assert_eq!(week[5].timestamp, latest);
    }

    #[test]
    fn reconstruction_uses_completed_sessions_and_a_common_grid() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let mut daily = Vec::new();
        for day in [5, 6, 7] {
            let mut account = daily_anchor(day, ValuationStatus::Complete, dec!(100));
            account.account_id = "account-1".to_string();
            daily.push(account);
            let mut cash = daily_anchor(day, ValuationStatus::Complete, dec!(200));
            cash.account_id = "account-2".to_string();
            daily.push(cash);
        }

        let mut exposures = HashMap::new();
        exposures.insert(
            "account-1".to_string(),
            vec![IntradayExposure {
                asset_id: "xeqt".to_string(),
                market_value_base: dec!(100),
            }],
        );
        exposures.insert("account-2".to_string(), Vec::new());

        let mut bars = HashMap::new();
        bars.insert(
            "xeqt".to_string(),
            vec![
                bar("xeqt", ny(5, 9, 30), dec!(10)),
                bar("xeqt", ny(5, 15, 55), dec!(11)),
                bar("xeqt", ny(6, 9, 30), dec!(20)),
                bar("xeqt", ny(6, 15, 55), dec!(22)),
                // The open session must not be fabricated before its close.
                bar("xeqt", ny(7, 9, 30), dec!(30)),
            ],
        );

        let samples = reconstruct_intraday_samples(&daily, &exposures, &bars, tz, ny(7, 15, 0));
        assert_eq!(
            samples.len(),
            8,
            "two sessions x two accounts x two grid points"
        );
        assert!(samples.iter().all(
            |sample| sample.sample.point.timestamp.date_naive() != at_day(7, 0, 0).date_naive()
        ));
        assert!(samples.iter().all(|sample| {
            sample.sample.point.timestamp == ny(5, 9, 35)
                || sample.sample.point.timestamp == ny(5, 16, 0)
                || sample.sample.point.timestamp == ny(6, 9, 35)
                || sample.sample.point.timestamp == ny(6, 16, 0)
        }));
        assert!(samples.iter().any(|sample| {
            sample.sample.account_id == "account-2"
                && sample.sample.point.total_value_base == dec!(200)
        }));
        assert!(samples.iter().any(|sample| {
            sample.sample.account_id == "account-1"
                && sample.sample.point.timestamp == ny(5, 16, 0)
                && sample.sample.point.total_value_base == dec!(100)
        }));
    }

    #[test]
    fn day_series_hands_off_a_singleton_latest_session() {
        let accounts = vec!["a".to_string(), "b".to_string()];
        let sample = |account_id: &str, timestamp| IntradayAccountSample {
            account_id: account_id.to_string(),
            point: IntradayValuationPoint {
                timestamp,
                total_value_base: dec!(1),
                net_contribution_base: Decimal::ZERO,
            },
        };
        let mut samples = vec![
            sample("a", ny(5, 14, 30)),
            sample("b", ny(5, 14, 30)),
            sample("a", ny(5, 14, 35)),
            sample("b", ny(5, 14, 35)),
            sample("a", ny(6, 14, 30)),
            sample("b", ny(6, 14, 30)),
        ];
        let points =
            observed_intraday_series(&accounts, std::mem::take(&mut samples), IntradayRange::Day);
        assert_eq!(points.len(), 2);
        assert!(points
            .iter()
            .all(|point| point.timestamp == ny(5, 14, 30) || point.timestamp == ny(5, 14, 35)));
    }
}
