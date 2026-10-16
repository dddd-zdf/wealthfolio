//! Fork: live intraday bars behind `QuoteServiceTrait::get_intraday_quotes`.
//! Bars are fetched from providers on request and never stored.

use std::collections::HashMap;

use futures::stream::{self, StreamExt};
use log::debug;
use tokio::sync::RwLock;

use super::client::MarketDataClient;
use super::model::Quote;
use crate::assets::{Asset, AssetRepositoryTrait};
use crate::errors::Result;

/// Intraday bars per asset id. Assets without market pricing (including term
/// deposits) or without intraday data are omitted.
pub(super) async fn get_intraday_quotes<A: AssetRepositoryTrait + ?Sized>(
    asset_repo: &A,
    client: &RwLock<MarketDataClient>,
    asset_ids: &[String],
    interval: &str,
    range: &str,
) -> Result<HashMap<String, Vec<Quote>>> {
    let assets: Vec<Asset> = asset_ids
        .iter()
        .filter_map(|id| asset_repo.get_by_id(id).ok())
        .filter(|asset| asset.needs_pricing() && asset.deposit_spec().is_none())
        .collect();
    let client = client.read().await;
    let client = &*client;
    let fetched: Vec<(String, Vec<Quote>)> = stream::iter(assets)
        .map(|asset| async move {
            let quotes = client
                .fetch_intraday_quotes(&asset, interval, range)
                .await
                .unwrap_or_else(|e| {
                    debug!("Intraday quotes unavailable for {}: {}", asset.id, e);
                    Vec::new()
                });
            (asset.id, quotes)
        })
        .buffer_unordered(6)
        .collect()
        .await;
    Ok(fetched
        .into_iter()
        .filter(|(_, quotes)| !quotes.is_empty())
        .collect())
}
