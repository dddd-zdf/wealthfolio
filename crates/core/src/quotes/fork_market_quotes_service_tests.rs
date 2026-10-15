// Included by service::tests to reuse its existing read-only repository doubles.

#[tokio::test]
async fn market_quote_uses_exact_stored_listing_without_any_configured_provider() -> Result<()> {
    use crate::test_support::in_memory::InMemoryAssetRepository;

    let assets = vec![
        Asset {
            id: "shop-ca".into(),
            kind: AssetKind::Investment,
            quote_mode: QuoteMode::Market,
            instrument_type: Some(InstrumentType::Equity),
            instrument_symbol: Some("SHOP".into()),
            instrument_exchange_mic: Some("XTSE".into()),
            quote_ccy: "CAD".into(),
            ..Default::default()
        },
        Asset {
            id: "shop-us".into(),
            kind: AssetKind::Investment,
            quote_mode: QuoteMode::Market,
            instrument_type: Some(InstrumentType::Equity),
            instrument_symbol: Some("SHOP".into()),
            instrument_exchange_mic: Some("XNAS".into()),
            quote_ccy: "USD".into(),
            ..Default::default()
        },
    ];
    let day = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    let mut canadian = stored_quote("shop-ca", day, dec!(150.25));
    canadian.data_source = "YAHOO".into();
    canadian.currency = "CAD".into();
    let mut american = stored_quote("shop-us", day, dec!(100.5));
    american.data_source = "YAHOO".into();
    american.currency = "USD".into();
    let requested = Arc::new(Mutex::new(Vec::new()));
    let service = QuoteService::new(
        Arc::new(SnapshotQuoteStore {
            quotes: HashMap::from([
                (canadian.asset_id.clone(), canadian.clone()),
                (american.asset_id.clone(), american.clone()),
            ]),
            requested_latest_quotes: requested.clone(),
        }),
        Arc::new(MockSyncStateStore {
            provider_sync_stats: vec![],
            with_errors: vec![],
            states: Arc::new(Mutex::new(HashMap::new())),
        }),
        Arc::new(MockProviderSettingsStore { providers: vec![] }),
        Arc::new(InMemoryAssetRepository::new(assets)),
        Arc::new(NoopActivityRepository),
        Arc::new(MockSecretStore),
    )
    .await?;

    let ca = service.get_market_quote("SHOP.TO").await?;
    assert_eq!(ca.price, "150.25");
    assert_eq!(ca.currency, "CAD");
    assert_eq!(ca.quote_time, canadian.timestamp);
    assert_eq!(ca.cache_status, "stored");
    assert!(ca.fetched_at.is_none());
    let us = service.get_market_quote("SHOP").await?;
    assert_eq!(us.price, "100.5");
    assert_eq!(us.currency, "USD");
    // No provider is configured: a network fallback would fail. All repository
    // mutation methods in these doubles panic, so this also guards read-only use.
    assert_eq!(
        *requested.lock().unwrap(),
        vec![vec!["shop-ca".to_string()], vec!["shop-us".to_string()]]
    );
    assert!(service.get_market_quote("SHOP.L").await.is_err());
    Ok(())
}

#[tokio::test]
async fn market_quote_distinguishes_finn_cboe_canada_from_bare_us_finn() -> Result<()> {
    use crate::test_support::in_memory::InMemoryAssetRepository;

    let assets = vec![
        Asset {
            id: "fidelity-ca".into(),
            kind: AssetKind::Investment,
            instrument_type: Some(InstrumentType::Equity),
            instrument_symbol: Some("FINN".into()),
            instrument_exchange_mic: Some("NEOE".into()),
            quote_ccy: "CAD".into(),
            ..Default::default()
        },
        Asset {
            id: "first-national-us".into(),
            kind: AssetKind::Investment,
            instrument_type: Some(InstrumentType::Equity),
            instrument_symbol: Some("FINN".into()),
            // Existing imports may have an incorrect asset currency. The
            // provider quote's USD must survive; never relabel its price CAD.
            quote_ccy: "CAD".into(),
            ..Default::default()
        },
    ];
    let day = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    let mut ca = stored_quote("fidelity-ca", day, dec!(33.83));
    ca.data_source = "YAHOO".into();
    ca.currency = "CAD".into();
    let mut us = stored_quote("first-national-us", day, dec!(19000));
    us.data_source = "YAHOO".into();
    us.currency = "USD".into();
    let service = QuoteService::new(
        Arc::new(SnapshotQuoteStore {
            quotes: HashMap::from([(ca.asset_id.clone(), ca), (us.asset_id.clone(), us)]),
            requested_latest_quotes: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(MockSyncStateStore {
            provider_sync_stats: vec![],
            with_errors: vec![],
            states: Arc::new(Mutex::new(HashMap::new())),
        }),
        Arc::new(MockProviderSettingsStore { providers: vec![] }),
        Arc::new(InMemoryAssetRepository::new(assets)),
        Arc::new(NoopActivityRepository),
        Arc::new(MockSecretStore),
    )
    .await?;
    let canadian = service.get_market_quote("FINN.NE").await?;
    let american = service.get_market_quote("FINN").await?;
    assert_eq!(
        (canadian.price.as_str(), canadian.currency.as_str()),
        ("33.83", "CAD")
    );
    assert_eq!(
        (american.price.as_str(), american.currency.as_str()),
        ("19000", "USD")
    );
    assert_eq!(canadian.cache_status, "stored");
    assert_eq!(american.cache_status, "stored");
    Ok(())
}
