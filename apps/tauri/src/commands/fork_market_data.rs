//! Fork: market data commands kept out of upstream's `market_data.rs`.

use wealthfolio_core::portfolio::valuation::intraday::{asset_intraday_bars, IntradayRange};
use wealthfolio_core::quotes::Quote;

use crate::profiles::ProfileAccess;

/// One asset's live 1D/1W price bars, oldest first (not stored).
#[tauri::command]
pub async fn get_asset_intraday_quotes(
    asset_id: String,
    range: String,
    state: ProfileAccess,
) -> Result<Vec<Quote>, String> {
    let context = state.context()?;
    let range =
        IntradayRange::parse(&range).ok_or_else(|| format!("Unsupported range: {}", range))?;
    asset_intraday_bars(context.quote_service().as_ref(), &asset_id, range)
        .await
        .map_err(|e| e.to_string())
}
