//! Fork: accounts the mixed-scope combined percentage can leave out of its
//! denominator without making the whole percentage unavailable.
//!
//! - A dormant account (zero value, zero gain) adds nothing to either side.
//! - A transaction-mode account has exact cash flows, so a gain earned from a
//!   zero opening balance (a few cents of interest in an emptied cash account)
//!   is real P&L: it adds to the numerator and nothing to the denominator.
//!
//! Holdings accounts with an amount but no denominator still make the combined
//! percentage unavailable, as upstream does.

use rust_decimal::Decimal;

use super::{ComponentMetrics, MixedComponent};
use crate::model::TrackingMode;

/// Whether a component without a return denominator can be skipped instead of
/// marking the combined percentage incomplete.
pub(super) fn skip_missing_denominator(
    component: &ComponentMetrics,
    components: &[MixedComponent],
) -> bool {
    if component.amount == Some(Decimal::ZERO) && !component.contributes_to_scope {
        return true;
    }
    component.amount.is_some()
        && components.iter().any(|candidate| {
            candidate.account == component.account && candidate.tracking != TrackingMode::Holdings
        })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::super::{build_mixed_result, ComponentMetrics, MixedComponent};
    use crate::model::*;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, day).expect("valid test date")
    }

    fn component(account: &str, tracking: TrackingMode) -> MixedComponent {
        MixedComponent {
            account: AccountId::new(account),
            tracking,
            history: Vec::new(),
        }
    }

    fn metrics(
        account: &str,
        amount: Decimal,
        denominator: Option<Decimal>,
        contributes_to_scope: bool,
    ) -> ComponentMetrics {
        ComponentMetrics {
            account: AccountId::new(account),
            start_date: date(12),
            end_date: date(19),
            amount: Some(amount),
            denominator,
            contributes_to_scope,
            basis_status: BasisStatus::NotApplicable,
            attribution: Attribution::default(),
            warnings: Vec::new(),
            reasons: Vec::new(),
        }
    }

    fn combined_percent(
        components: &[MixedComponent],
        metrics: Vec<ComponentMetrics>,
    ) -> PerformanceResult {
        let currency = Currency::parse("CAD").expect("valid currency");
        build_mixed_result(
            components,
            metrics,
            Vec::new(),
            &currency,
            Some(date(12)),
            false,
            false,
        )
    }

    #[test]
    fn dormant_account_does_not_hide_the_combined_percent() {
        let components = [
            component("zero-account", TrackingMode::Transactions),
            component("holdings", TrackingMode::Holdings),
        ];
        let result = combined_percent(
            &components,
            vec![
                metrics("zero-account", Decimal::ZERO, None, false),
                metrics("holdings", dec!(100), Some(dec!(1000)), true),
            ],
        );

        assert_eq!(result.returns.value_return, Some(dec!(0.1)));
        assert_eq!(result.summary.percent_status, SummaryStatus::Complete);
    }

    #[test]
    fn transaction_account_earning_from_zero_keeps_the_combined_percent() {
        let components = [
            component("emptied-cash", TrackingMode::Transactions),
            component("holdings", TrackingMode::Holdings),
        ];
        let result = combined_percent(
            &components,
            vec![
                metrics("emptied-cash", dec!(0.01), None, true),
                metrics("holdings", dec!(100), Some(dec!(1000)), true),
            ],
        );

        assert_eq!(result.summary.percent_status, SummaryStatus::Complete);
        let percent = result.summary.percent.expect("combined percent");
        assert!(percent > dec!(0.1) && percent < dec!(0.1001), "{percent}");
    }

    #[test]
    fn holdings_account_without_denominator_still_hides_the_combined_percent() {
        let components = [
            component("new-holdings", TrackingMode::Holdings),
            component("holdings", TrackingMode::Holdings),
        ];
        let result = combined_percent(
            &components,
            vec![
                metrics("new-holdings", dec!(50), None, true),
                metrics("holdings", dec!(100), Some(dec!(1000)), true),
            ],
        );

        assert_eq!(result.returns.value_return, None);
        assert_eq!(result.summary.percent_status, SummaryStatus::Unavailable);
    }
}
