//! Fork: MCP client compatibility layers around the upstream `/mcp` service.

use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};

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
pub(super) async fn resource_challenge(req: Request, next: Next) -> Response {
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
