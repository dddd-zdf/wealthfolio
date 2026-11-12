//! Fork: short-lived account valuation samples for intraday charts.
//!
//! These chart observations come from complete, clean daily projection rows.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use diesel::sql_query;
use diesel::sql_types::{BigInt, Date, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};
use rust_decimal::Decimal;
use wealthfolio_core::errors::Result;
use wealthfolio_core::portfolio::valuation::intraday::{
    IntradayAccountSample, IntradayValuationPoint, ReconstructedIntradaySample,
};
use wealthfolio_core::portfolio::valuation::DailyAccountValuation;

use super::ValuationRepository;
use crate::db::get_connection;
use crate::errors::StorageError;

#[derive(Debug, QueryableByName)]
struct IntradaySampleRow {
    #[diesel(sql_type = Text, column_name = "account_id")]
    account_id: String,
    #[diesel(sql_type = BigInt, column_name = "timestamp")]
    timestamp: i64,
    #[diesel(sql_type = Text, column_name = "total_value_base")]
    total_value_base: String,
    #[diesel(sql_type = Text, column_name = "net_contribution_base")]
    net_contribution_base: String,
}

#[derive(QueryableByName)]
struct IntradaySessionRow {
    #[diesel(sql_type = Date)]
    session: NaiveDate,
}

fn sampled_days(
    conn: &mut SqliteConnection,
    base_currency: &str,
    since: DateTime<Utc>,
) -> Result<HashSet<NaiveDate>> {
    // New York regular-session samples and their UTC timestamps share the date.
    let rows = sql_query(
        "SELECT DISTINCT date(timestamp, 'unixepoch') AS session
         FROM intraday_account_samples WHERE base_currency = ? AND timestamp >= ?",
    )
    .bind::<Text, _>(base_currency)
    .bind::<BigInt, _>(since.timestamp())
    .load::<IntradaySessionRow>(conn)
    .map_err(StorageError::from)?;
    Ok(rows.into_iter().map(|row| row.session).collect())
}

impl ValuationRepository {
    pub fn intraday_sample_days(
        &self,
        base_currency: &str,
        since: DateTime<Utc>,
    ) -> Result<HashSet<NaiveDate>> {
        let mut conn = get_connection(&self.pool)?;
        sampled_days(&mut conn, base_currency, since)
    }

    /// Seed completed sessions with reconstructed chart values. An observed
    /// session wins as a whole, even if its timestamps differ from the bars.
    /// The original daily projection must still be clean and current at write time.
    pub async fn seed_intraday_samples(
        &self,
        daily: Vec<DailyAccountValuation>,
        samples: Vec<ReconstructedIntradaySample>,
        base_currency: String,
        since: DateTime<Utc>,
    ) -> Result<usize> {
        self.writer
            .exec(move |conn| {
                let existing = sampled_days(conn, &base_currency, since)?;
                let anchors: HashMap<_, _> = daily
                    .into_iter()
                    .map(|row| ((row.account_id, row.valuation_date), row.calculated_at))
                    .collect();
                let mut inserted = 0;
                for ReconstructedIntradaySample { anchor_date: date, sample } in samples {
                    let point = sample.point;
                    if existing.contains(&point.timestamp.date_naive()) {
                        continue;
                    }
                    let key = (sample.account_id, date);
                    let Some(calculated_at) = anchors.get(&key) else {
                        continue;
                    };
                    inserted += sql_query(
                        "INSERT INTO intraday_account_samples
                         (account_id, timestamp, base_currency, total_value_base, net_contribution_base)
                         SELECT d.account_id, ?, d.base_currency, ?, ?
                         FROM daily_account_valuation d JOIN accounts a ON a.id = d.account_id
                         WHERE d.account_id = ? AND d.valuation_date = ?
                           AND d.base_currency = ? AND d.calculated_at = ?
                           AND d.value_status = 'COMPLETE' AND a.is_archived = 0
                           AND NOT EXISTS (
                               SELECT 1 FROM projection_state ps
                               WHERE ps.dirty_from <= d.valuation_date
                                 AND (ps.scope = '@all' OR ps.scope = d.account_id)
                           )
                         ON CONFLICT(account_id, timestamp, base_currency) DO NOTHING",
                    )
                    .bind::<BigInt, _>(point.timestamp.timestamp())
                    .bind::<Text, _>(point.total_value_base.to_string())
                    .bind::<Text, _>(point.net_contribution_base.to_string())
                    .bind::<Text, _>(&key.0)
                    .bind::<Text, _>(date.to_string())
                    .bind::<Text, _>(&base_currency)
                    .bind::<Text, _>(calculated_at.to_rfc3339())
                    .execute(conn)
                    .map_err(StorageError::from)?;
                }
                Ok(inserted)
            })
            .await
    }

