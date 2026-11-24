//! Fork: MCP client compatibility layers around the upstream `/mcp` service.

use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;

/// Applies fork-specific HTTP transport compatibility settings.
///
/// Disables rmcp's default SEP-1699 empty-data SSE priming event (`sse_retry`),
/// which causes clients such as Gemini's `2025-11-25` MCP client to fail when
/// parsing the initial empty `data:` frame on `initialize`.
pub(super) fn configure_http(config: StreamableHttpServerConfig) -> StreamableHttpServerConfig {
    config.with_sse_retry(None)
}

/// rmcp answers a session-less request that isn't `initialize` with 422.
/// Clients speaking MCP 2026-07-28 (e.g. ChatGPT, which opens with
/// `server/discover`) only fall back to the `initialize` handshake on a 400
/// without a modern JSON-RPC error body, so report it as 400.
pub(super) async fn legacy_fallback_status(mut response: Response) -> Response {
    if response.status() == StatusCode::UNPROCESSABLE_ENTITY {
        *response.status_mut() = StatusCode::BAD_REQUEST;
    }
    response
}

/// Points OAuth-capable clients at the protected-resource metadata when the
/// PAT check rejects a request.
pub(crate) async fn resource_challenge(req: Request, next: Next) -> Response {
    let challenge = format!(
        r#"Bearer resource_metadata="{}""#,
        super::oauth::resource_metadata_url(req.headers())
    );
    let mut response = next.run(req).await;
    if response.status() == StatusCode::UNAUTHORIZED {
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::Router;
    use rmcp::handler::server::ServerHandler;
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    use tower::ServiceExt;

    use super::*;

    #[derive(Clone, Default)]
    struct DummyHandler;

    impl ServerHandler for DummyHandler {}

    #[test]
    fn configure_http_disables_sse_retry_priming_event() {
        let default = StreamableHttpServerConfig::default();
        assert!(default.sse_retry.is_some());

        let configured = configure_http(default);
        assert!(configured.sse_retry.is_none());
    }

    #[tokio::test]
    async fn initialize_response_omits_empty_priming_event() {
        let config = configure_http(
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .disable_allowed_hosts(),
        );
        let service = StreamableHttpService::new(
            || Ok(DummyHandler),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let app = Router::new().nest_service("/mcp", service);

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "mcp", "version": "0.1.0" }
            }
        });
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, "wealth.example.com")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("mcp-session-id"));

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let sse = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            !sse.contains("retry:"),
            "unexpected SSE priming retry line: {sse}"
        );

        let data_lines: Vec<&str> = sse
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .collect();
        assert_eq!(data_lines.len(), 1, "expected single JSON-RPC event: {sse}");
        assert!(
            !data_lines[0].is_empty(),
            "first SSE data event must not be empty: {sse}"
        );
        let parsed: serde_json::Value = serde_json::from_str(data_lines[0]).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2025-11-25");
    }
}
