//! Fork: quote-sync policies attached to upstream's `sync.rs` and
//! `sync_state.rs` through one-line hooks.
//!
//! - Long-closed positions only need prices for the days they were held
//!   (`backfill_end_date`, `backfill_required_start`).
//! - Term deposits are priced from their terms, not fetched (`fetch_history`).
//! - A sync reports only new or changed closes, so a refetch of unchanged data
//!   does not trigger a portfolio recalculation (`changed_quote_count`).

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use log::info;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use super::client::{HistoricalQuoteFetchError, MarketDataClient};
use super::constants::*;
use super::model::Quote;
use super::store::QuoteStore;
use super::sync_state::{SymbolSyncPlan, SyncPlanningInputs};
use super::types::{AssetId, Day, QuoteSource};
use crate::assets::{Asset, DepositSpec};

/// End of a BackfillHistory window for a non-FX asset.
///
/// Positions that were closed more than the grace period ago only need prices
/// for the period they were held (plus a small margin), so a full history
/// rebuild must not re-download years of prices up to today for them. Open or
/// recently closed positions still backfill up to `fetch_end_date`.
pub(super) fn backfill_end_date(
    inputs: &SyncPlanningInputs,
    planning_today: NaiveDate,
    fetch_end_date: NaiveDate,
) -> NaiveDate {
    match inputs.position_closed_date {
        Some(closed)
            if !inputs.is_active
                && closed + Duration::days(CLOSED_POSITION_GRACE_PERIOD_DAYS) < planning_today =>
        {
            let held_until = inputs.activity_max.map_or(closed, |last| last.max(closed));
            (held_until + Duration::days(BACKFILL_SAFETY_MARGIN_DAYS)).min(fetch_end_date)
        }
        _ => fetch_end_date,
    }
}

/// Earliest quote a position's history needs before it stops counting as
/// `NeedsBackfill`.
///
/// A sold position only needs prices for the days it was held: the buffer can
/// start on a weekend or holiday that never gets a quote, which would refetch
/// the same window on every sync.
pub(super) fn backfill_required_start(
    inputs: &SyncPlanningInputs,
    activity_min: NaiveDate,
    grace_period_days: i64,
    today: NaiveDate,
) -> NaiveDate {
    if inputs.is_active || is_recently_closed(inputs, grace_period_days, today) {
        activity_min - Duration::days(QUOTE_HISTORY_BUFFER_DAYS + BACKFILL_SAFETY_MARGIN_DAYS)
    } else {
        activity_min
    }
}

/// Whether a closed position is still within the grace period after its close
/// date (or, without one, its last activity).
fn is_recently_closed(
    inputs: &SyncPlanningInputs,
    grace_period_days: i64,
    today: NaiveDate,
) -> bool {
    if inputs.is_active {
        return false;
    }
    if let Some(closed_date) = inputs.position_closed_date {
        if (today - closed_date).num_days() <= grace_period_days {
            return true;
        }
    }
    inputs
        .activity_max
        .is_some_and(|activity_max| (today - activity_max).num_days() <= grace_period_days)
}

