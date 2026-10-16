//! Fork tests: manual prices not updated today show no day change
//! (`fork_day_change`). A child of the upstream test module to share its mocks.

use super::*;

fn create_manual_holding(symbol: &str) -> Holding {
    let mut holding = create_holding(
        "h1",
        HoldingType::Security,
        symbol,
        dec!(10),
        "CAD",
        "CAD",
        Some(dec!(1400.0)),
        None,
    );
    holding.instrument.as_mut().unwrap().pricing_mode = "MANUAL".to_string();
    holding
}

#[tokio::test]
async fn manual_quote_from_earlier_day_reports_zero_day_change() {
    let (_fx_service, market_data_service, valuation_service) = setup_test_env();
    let valuation_service =
        valuation_service.with_today(NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());

    let latest_quote = create_quote("2024-01-10", dec!(150.0), "CAD");
    let prev_quote = create_quote("2024-01-09", dec!(145.0), "CAD");
    market_data_service.add_quote_pair("FUND", latest_quote, Some(prev_quote));

    let mut holdings = vec![create_manual_holding("FUND")];

    valuation_service
        .calculate_holdings_live_valuation(&mut holdings)
        .await
        .unwrap();
    let holding = &holdings[0];

    assert_monetary_value_approx(
        Some(&holding.market_value),
        dec!(1500.0),
        dec!(1500.0),
        TOLERANCE,
        "Market Value",
    );
    assert_eq!(holding.day_change, Some(MonetaryValue::zero()));
    assert_eq!(holding.day_change_pct, Some(Decimal::ZERO));
    assert_monetary_value_approx(
        holding.prev_close_value.as_ref(),
        dec!(1500.0),
        dec!(1500.0),
        TOLERANCE,
        "Prev Close Value",
    );
}

#[tokio::test]
async fn manual_quote_entered_today_reports_change_since_previous_quote() {
    let (_fx_service, market_data_service, valuation_service) = setup_test_env();

    // Previous manual update was days earlier; the whole move shows on the update day.
    let latest_quote = create_quote("2024-01-10", dec!(150.0), "CAD");
    let prev_quote = create_quote("2024-01-03", dec!(145.0), "CAD");
    market_data_service.add_quote_pair("FUND", latest_quote, Some(prev_quote));

    let mut holdings = vec![create_manual_holding("FUND")];

    valuation_service
        .calculate_holdings_live_valuation(&mut holdings)
        .await
        .unwrap();

    assert_monetary_value_approx(
        holdings[0].day_change.as_ref(),
        dec!(50.0),
        dec!(50.0),
        TOLERANCE,
        "Day Change",
    );
}

#[tokio::test]
async fn market_quote_from_last_session_still_reports_day_change() {
    let (_fx_service, market_data_service, valuation_service) = setup_test_env();
    // Fri 2024-01-12 close, viewed on Mon 2024-01-15 before the next close.
    let valuation_service =
        valuation_service.with_today(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap());

    let latest_quote = create_quote("2024-01-12", dec!(150.0), "CAD");
    let prev_quote = create_quote("2024-01-11", dec!(145.0), "CAD");
    market_data_service.add_quote_pair("XYZ.TO", latest_quote, Some(prev_quote));

    let mut holdings = vec![create_holding(
        "h1",
        HoldingType::Security,
        "XYZ.TO",
        dec!(10),
        "CAD",
        "CAD",
        Some(dec!(1400.0)),
        None,
    )];

    valuation_service
        .calculate_holdings_live_valuation(&mut holdings)
        .await
        .unwrap();

    assert_monetary_value_approx(
        holdings[0].day_change.as_ref(),
        dec!(50.0),
        dec!(50.0),
        TOLERANCE,
        "Day Change",
    );
}
