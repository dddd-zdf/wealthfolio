//! Term deposits (GICs): value worked out from the deposit's terms.
//!
//! The terms live in `Asset.metadata["deposit"]`. Quote sync turns them into
//! daily prices (see `quotes::sync`), so holdings, history and performance use
//! them like any other price.

use chrono::{Months, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

/// Metadata key holding a [`DepositSpec`].
pub const DEPOSIT_METADATA_KEY: &str = "deposit";

/// How often interest is added to the balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DepositCompounding {
    Annual,
    SemiAnnual,
    Quarterly,
    Monthly,
    /// Simple interest, all paid at maturity.
    AtMaturity,
}

impl DepositCompounding {
    fn period_months(self) -> Option<u32> {
        match self {
            Self::Annual => Some(12),
            Self::SemiAnnual => Some(6),
            Self::Quarterly => Some(3),
            Self::Monthly => Some(1),
            Self::AtMaturity => None,
        }
    }
}

/// Term deposit terms stored in `Asset.metadata["deposit"]`.
///
/// The value is for the whole deposit, so the holding's quantity is 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepositSpec {
    pub principal: Decimal,
    /// Annual rate as a fraction (e.g. 0.0395 = 3.95%).
    pub annual_rate: Decimal,
    pub start_date: NaiveDate,
    pub maturity_date: NaiveDate,
    pub compounding: DepositCompounding,
}

impl DepositSpec {
    /// Checks the terms make sense before they are saved.
    pub fn validate(&self) -> Result<(), String> {
        if self.principal <= Decimal::ZERO {
            return Err("principal must be positive".to_string());
        }
        if self.annual_rate < Decimal::ZERO || self.annual_rate >= Decimal::ONE {
            return Err("annualRate must be a fraction between 0 and 1 (e.g. 0.0395)".to_string());
        }
        if self.maturity_date <= self.start_date {
            return Err("maturityDate must be after startDate".to_string());
        }
        Ok(())
    }

    /// Value of the deposit at the end of `date`, rounded to cents.
    ///
    /// Interest compounds on each anniversary of the start date at the chosen
    /// frequency (rounded to cents, as banks post it) and accrues daily on a
    /// 365-day year in between. After maturity the value stays at the maturity
    /// value. Returns `None` before the start date.
    pub fn value_on(&self, date: NaiveDate) -> Option<Decimal> {
        if date < self.start_date {
            return None;
        }
        let date = date.min(self.maturity_date);
        let mut balance = self.principal;
        let mut last_posting = self.start_date;

        if let Some(months) = self.compounding.period_months() {
            let period_rate = self.annual_rate * Decimal::from(months) / Decimal::from(12);
            let mut k = 1;
            while let Some(next) = self.start_date.checked_add_months(Months::new(months * k)) {
                if next > date {
                    break;
                }
                balance = round_cents(balance * (Decimal::ONE + period_rate));
                last_posting = next;
                k += 1;
            }
        }

        let days = Decimal::from((date - last_posting).num_days());
        let accrued = balance * self.annual_rate * days / Decimal::from(365);
        Some(round_cents(balance + accrued))
    }
}

fn round_cents(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn mcan(compounding: DepositCompounding) -> DepositSpec {
        DepositSpec {
            principal: dec!(41464),
            annual_rate: dec!(0.0395),
            start_date: day(2025, 2, 4),
            maturity_date: day(2027, 2, 4),
            compounding,
        }
    }

    #[test]
    fn annual_compounding_posts_on_anniversaries_and_accrues_between() {
        let spec = mcan(DepositCompounding::Annual);
        assert_eq!(spec.value_on(day(2025, 2, 3)), None);
        assert_eq!(spec.value_on(day(2025, 2, 4)), Some(dec!(41464)));
        // One year: 41464 * 1.0395 = 43101.828 -> 43101.83
        assert_eq!(spec.value_on(day(2026, 2, 4)), Some(dec!(43101.83)));
        // 245 days into year two: 43101.83 * 0.0395 * 245 / 365 = 1142.79
        assert_eq!(spec.value_on(day(2026, 10, 7)), Some(dec!(44244.62)));
        // Maturity: 43101.83 * 1.0395 = 44804.35
        assert_eq!(spec.value_on(day(2027, 2, 4)), Some(dec!(44804.35)));
        assert_eq!(spec.value_on(day(2028, 1, 1)), Some(dec!(44804.35)));
    }

    #[test]
    fn simple_interest_at_maturity() {
        let spec = mcan(DepositCompounding::AtMaturity);
        // 41464 * 0.0395 * 730 / 365 = 3275.656
        assert_eq!(spec.value_on(day(2027, 2, 4)), Some(dec!(44739.66)));
    }

    #[test]
    fn monthly_compounding_beats_annual() {
        let monthly = mcan(DepositCompounding::Monthly)
            .value_on(day(2027, 2, 4))
            .unwrap();
        let annual = mcan(DepositCompounding::Annual)
            .value_on(day(2027, 2, 4))
            .unwrap();
        assert!(monthly > annual);
    }

    #[test]
    fn month_end_start_keeps_compounding() {
        let spec = DepositSpec {
            principal: dec!(1000),
            annual_rate: dec!(0.12),
            start_date: day(2025, 1, 31),
            maturity_date: day(2025, 4, 30),
            compounding: DepositCompounding::Monthly,
        };
        // Postings on Feb 28, Mar 31, Apr 30: 1000 * 1.01^3 = 1030.30
        assert_eq!(spec.value_on(day(2025, 4, 30)), Some(dec!(1030.30)));
    }

    #[test]
    fn parses_metadata_and_rejects_bad_terms() {
        let spec: DepositSpec = serde_json::from_value(serde_json::json!({
            "principal": "41464",
            "annualRate": "0.0395",
            "startDate": "2025-02-04",
            "maturityDate": "2027-02-04",
            "compounding": "ANNUAL"
        }))
        .unwrap();
        assert_eq!(spec, mcan(DepositCompounding::Annual));
        assert!(spec.validate().is_ok());
        let round_trip: DepositSpec =
            serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
        assert_eq!(
            round_trip.value_on(day(2026, 10, 7)),
            spec.value_on(day(2026, 10, 7))
        );

        let mut bad = spec.clone();
        bad.annual_rate = dec!(3.95);
        assert!(bad.validate().is_err());
        bad = spec.clone();
        bad.maturity_date = bad.start_date;
        assert!(bad.validate().is_err());
    }
}