    /// Record one scheduled intraday observation for every eligible account.
    ///
    /// The writer transaction prunes stale observations and copies only rows
    /// from the requested complete daily projection. A dirty projection marker
    /// for the whole profile or account makes that account ineligible until the
    /// projection has been rebuilt.
    pub async fn record_intraday_sample(
        &self,
        timestamp: DateTime<Utc>,
        date: NaiveDate,
        base_currency: String,
        retain_from: DateTime<Utc>,
    ) -> Result<()> {
        let timestamp_seconds = timestamp.timestamp();
        let retain_from_seconds = retain_from.timestamp();
        let date = date.to_string();

        self.writer
            .exec(move |conn| {
                sql_query("DELETE FROM intraday_account_samples WHERE timestamp < ?")
                    .bind::<BigInt, _>(retain_from_seconds)
                    .execute(conn)
                    .map_err(StorageError::from)?;

                sql_query(
                    "INSERT INTO intraday_account_samples \
                     (account_id, timestamp, base_currency, total_value_base, net_contribution_base) \
                     SELECT d.account_id, ?, d.base_currency, d.total_value_base, d.net_contribution_base \
                     FROM daily_account_valuation d \
                     INNER JOIN accounts a ON a.id = d.account_id \
                     WHERE d.valuation_date = ? \
                       AND d.base_currency = ? \
                       AND d.value_status = 'COMPLETE' \
                       AND a.is_archived = 0 \
                       AND NOT EXISTS ( \
                           SELECT 1 FROM projection_state ps \
                           WHERE ps.dirty_from IS NOT NULL \
                             AND ps.dirty_from <= d.valuation_date \
                             AND (ps.scope = '@all' OR ps.scope = d.account_id) \
                       ) \
                     ON CONFLICT(account_id, timestamp, base_currency) DO UPDATE SET \
                       total_value_base = excluded.total_value_base, \
                       net_contribution_base = excluded.net_contribution_base",
                )
                .bind::<BigInt, _>(timestamp_seconds)
                .bind::<Text, _>(date)
                .bind::<Text, _>(base_currency)
                .execute(conn)
                .map_err(StorageError::from)?;

                Ok(())
            })
            .await
    }

    /// Read retained intraday observations for selected accounts and currency.
    pub fn get_intraday_samples(
        &self,
        account_ids: &[String],
        base_currency: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<IntradayAccountSample>> {
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = get_connection(&self.pool)?;
        let placeholders = std::iter::repeat_n("?", account_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT account_id, timestamp, total_value_base, net_contribution_base \
             FROM intraday_account_samples \
             WHERE account_id IN ({placeholders}) \
               AND base_currency = ? \
               AND timestamp >= ? \
             ORDER BY timestamp ASC, account_id ASC"
        );

        let mut query = sql_query(sql).into_boxed::<diesel::sqlite::Sqlite>();
        for account_id in account_ids {
            query = query.bind::<Text, _>(account_id);
        }
        query = query.bind::<Text, _>(base_currency);
        query = query.bind::<BigInt, _>(since.timestamp());

        let rows = query
            .load::<IntradaySampleRow>(&mut conn)
            .map_err(StorageError::from)?;
        rows.into_iter()
            .map(|row| {
                let timestamp =
                    DateTime::<Utc>::from_timestamp(row.timestamp, 0).ok_or_else(|| {
                        StorageError::SerializationError(format!(
                            "Invalid intraday sample timestamp: {}",
                            row.timestamp
                        ))
                    })?;
                let total_value_base =
                    row.total_value_base.parse::<Decimal>().map_err(|error| {
                        StorageError::SerializationError(format!(
                            "Invalid intraday total value: {error}"
                        ))
                    })?;
                let net_contribution_base =
                    row.net_contribution_base
                        .parse::<Decimal>()
                        .map_err(|error| {
                            StorageError::SerializationError(format!(
                                "Invalid intraday net contribution: {error}"
                            ))
                        })?;

                Ok(IntradayAccountSample {
                    account_id: row.account_id,
                    point: IntradayValuationPoint {
                        timestamp,
                        total_value_base,
                        net_contribution_base,
                    },
                })
            })
            .collect::<std::result::Result<Vec<_>, StorageError>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use diesel::{sql_query, RunQueryDsl, SqliteConnection};
    use rust_decimal::Decimal;
    use tempfile::tempdir;
    use wealthfolio_core::portfolio::economic_events::BasisStatus;
    use wealthfolio_core::portfolio::valuation::{
        DailyAccountValuation, ExternalFlowSource, ValuationRepositoryTrait, ValuationStatus,
    };

    use super::*;
    use crate::db::{create_pool, get_connection, run_migrations, write_actor::spawn_writer};

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, 15).expect("valid test date")
    }

