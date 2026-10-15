//! Read-only market quote lookup for MCP callers.

use std::sync::Arc;

use serde::Deserialize;
use wealthfolio_core::quotes::MarketQuoteSnapshot;

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};

const MAX_SYMBOL_LENGTH: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetMarketQuoteArgs {
    symbol: String,
}

/// Normalize and validate a raw Yahoo Finance symbol.
///
/// Yahoo symbols may include exchange suffixes and special forms such as
/// `BRK.B`, `EURUSD=X`, and `^GSPC`, so the accepted alphabet is deliberately
/// small but broader than ordinary ticker symbols.
fn normalize_symbol(raw: &str) -> Result<String, AgentToolError> {
    let symbol = raw.trim();
    if symbol.is_empty() {
        return Err(AgentToolError::InvalidInput(
            "symbol must not be empty".to_string(),
        ));
    }
    if symbol.len() > MAX_SYMBOL_LENGTH {
        return Err(AgentToolError::InvalidInput(format!(
            "symbol must be at most {MAX_SYMBOL_LENGTH} ASCII characters"
        )));
    }
    if !symbol.is_ascii() {
        return Err(AgentToolError::InvalidInput(
            "symbol must contain only ASCII characters".to_string(),
        ));
    }

    let mut chars = symbol.chars();
    let first = chars.next().expect("empty symbol checked above");
    if !first.is_ascii_alphanumeric() && first != '^' {
        return Err(AgentToolError::InvalidInput(
            "symbol must start with a letter, digit, or ^".to_string(),
        ));
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '=' | '^' | '_')) {
        return Err(AgentToolError::InvalidInput(
            "symbol contains unsupported characters".to_string(),
        ));
    }

    Ok(symbol.to_ascii_uppercase())
}

fn parse_symbol(args: serde_json::Value) -> Result<String, AgentToolError> {
    let args: GetMarketQuoteArgs = serde_json::from_value(args)?;
    normalize_symbol(&args.symbol)
}

/// Fetch one quote without adding an asset or changing portfolio state.
pub struct GetMarketQuote;

#[async_trait::async_trait]
impl AgentTool for GetMarketQuote {
    fn name(&self) -> &'static str {
        "get_market_quote"
    }

    fn description(&self) -> &'static str {
        "Get a Yahoo quote for an exact raw Yahoo symbol, including its exchange suffix: FINN.NE is Fidelity on Cboe Canada (NEOE), while FINN is a different US instrument. Tracked symbols use stored quotes first without refreshing the portfolio; other symbols use a shared cached Yahoo lookup with a bounded agent allowance and may return an older quote with cacheStatus and note. Inspect quoteTime, fetchedAt, and currency. Does not add assets or transactions."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "symbol": {
                    "type": "string",
                    "maxLength": MAX_SYMBOL_LENGTH,
                    "pattern": "^[A-Za-z0-9^][A-Za-z0-9.\\-=^_]*$",
                    "description": "Raw Yahoo Finance symbol, including exchange suffixes such as SHOP.TO, crypto pairs such as BTC-USD, FX such as EURUSD=X, or indices such as ^GSPC."
                }
            },
            "required": ["symbol"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::HoldingsRead]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Read
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let symbol = parse_symbol(args)?;
        let snapshot: MarketQuoteSnapshot = env
            .quote_service()
            .get_market_quote(&symbol)
            .await
            .map_err(|error| AgentToolError::ExecutionFailed(error.to_string()))?;

        Ok(AgentToolResult {
            content: serde_json::to_value(snapshot)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_symbol_and_rejects_unknown_fields() {
        let schema = GetMarketQuote.input_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], serde_json::json!(["symbol"]));
        assert_eq!(
            schema["properties"]["symbol"]["maxLength"],
            MAX_SYMBOL_LENGTH
        );
    }

    #[test]
    fn raw_yahoo_symbols_are_trimmed_and_uppercased() {
        for (raw, expected) in [
            (" shop.to ", "SHOP.TO"),
            ("brk.b", "BRK.B"),
            ("btc-usd", "BTC-USD"),
            ("eurusd=x", "EURUSD=X"),
            ("^gspc", "^GSPC"),
        ] {
            assert_eq!(normalize_symbol(raw).unwrap(), expected);
        }
    }

    #[test]
    fn runtime_validation_rejects_empty_oversized_and_unsafe_symbols() {
        let oversized = "A".repeat(MAX_SYMBOL_LENGTH + 1);
        for args in [
            serde_json::json!({"symbol": ""}),
            serde_json::json!({"symbol": "   "}),
            serde_json::json!({"symbol": oversized}),
            serde_json::json!({"symbol": "A B"}),
            serde_json::json!({"symbol": "https://example.com"}),
            serde_json::json!({"symbol": "../A"}),
            serde_json::json!({"symbol": "A/B"}),
            serde_json::json!({"symbol": "Å"}),
        ] {
            assert!(matches!(
                parse_symbol(args),
                Err(AgentToolError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn unknown_arguments_are_rejected_before_symbol_validation() {
        assert!(matches!(
            parse_symbol(serde_json::json!({"symbol": "A", "refresh": true})),
            Err(AgentToolError::InvalidArgs(_))
        ));
    }
}
