//! Fork: negative-balance health input ignores precision residues.
//!
//! Cash and total-value residues below half the account currency's smallest
//! unit are not deficits, so an earlier residue cannot hide a later real one.

use rust_decimal::Decimal;
use wealthfolio_core::fx::currency::currency_minor_unit;

/// Whether a negative cash or total value is at least half a minor unit.
pub(super) fn is_meaningful_deficit(cash: Decimal, total: Decimal, currency: &str) -> bool {
    let half_minor_unit = currency_minor_unit(currency) / Decimal::TWO;
    cash <= -half_minor_unit || total <= -half_minor_unit
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};
    use diesel::sql_query;
    use diesel::RunQueryDsl;
    use rust_decimal::Decimal;
    use tempfile::tempdir;
    use wealthfolio_core::portfolio::economic_events::BasisStatus;
    use wealthfolio_core::portfolio::valuation::{
        DailyAccountValuation, ExternalFlowSource, ValuationRepositoryTrait, ValuationStatus,
    };

    use super::super::ValuationRepository;
    use crate::db::{create_pool, get_connection, run_migrations, write_actor::spawn_writer};

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 1, day).expect("valid test date")
    }

    fn valuation_with_currency(
        account: &str,
        day: u32,
        currency: &str,
        cash: Decimal,
        total: Decimal,
    ) -> DailyAccountValuation {
        DailyAccountValuation {
            id: format!("{account}_{}", date(day)),
            account_id: account.to_string(),
            valuation_date: date(day),
            account_currency: currency.to_string(),
            base_currency: currency.to_string(),
            fx_rate_to_base: Decimal::ONE,
            cash_balance: cash,
            investment_market_value: total - cash,
            total_value: total,
            cost_basis: Decimal::ZERO,
            book_basis: Decimal::ZERO,
            net_contribution: Decimal::ZERO,
            cash_balance_base: cash,
            investment_market_value_base: total - cash,
            total_value_base: total,
            cost_basis_base: Decimal::ZERO,
            book_basis_base: Decimal::ZERO,
            net_contribution_base: Decimal::ZERO,
            external_inflow_base: Decimal::ZERO,
            external_outflow_base: Decimal::ZERO,
            external_flow_source: ExternalFlowSource::NoFlow,
            performance_eligible_value_base: total,
            value_status: ValuationStatus::Complete,
            basis_status: BasisStatus::Complete,
            calculated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn negative_balance_uses_currency_precision_and_first_meaningful_date() {
        std::env::set_var("CONNECT_API_URL", "http://test.local");
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db").to_string_lossy().to_string();
        run_migrations(&db_path).unwrap();
        let pool = create_pool(&db_path).unwrap();
        let writer = spawn_writer((*pool).clone()).unwrap();
        let mut conn = get_connection(&pool).unwrap();
        let cases = vec![
            (
                "cad_residue_then_deficit",
                "CAD",
                vec![
                    (1, Decimal::new(-543, 8), Decimal::from(100)),
                    (2, Decimal::new(-1, 2), Decimal::from(100)),
                ],
            ),
            (
                "cad_residue_only",
                "CAD",
                vec![(1, Decimal::new(-543, 8), Decimal::from(100))],
            ),
            (
                "cad_half_unit",
                "CAD",
                vec![(1, Decimal::new(-5, 3), Decimal::from(100))],
            ),
            (
                "jpy_residue",
                "JPY",
                vec![(1, Decimal::new(-49, 2), Decimal::from(100))],
            ),
            (
                "jpy_half_unit",
                "JPY",
                vec![(1, Decimal::new(-5, 1), Decimal::from(100))],
            ),
            (
                "kwd_residue",
                "KWD",
                vec![(1, Decimal::new(-49, 5), Decimal::from(100))],
            ),
            (
                "kwd_half_unit",
                "KWD",
                vec![(1, Decimal::new(-5, 4), Decimal::from(100))],
            ),
            (
                "btc_residue",
                "BTC",
                vec![(1, Decimal::new(-4, 9), Decimal::from(1))],
            ),
            (
                "btc_deficit",
                "BTC",
                vec![(1, Decimal::new(-1, 8), Decimal::from(1))],
            ),
            (
                "total_deficit",
                "CAD",
                vec![(1, Decimal::from(10), Decimal::new(-1, 2))],
            ),
            (
                "zero_or_positive",
                "CAD",
                vec![
                    (1, Decimal::ZERO, Decimal::ZERO),
                    (2, Decimal::ONE, Decimal::ONE),
                ],
            ),
        ];
        for (account, currency, _) in &cases {
            sql_query(format!(
                "INSERT INTO accounts (id, name, account_type, currency, is_default, is_active, \
                 created_at, updated_at, tracking_mode, is_archived) \
                 VALUES ('{account}', '{account}', 'SECURITIES', '{currency}', 0, 1, datetime('now'), \
                 datetime('now'), 'TRANSACTIONS', 0)"
            ))
            .execute(&mut conn)
            .unwrap();
        }
        let repository = ValuationRepository::new(pool.clone(), writer);
        for (account, currency, values) in &cases {
            let records: Vec<_> = values
                .iter()
                .map(|(day, cash, total)| {
                    valuation_with_currency(account, *day, currency, *cash, *total)
                })
                .collect();
            repository
                .replace_valuations_for_account(account, None, &records)
                .await
                .unwrap();
        }

        let mut found = repository
            .get_accounts_with_negative_balance(
                &cases
                    .iter()
                    .map(|(account, _, _)| (*account).to_string())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        found.sort_by(|a, b| a.account_id.cmp(&b.account_id));

        let summary: Vec<_> = found
            .iter()
            .map(|info| {
                (
                    info.account_id.as_str(),
                    info.first_negative_date,
                    info.cash_balance,
                    info.total_value,
                    info.account_currency.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "btc_deficit",
                    date(1),
                    Decimal::new(-1, 8),
                    Decimal::from(1),
                    "BTC",
                ),
                (
                    "cad_half_unit",
                    date(1),
                    Decimal::new(-5, 3),
                    Decimal::from(100),
                    "CAD",
                ),
                (
                    "cad_residue_then_deficit",
                    date(2),
                    Decimal::new(-1, 2),
                    Decimal::from(100),
                    "CAD",
                ),
                (
                    "jpy_half_unit",
                    date(1),
                    Decimal::new(-5, 1),
                    Decimal::from(100),
                    "JPY",
                ),
                (
                    "kwd_half_unit",
                    date(1),
                    Decimal::new(-5, 4),
                    Decimal::from(100),
                    "KWD",
                ),
                (
                    "total_deficit",
                    date(1),
                    Decimal::from(10),
                    Decimal::new(-1, 2),
                    "CAD",
                ),
            ]
        );
    }
}