    fn valuation(
        account_id: &str,
        currency: &str,
        status: ValuationStatus,
    ) -> DailyAccountValuation {
        let total = Decimal::new(12345, 2);
        DailyAccountValuation {
            id: format!("{account_id}_{}", date()),
            account_id: account_id.to_string(),
            valuation_date: date(),
            account_currency: currency.to_string(),
            base_currency: currency.to_string(),
            fx_rate_to_base: Decimal::ONE,
            cash_balance: Decimal::new(2345, 2),
            investment_market_value: Decimal::new(10000, 2),
            total_value: total,
            cost_basis: Decimal::new(10000, 2),
            book_basis: Decimal::new(12345, 2),
            net_contribution: Decimal::new(9000, 2),
            cash_balance_base: Decimal::new(2345, 2),
            investment_market_value_base: Decimal::new(10000, 2),
            total_value_base: total,
            cost_basis_base: Decimal::new(10000, 2),
            book_basis_base: Decimal::new(12345, 2),
            net_contribution_base: Decimal::new(9000, 2),
            external_inflow_base: Decimal::ZERO,
            external_outflow_base: Decimal::ZERO,
            external_flow_source: ExternalFlowSource::NoFlow,
            performance_eligible_value_base: total,
            value_status: status,
            basis_status: BasisStatus::Complete,
            calculated_at: Utc::now(),
        }
    }

    fn insert_account(conn: &mut SqliteConnection, id: &str, archived: bool) {
        sql_query(format!(
            "INSERT INTO accounts (id, name, account_type, currency, is_default, is_active, \
             created_at, updated_at, tracking_mode, is_archived) \
             VALUES ('{id}', '{id}', 'SECURITIES', 'USD', 0, 1, datetime('now'), \
             datetime('now'), 'TRANSACTIONS', {})",
            i32::from(archived)
        ))
        .execute(conn)
        .expect("insert test account");
    }

