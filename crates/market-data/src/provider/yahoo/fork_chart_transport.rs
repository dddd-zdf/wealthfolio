//! One explicit chart request per permit. The library's chart methods silently
//! retry some failures; reuse its parser without bypassing our request pacing.

use yahoo_finance_api::{YResponse, YahooError};

use super::fork_request_policy::{global_policy, http_client};

pub(super) enum Window<'a> {
    Range(&'a str),
    Period { start: i64, end: i64 },
}

/// Caller holds the global request permit until this response is observed.
pub(super) async fn chart(
    symbol: &str,
    interval: &str,
    window: Window<'_>,
) -> Result<YResponse, YahooError> {
    let client = http_client().map_err(|_| YahooError::BuilderFailed)?;
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        urlencoding::encode(symbol)
    );
    let request = client
        .get(url)
        .header(reqwest::header::USER_AGENT, super::YAHOO_USER_AGENT)
        .query(&[
            ("symbol", symbol),
            ("interval", interval),
            ("events", "div|split|capitalGains"),
        ]);
    let request = match window {
        Window::Range(range) => request.query(&[("range", range)]),
        Window::Period { start, end } => request.query(&[("period1", start), ("period2", end)]),
    };
    let response = request.send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        global_policy()
            .note_rate_limited(super::retry_after(&response))
            .await;
        return Err(YahooError::TooManyRequests("chart lookup".into()));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(YahooError::Unauthorized);
    }
    if status.is_server_error() {
        return Err(YahooError::ServerError(status.as_str().into()));
    }
    let text = response.text().await?;
    decode_chart(&text)
}

fn decode_chart(text: &str) -> Result<YResponse, YahooError> {
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return Err(YahooError::EmptyResponse);
    }
    if text.starts_with('<') {
        return Err(YahooError::HtmlResponse);
    }
    if !text.starts_with('{') && text.to_ascii_lowercase().contains("too many requests") {
        return Err(YahooError::TooManyRequests("chart lookup".into()));
    }
    let response = YResponse::from_json(serde_json::from_str(text)?)?;
    if response.chart.result.as_ref().is_none_or(Vec::is_empty) {
        if let Some(error) = response.chart.error {
            return Err(YahooError::ApiError(error));
        }
        return Err(YahooError::NoResult);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_error_is_preserved_for_existing_symbol_and_range_classification() {
        let error = decode_chart(r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#).unwrap_err();
        assert!(matches!(error, YahooError::ApiError(_)));
        assert!(error.to_string().contains("No data found"));
        assert!(matches!(
            decode_chart("Too Many Requests"),
            Err(YahooError::TooManyRequests(_))
        ));
        assert!(matches!(
            decode_chart("<html>maintenance</html>"),
            Err(YahooError::HtmlResponse)
        ));
        assert!(matches!(decode_chart(""), Err(YahooError::EmptyResponse)));
    }
}
