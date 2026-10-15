//! Opt-in upstream smoke test; never reads portfolio data or credentials.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]
use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use wealthfolio_market_data::{
    InstrumentId, MarketDataProvider, ProviderInstrument, QuoteContext, QuoteIdentifiers,
    YahooProvider,
};

fn context(instrument: InstrumentId) -> QuoteContext {
    QuoteContext {
        instrument,
        identifiers: QuoteIdentifiers::default(),
        overrides: None,
        currency_hint: Some("USD".into()),
        preferred_provider: None,
        bond_metadata: None,
        custom_provider_code: None,
    }
}

#[tokio::test]
#[ignore = "one live Yahoo symbol lookup; run explicitly when changing quote transport"]
async fn yahoo_cboe_quote_cache_survives_provider_reconstruction() {
    let ctx = context(InstrumentId::Equity {
        ticker: "FINN".into(),
        mic: Some("NEOE".into()),
    });
    let instrument = ProviderInstrument::EquitySymbol {
        symbol: "FINN.NE".into(),
    };
    let first = YahooProvider::new()
        .await
        .unwrap()
        .get_latest_quote(&ctx, instrument.clone())
        .await
        .unwrap();
    assert!(first.close > Decimal::ZERO);
    assert_eq!(first.currency, "CAD");
    let fetched_at = YahooProvider::cached_quote("FINN.NE").unwrap().fetched_at;
    let second = YahooProvider::new()
        .await
        .unwrap()
        .get_latest_quote(&ctx, instrument)
        .await
        .unwrap();
    assert_eq!(second.close, first.close);
    assert_eq!(second.timestamp, first.timestamp);
    assert_eq!(
        YahooProvider::cached_quote("FINN.NE").unwrap().fetched_at,
        fetched_at
    );
    eprintln!(
        "FINN.NE: CAD, quote time {}, reconstructed provider reused fetch at {}",
        first.timestamp, fetched_at
    );
}

#[tokio::test]
#[ignore = "requires live Yahoo HTTPS access; run explicitly for dependency upgrades"]
async fn yahoo_search_quotes_history_and_corporate_actions() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let provider = YahooProvider::new().await.unwrap();
        assert!(!provider.search("AAPL").await.unwrap().is_empty());
        let cases = [
            (
                InstrumentId::Equity {
                    ticker: "AAPL".into(),
                    mic: None,
                },
                ProviderInstrument::EquitySymbol {
                    symbol: "AAPL".into(),
                },
            ),
            (
                InstrumentId::Crypto {
                    base: "BTC".into(),
                    quote: "USD".into(),
                },
                ProviderInstrument::CryptoSymbol {
                    symbol: "BTC-USD".into(),
                },
            ),
            (
                InstrumentId::Fx {
                    base: "EUR".into(),
                    quote: "USD".into(),
                },
                ProviderInstrument::FxSymbol {
                    symbol: "EURUSD=X".into(),
                },
            ),
        ];
        let end = Utc::now();
        for (instrument, params) in cases {
            let ctx = context(instrument);
            let quote = provider
                .get_latest_quote(&ctx, params.clone())
                .await
                .unwrap();
            assert!(quote.close > Decimal::ZERO);
            let history = provider
                .get_historical_quotes(&ctx, params, end - Duration::days(10), end)
                .await
                .unwrap();
            assert!(!history.is_empty());
            assert!(history.iter().all(|quote| quote.close > Decimal::ZERO));
        }
        let ctx = context(InstrumentId::Equity {
            ticker: "AAPL".into(),
            mic: None,
        });
        let params = ProviderInstrument::EquitySymbol {
            symbol: "AAPL".into(),
        };
        // Fixed past windows contain known Apple dividend and split events.
        let start = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap();
        assert!(!provider
            .get_dividends(&ctx, params.clone(), start, end)
            .await
            .unwrap()
            .is_empty());
        assert!(!provider
            .get_splits(&ctx, params, start, end)
            .await
            .unwrap()
            .is_empty());
    })
    .await
    .expect("Yahoo smoke test exceeded its time budget");
}