    #[tokio::test]
    async fn records_only_clean_complete_accounts_and_prunes_idempotently() {
        std::env::set_var("CONNECT_API_URL", "http://test.local");
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("test.db").to_string_lossy().to_string();
        run_migrations(&db_path).expect("migrations");
        let pool = create_pool(&db_path).expect("pool");
        let writer = spawn_writer((*pool).clone()).expect("writer");
        {
            let mut conn = get_connection(&pool).expect("connection");
            insert_account(&mut conn, "complete", false);
            insert_account(&mut conn, "incomplete", false);
            insert_account(&mut conn, "archived", true);
            insert_account(&mut conn, "other_currency", false);
            // A fresh profile is initially dirty until its first projection.
            sql_query("UPDATE projection_state SET dirty_from = NULL")
                .execute(&mut conn)
                .expect("clear initial projection marker");
        }

        let repository = ValuationRepository::new(pool.clone(), writer);
        repository
            .replace_valuations_for_account(
                "complete",
                None,
                &[valuation("complete", "USD", ValuationStatus::Complete)],
            )
            .await
            .expect("complete valuation");
        repository
            .replace_valuations_for_account(
                "incomplete",
                None,
                &[valuation(
                    "incomplete",
                    "USD",
                    ValuationStatus::PartialUnpriced,
                )],
            )
            .await
            .expect("incomplete valuation");
        repository
            .replace_valuations_for_account(
                "archived",
                None,
                &[valuation("archived", "USD", ValuationStatus::Complete)],
            )
            .await
            .expect("archived valuation");
        repository
            .replace_valuations_for_account(
                "other_currency",
                None,
                &[valuation(
                    "other_currency",
                    "CAD",
                    ValuationStatus::Complete,
                )],
            )
            .await
            .expect("other-currency valuation");

        let first = Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap();
        let second = first + chrono::Duration::minutes(5);
        repository
            .record_intraday_sample(first, date(), "USD".to_string(), first)
            .await
            .expect("first sample");
        repository
            .record_intraday_sample(first, date(), "USD".to_string(), first)
            .await
            .expect("idempotent sample");

        let samples = repository
            .get_intraday_samples(
                &[
                    "complete".to_string(),
                    "incomplete".to_string(),
                    "archived".to_string(),
                    "other_currency".to_string(),
                ],
                "USD",
                first,
            )
            .expect("read first samples");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].account_id, "complete");
        assert_eq!(samples[0].point.total_value_base, Decimal::new(12345, 2));

        let anchors = repository
            .get_historical_valuations_for_accounts(
                &["complete".into(), "other_currency".into()],
                Some(date()),
                Some(date()),
            )
            .expect("read seed anchors");
        let seed = |account_id: &str| ReconstructedIntradaySample {
            anchor_date: date(),
            sample: IntradayAccountSample {
                account_id: account_id.into(),
                point: IntradayValuationPoint {
                    timestamp: second,
                    total_value_base: Decimal::ONE,
                    net_contribution_base: Decimal::ZERO,
                },
            },
        };
        assert_eq!(
            repository
                .seed_intraday_samples(anchors.clone(), vec![seed("complete")], "USD".into(), first)
                .await
                .expect("preserve the observed USD session"),
            0
        );
        assert_eq!(
            repository
                .seed_intraday_samples(anchors, vec![seed("other_currency")], "CAD".into(), first)
                .await
                .expect("seed the unsampled CAD session"),
            1
        );

        {
            let mut conn = get_connection(&pool).expect("connection");
            sql_query(
                "INSERT INTO projection_state (dirty_from, scope, version) VALUES (?, ?, 1) \
                 ON CONFLICT(scope) DO UPDATE SET dirty_from = excluded.dirty_from",
            )
            .bind::<Text, _>(date().to_string())
            .bind::<Text, _>("complete")
            .execute(&mut conn)
            .expect("mark account dirty");
        }
        repository
            .record_intraday_sample(second, date(), "USD".to_string(), first)
            .await
            .expect("dirty sample");
        let samples = repository
            .get_intraday_samples(&["complete".to_string()], "USD", first)
            .expect("read dirty samples");
        assert_eq!(samples.len(), 1, "dirty account must not be captured");

        {
            let mut conn = get_connection(&pool).expect("connection");
            sql_query("UPDATE projection_state SET dirty_from = NULL WHERE scope = ?")
                .bind::<Text, _>("complete")
                .execute(&mut conn)
                .expect("clear account dirty marker");
        }
        let mut changed = valuation("complete", "USD", ValuationStatus::Complete);
        changed.total_value_base += Decimal::ONE;
        repository
            .replace_valuations_for_account("complete", None, &[changed])
            .await
            .expect("changed daily valuation");
        repository
            .record_intraday_sample(second, date(), "USD".to_string(), first)
            .await
            .expect("second sample");
        let ids = ["complete".to_string()];
        let samples = repository
            .get_intraday_samples(&ids, "USD", first)
            .expect("read both observations");
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].point.total_value_base, Decimal::new(12345, 2));
        assert_eq!(samples[1].point.total_value_base, Decimal::new(12445, 2));

        let third = second + chrono::Duration::minutes(5);
        repository
            .replace_valuations_for_account(
                "complete",
                None,
                &[valuation("complete", "EUR", ValuationStatus::Complete)],
            )
            .await
            .expect("base currency changed");
        repository
            .record_intraday_sample(third, date(), "EUR".to_string(), second)
            .await
            .expect("new base currency sample and prune");
        let samples = repository
            .get_intraday_samples(&ids, "USD", first)
            .expect("read retained samples");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].point.timestamp, second);
        let samples = repository
            .get_intraday_samples(&ids, "EUR", first)
            .expect("read new currency samples");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].point.timestamp, third);
    }
}
