//! Fork: MCP client compatibility layers around the upstream `/mcp` service.

use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig,
};

/// Disables rmcp's SEP-1699 empty-data SSE priming event (`data: \nretry: 3000`)
/// on `initialize` and standalone `GET /mcp` streams so clients that parse
/// every `data:` frame as JSON-RPC (such as Gemini's `2025-11-25` MCP client)
/// do not fail handshake.
pub(super) fn configure_http(config: StreamableHttpServerConfig) -> StreamableHttpServerConfig {
    config.with_sse_retry(None)
}

/// Builds the session manager with request-wise SSE priming disabled.
///
/// `LocalSessionManager::default()` keeps its own `SessionConfig::sse_retry`
/// (`Some(3s)`), which prepends an empty `data:` priming frame on every
/// post-`initialize` session `POST /mcp` stream (`tools/list`, `tools/call`)
/// independently of [`StreamableHttpServerConfig::sse_retry`].
pub(crate) fn session_manager() -> LocalSessionManager {
    let mut manager = LocalSessionManager::default();
    manager.session_config.sse_retry = None;
    manager
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
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    use tower::ServiceExt;

    use super::*;

    #[derive(Clone, Default)]
    struct DummyHandler;

    impl ServerHandler for DummyHandler {}

    fn mcp_app(sessions: LocalSessionManager) -> Router {
        let config = configure_http(
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .disable_allowed_hosts(),
        );
        let service = StreamableHttpService::new(|| Ok(DummyHandler), Arc::new(sessions), config);
        Router::new().nest_service("/mcp", service)
    }

    fn post_request(
        session_id: Option<&str>,
        body: serde_json::Value,
    ) -> axum::http::Request<Body> {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, "wealth.example.com")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(session_id) = session_id {
            builder = builder
                .header("mcp-session-id", session_id)
                .header("mcp-protocol-version", "2025-11-25");
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    fn assert_single_jsonrpc_sse_event(sse: &str) -> serde_json::Value {
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
        serde_json::from_str(data_lines[0]).unwrap()
    }

    #[test]
    fn disables_sse_retry_on_http_and_session_configs() {
        assert!(StreamableHttpServerConfig::default().sse_retry.is_some());
        assert!(LocalSessionManager::default()
            .session_config
            .sse_retry
            .is_some());

        let configured_http = configure_http(StreamableHttpServerConfig::default());
        assert!(configured_http.sse_retry.is_none());
        assert!(session_manager().session_config.sse_retry.is_none());
    }

    #[tokio::test]
    async fn default_session_manager_still_emits_priming_on_tools_list() {
        let app = mcp_app(LocalSessionManager::default());
        let init_resp = app
            .clone()
            .oneshot(post_request(
                None,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": { "name": "mcp", "version": "0.1.0" }
                    }
                }),
            ))
            .await
            .unwrap();
        let session_id = init_resp
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let _ = to_bytes(init_resp.into_body(), usize::MAX).await.unwrap();

        let notify = app
            .clone()
            .oneshot(post_request(
                Some(&session_id),
                serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            ))
            .await
            .unwrap();
        assert_eq!(notify.status(), StatusCode::ACCEPTED);

        let list_resp = app
            .oneshot(post_request(
                Some(&session_id),
                serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            ))
            .await
            .unwrap();
        let bytes = to_bytes(list_resp.into_body(), usize::MAX).await.unwrap();
        let sse = std::str::from_utf8(&bytes).unwrap();
        let first_data = sse
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .map(str::trim);
        assert_eq!(first_data, Some(""));
    }

    #[tokio::test]
    async fn session_handshake_and_requests_omit_empty_priming_events() {
        let app = mcp_app(session_manager());

        let init_resp = app
            .clone()
            .oneshot(post_request(
                None,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": { "name": "mcp", "version": "0.1.0" }
                    }
                }),
            ))
            .await
            .unwrap();
        assert_eq!(init_resp.status(), StatusCode::OK);
        let session_id = init_resp
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let init_bytes = to_bytes(init_resp.into_body(), usize::MAX).await.unwrap();
        let init_json = assert_single_jsonrpc_sse_event(std::str::from_utf8(&init_bytes).unwrap());
        assert_eq!(init_json["result"]["protocolVersion"], "2025-11-25");

        let notify = app
            .clone()
            .oneshot(post_request(
                Some(&session_id),
                serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            ))
            .await
            .unwrap();
        assert_eq!(notify.status(), StatusCode::ACCEPTED);

        let list_resp = app
            .oneshot(post_request(
                Some(&session_id),
                serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            ))
            .await
            .unwrap();
        assert_eq!(list_resp.status(), StatusCode::OK);
        let list_bytes = to_bytes(list_resp.into_body(), usize::MAX).await.unwrap();
        let list_json = assert_single_jsonrpc_sse_event(std::str::from_utf8(&list_bytes).unwrap());
        assert!(list_json["result"]["tools"].is_array());
    }
}