/// Quote history for a sync plan: term deposits are priced from their terms;
/// everything else comes from the market data client.
pub(super) async fn fetch_history(
    client: &MarketDataClient,
    asset: &Asset,
    plan: &SymbolSyncPlan,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<Quote>, HistoricalQuoteFetchError> {
    match asset.deposit_spec() {
        Some(spec) => Ok(deposit_quotes(asset, &spec, plan.start_date, plan.end_date)),
        None => {
            client
                .fetch_historical_quotes_with_context(asset, start, end)
                .await
        }
    }
}

/// Daily prices for a term deposit from its start (or `start`) to its maturity
/// (or `end`), worked out from its terms.
fn deposit_quotes(
    asset: &Asset,
    spec: &DepositSpec,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<Quote> {
    let now = Utc::now();
    let mut quotes = Vec::new();
    let mut day = start.max(spec.start_date);
    let last = end.min(spec.maturity_date);
    while day <= last {
        if let Some(value) = spec.value_on(day) {
            quotes.push(Quote {
                id: format!(
                    "{}_{}_{}",
                    asset.id,
                    day.format("%Y-%m-%d"),
                    DATA_SOURCE_CALCULATED
                ),
                asset_id: asset.id.clone(),
                timestamp: Utc
                    .from_utc_datetime(&day.and_hms_opt(12, 0, 0).expect("noon is a valid time")),
                open: value,
                high: value,
                low: value,
                close: value,
                adjclose: value,
                volume: Decimal::ZERO,
                currency: asset.quote_ccy.clone(),
                data_source: DATA_SOURCE_CALCULATED.to_string(),
                created_at: now,
                notes: None,
            });
        }
        day += Duration::days(1);
    }
    quotes
}

/// Number of fetched quotes (sorted by time) that are new or changed against
/// the store, logged when non-zero.
///
/// Providers can return days outside the plan (FX has weekend bars), so the
/// comparison covers the days actually fetched.
pub(super) fn changed_quote_count<Q: QuoteStore + ?Sized>(
    store: &Q,
    asset_id: &AssetId,
    plan: &SymbolSyncPlan,
    quotes: &[Quote],
) -> usize {
    let (Some(first), Some(last)) = (quotes.first(), quotes.last()) else {
        return 0;
    };
    let first_day = first.timestamp.date_naive().min(plan.start_date);
    let last_day = last.timestamp.date_naive().max(plan.end_date);
    let quotes_changed = count_changed_quotes(quotes, |source| {
        store
            .range(
                asset_id,
                Day::new(first_day),
                Day::new(last_day),
                Some(source),
            )
            .unwrap_or_default()
    });
    if quotes_changed > 0 {
        // Any change makes the portfolio job recalculate, so name the asset.
        info!(
            "{} new or changed closes for {} (plan {}..{}, fetched {}..{})",
            quotes_changed,
            asset_id,
            plan.start_date,
            plan.end_date,
            first.timestamp.date_naive(),
            last.timestamp.date_naive(),
        );
    }
    quotes_changed
}

/// Counts fetched quotes (sorted by time) whose close differs from the stored
/// row with the same id, or, for a new day, from the previous day's close.
/// Stored rows are loaded per source: an unfiltered range keeps one quote per
/// day, so a day another source also covers (such as a second exchange
/// listing) would hide this provider's row and count it as changed on every
/// sync. A new day at the previous close (a weekend FX bar) only carries the
/// value forward, which valuations already do.
fn count_changed_quotes(
    fetched: &[Quote],
    load_stored: impl Fn(&QuoteSource) -> Vec<Quote>,
) -> usize {
    let mut stored: HashMap<String, Decimal> = HashMap::new();
    let sources: HashSet<&str> = fetched.iter().map(|q| q.data_source.as_str()).collect();
    for source in sources {
        let source = QuoteSource::from_storage_string(source);
        stored.extend(load_stored(&source).into_iter().map(|q| (q.id, q.close)));
    }
    let mut previous: HashMap<&str, Decimal> = HashMap::new();
    let mut changed = 0;
    for quote in fetched {
        let reference = match stored.get(&quote.id) {
            Some(close) => Some(close),
            None => previous.get(quote.data_source.as_str()),
        };
        if reference != Some(&quote.close) {
            changed += 1;
        }
        previous.insert(quote.data_source.as_str(), quote.close);
    }
    changed
}

#[cfg(test)]
mod deposit_quote_tests {
    use super::*;
    use crate::assets::DepositCompounding;
    use rust_decimal_macros::dec;

    #[test]
    fn deposit_quotes_cover_the_term_only() {
        let asset = Asset {
            id: "gic".to_string(),
            quote_ccy: "CAD".to_string(),
            ..Default::default()
        };
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let spec = DepositSpec {
            principal: dec!(1000),
            annual_rate: dec!(0.0365),
            start_date: day(2025, 1, 10),
            maturity_date: day(2025, 1, 12),
            compounding: DepositCompounding::Annual,
        };

        let quotes = deposit_quotes(&asset, &spec, day(2025, 1, 1), day(2025, 1, 31));

        let closes: Vec<_> = quotes.iter().map(|q| q.close).collect();
        assert_eq!(closes, vec![dec!(1000), dec!(1000.10), dec!(1000.20)]);
        assert_eq!(quotes[0].id, "gic_2025-01-10_CALCULATED");
        assert_eq!(quotes[0].data_source, DATA_SOURCE_CALCULATED);
        assert_eq!(quotes[0].currency, "CAD");
        assert!(deposit_quotes(&asset, &spec, day(2025, 2, 1), day(2025, 2, 5)).is_empty());
    }
}

#[cfg(test)]
mod changed_quote_tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn quote(source: &str, day: u32, close: Decimal) -> Quote {
        let ts = Utc.with_ymd_and_hms(2025, 10, day, 20, 0, 0).unwrap();
        Quote {
            id: format!("aep_2025-10-{day:02}_{source}"),
            asset_id: "aep".to_string(),
            timestamp: ts,
            open: close,
            high: close,
            low: close,
            close,
            adjclose: close,
            volume: Decimal::ZERO,
            currency: "USD".to_string(),
            data_source: source.to_string(),
            created_at: ts,
            notes: None,
        }
    }

    /// Stored rows filtered by source, as `QuoteStore::range` does with a source.
    fn loader(stored: Vec<Quote>) -> impl Fn(&QuoteSource) -> Vec<Quote> {
        move |source| {
            stored
                .iter()
                .filter(|q| QuoteSource::from_storage_string(&q.data_source) == *source)
                .cloned()
                .collect()
        }
    }

    #[test]
    fn refetched_closes_shadowed_by_another_source_are_unchanged() {
        // A second listing (Frankfurt) has quotes on the same days as Yahoo.
        let stored = vec![
            quote("YAHOO", 6, dec!(81)),
            quote("BOERSE_FRANKFURT", 6, dec!(97.8)),
            quote("YAHOO", 7, dec!(81.85)),
            quote("BOERSE_FRANKFURT", 7, dec!(102)),
        ];
        let fetched = vec![quote("YAHOO", 6, dec!(81)), quote("YAHOO", 7, dec!(81.85))];

        assert_eq!(count_changed_quotes(&fetched, loader(stored)), 0);
    }

    #[test]
    fn a_new_day_at_the_previous_close_is_unchanged() {
        // Saturday's FX bar repeated as Sunday's once the UTC day rolls over.
        let stored = vec![quote("YAHOO", 3, dec!(0.7019))];
        let fetched = vec![
            quote("YAHOO", 3, dec!(0.7019)),
            quote("YAHOO", 4, dec!(0.7019)),
        ];

        assert_eq!(count_changed_quotes(&fetched, loader(stored)), 0);
    }

    #[test]
    fn new_or_moved_closes_are_counted() {
        let stored = vec![quote("YAHOO", 6, dec!(81))];
        let fetched = vec![
            quote("YAHOO", 6, dec!(81.5)), // moved
            quote("YAHOO", 7, dec!(82)),   // new day
        ];

        assert_eq!(count_changed_quotes(&fetched, loader(stored)), 2);
    }
}

#[cfg(test)]
mod backfill_end_date_tests {
    use super::*;

    fn inputs(closed: Option<NaiveDate>, last_activity: Option<NaiveDate>) -> SyncPlanningInputs {
        SyncPlanningInputs {
            is_active: closed.is_none(),
            position_closed_date: closed,
            activity_min: None,
            activity_max: last_activity,
            quote_min: None,
            quote_max: None,
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn open_position_backfills_to_fetch_end() {
        let end = backfill_end_date(
            &inputs(None, Some(d("2026-01-01"))),
            d("2026-10-01"),
            d("2026-09-30"),
        );
        assert_eq!(end, d("2026-09-30"));
    }

    #[test]
    fn recently_closed_position_backfills_to_fetch_end() {
        let end = backfill_end_date(
            &inputs(Some(d("2026-09-20")), Some(d("2026-09-20"))),
            d("2026-10-01"),
            d("2026-09-30"),
        );
        assert_eq!(end, d("2026-09-30"));
    }

    #[test]
    fn long_closed_position_stops_after_holding_period() {
        let end = backfill_end_date(
            &inputs(Some(d("2025-11-18")), Some(d("2025-11-17"))),
            d("2026-10-01"),
            d("2026-09-30"),
        );
        assert_eq!(
            end,
            d("2025-11-18") + Duration::days(BACKFILL_SAFETY_MARGIN_DAYS)
        );
    }
}

#[cfg(test)]
mod sold_position_category_tests {
    use super::super::sync_state::{determine_sync_category, SyncCategory};
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn sold(first_buy: NaiveDate, quote_min: NaiveDate) -> SyncPlanningInputs {
        SyncPlanningInputs {
            is_active: false,
            position_closed_date: Some(d(2025, 11, 21)),
            activity_min: Some(first_buy),
            activity_max: Some(d(2025, 11, 21)),
            quote_min: Some(quote_min),
            quote_max: Some(d(2025, 12, 1)),
        }
    }

    #[test]
    fn sold_position_with_held_days_covered_is_closed() {
        // Sold a year ago; quotes start the Monday after a weekend that the
        // 52-day buffer before the first buy would require.
        let inputs = sold(d(2025, 11, 12), d(2025, 9, 22));
        assert_eq!(
            determine_sync_category(&inputs, 30, d(2026, 10, 3)),
            SyncCategory::Closed
        );
    }

    #[test]
    fn sold_position_missing_held_days_still_backfills() {
        let inputs = sold(d(2025, 9, 1), d(2025, 9, 22));
        assert_eq!(
            determine_sync_category(&inputs, 30, d(2026, 10, 3)),
            SyncCategory::NeedsBackfill
        );
    }
}
