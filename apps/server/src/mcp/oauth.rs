//! OAuth 2.1 front door for `/mcp`, so hosted MCP clients (Claude custom
//! connectors, ChatGPT connectors) can connect without a pasted token.
//!
//! The flow ends in an ordinary Personal Access Token: approving the consent
//! screen mints a PAT in the default profile (the one `/mcp` serves when no
//! profile header is sent), and the token endpoint hands it out as the
//! access token. Existing PAT auth, scopes, audit and revocation apply
//! unchanged.
//!
//! Nothing new is persisted. Client registration is stateless (the client id
//! encodes its redirect URIs), and authorization codes live in memory for
//! [`CODE_TTL`]; a restart mid-flow just means approving again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Form, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wealthfolio_agent_tools::AgentScope;
use wealthfolio_storage_sqlite::agent::NewPersonalAccessToken;

use crate::{
    error::{ApiError, ApiResult},
    main_lib::AppState,
    mcp::auth::{generate_token, hash_token, token_prefix},
    profiles::WebProfiles,
};

/// How long an issued authorization code stays redeemable.
const CODE_TTL: Duration = Duration::from_secs(300);
const CLIENT_ID_PREFIX: &str = "wfc_";

/// Authorization codes awaiting exchange, shared by the approve and token
/// endpoints.
#[derive(Default)]
pub struct OAuthCodes(Mutex<HashMap<String, PendingCode>>);

struct PendingCode {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    access_token: String,
    scopes: Vec<String>,
    expires_at: Instant,
}

impl OAuthCodes {
    fn insert(&self, code: String, pending: PendingCode) {
        let mut codes = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        codes.retain(|_, entry| entry.expires_at > now);
        codes.insert(code, pending);
    }

    /// Removes and returns the code: each code is redeemable once.
    fn take(&self, code: &str) -> Option<PendingCode> {
        let mut codes = self.0.lock().unwrap_or_else(|p| p.into_inner());
        codes
            .remove(code)
            .filter(|entry| entry.expires_at > Instant::now())
    }
}

/// Registered client metadata, carried inside the client id itself.
#[derive(Serialize, Deserialize)]
struct ClientInfo {
    name: String,
    redirect_uris: Vec<String>,
}

fn encode_client_id(client: &ClientInfo) -> String {
    let json = serde_json::to_vec(client).expect("client info serializes");
    format!("{CLIENT_ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

fn decode_client_id(client_id: &str) -> Option<ClientInfo> {
    let encoded = client_id.strip_prefix(CLIENT_ID_PREFIX)?;
    let json = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    serde_json::from_slice(&json).ok()
}

/// Any HTTPS redirect (hosted clients such as Claude, ChatGPT and Muse use
/// their own hosts; the consent screen shows where the browser returns), or
/// plain HTTP on loopback for local clients such as Claude Code.
fn redirect_allowed(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    matches!(
        (url.scheme(), url.host_str()),
        ("https", Some(_)) | ("http", Some("localhost" | "127.0.0.1" | "[::1]"))
    )
}

/// Checks the client id and that `redirect_uri` is one it registered.
fn validate_client(client_id: &str, redirect_uri: &str) -> Option<ClientInfo> {
    decode_client_id(client_id).filter(|client| {
        client.redirect_uris.iter().any(|uri| uri == redirect_uri) && redirect_allowed(redirect_uri)
    })
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Public origin as seen by the client, honoring the reverse proxy headers.
pub(crate) fn public_origin(headers: &HeaderMap) -> String {
    let first = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let proto = first("x-forwarded-proto").unwrap_or_else(|| "http".into());
    let host = first("x-forwarded-host")
        .or_else(|| first(header::HOST.as_str()))
        .unwrap_or_else(|| "localhost".into());
    format!("{proto}://{host}")
}

/// URL of the protected-resource metadata, advertised in `WWW-Authenticate`.
pub(crate) fn resource_metadata_url(headers: &HeaderMap) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource/mcp",
        public_origin(headers)
    )
}

fn supported_scopes() -> Vec<&'static str> {
    AgentScope::ALL.iter().map(|scope| scope.as_str()).collect()
}

/// Valid requested scopes, or the read-only set when none are usable.
fn requested_or_default_scopes(requested: Option<&str>) -> Vec<String> {
    let parsed: Vec<String> = requested
        .unwrap_or_default()
        .split_whitespace()
        .filter(|s| AgentScope::parse(s).is_some())
        .map(String::from)
        .collect();
    if parsed.is_empty() {
        AgentScope::READ_SCOPES
            .iter()
            .map(|scope| scope.as_str().to_string())
            .collect()
    } else {
        parsed
    }
}

/// Public OAuth routes: discovery metadata, registration, authorize, token.
pub fn router(profiles: Arc<WebProfiles>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        .route("/oauth/register", post(register))
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/token", post(token))
        .with_state(profiles)
}

async fn protected_resource(headers: HeaderMap) -> Json<serde_json::Value> {
    let origin = public_origin(&headers);
    Json(serde_json::json!({
        "resource": format!("{origin}/mcp"),
        "authorization_servers": [origin],
        "scopes_supported": supported_scopes(),
        "bearer_methods_supported": ["header"],
        "resource_name": "Wealthfolio",
    }))
}

async fn authorization_server(headers: HeaderMap) -> Json<serde_json::Value> {
    let origin = public_origin(&headers);
    Json(serde_json::json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "registration_endpoint": format!("{origin}/oauth/register"),
        "scopes_supported": supported_scopes(),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    redirect_uris: Vec<String>,
}

/// Dynamic client registration (RFC 7591), public clients only.
async fn register(Json(request): Json<RegisterRequest>) -> Response {
    if request.redirect_uris.is_empty()
        || !request.redirect_uris.iter().all(|u| redirect_allowed(u))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "Redirect URIs must use HTTPS, or HTTP on loopback",
        );
    }
    let name = request
        .client_name
        .map(|name| name.trim().chars().take(64).collect::<String>())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "MCP client".into());
    let client = ClientInfo {
        name,
        redirect_uris: request.redirect_uris,
    };
    let client_id = encode_client_id(&client);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "client_id": client_id,
            "client_name": client.name,
            "redirect_uris": client.redirect_uris,
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    scope: Option<String>,
    state: Option<String>,
}

/// Validates the request, then hands off to the web app's consent screen,
/// which sits behind the normal login.
async fn authorize(Query(query): Query<AuthorizeQuery>) -> Response {
    let (Some(client_id), Some(redirect_uri)) = (&query.client_id, &query.redirect_uri) else {
        return (StatusCode::BAD_REQUEST, "Missing client_id or redirect_uri").into_response();
    };
    let Some(client) = validate_client(client_id, redirect_uri) else {
        // Never redirect to an unverified URI.
        return (StatusCode::BAD_REQUEST, "Unknown client or redirect URI").into_response();
    };
    let error_redirect = |error: &str| {
        let mut url = url::Url::parse(redirect_uri).expect("validated redirect URI");
        url.query_pairs_mut().append_pair("error", error);
        if let Some(state) = &query.state {
            url.query_pairs_mut().append_pair("state", state);
        }
        Redirect::to(url.as_str()).into_response()
    };
    if query.response_type.as_deref() != Some("code") {
        return error_redirect("unsupported_response_type");
    }
    let challenge = query.code_challenge.as_deref().unwrap_or_default();
    if challenge.is_empty() || query.code_challenge_method.as_deref() != Some("S256") {
        return error_redirect("invalid_request");
    }

    let scopes = requested_or_default_scopes(query.scope.as_deref());
    let mut consent = url::Url::parse("http://consent/oauth/consent").expect("static URL");
    {
        let mut pairs = consent.query_pairs_mut();
        pairs
            .append_pair("client_id", client_id)
            .append_pair("client_name", &client.name)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("code_challenge", challenge)
            .append_pair("scope", &scopes.join(" "));
        if let Some(state) = &query.state {
            pairs.append_pair("state", state);
        }
    }
    Redirect::to(&format!(
        "/oauth/consent?{}",
        consent.query().unwrap_or_default()
    ))
    .into_response()
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
}

async fn token(
    State(profiles): State<Arc<WebProfiles>>,
    Form(request): Form<TokenRequest>,
) -> Response {
    if request.grant_type != "authorization_code" {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Only authorization_code is supported",
        );
    }
    let invalid_grant = || {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "The authorization code is invalid or expired",
        )
    };
    let (Some(code), Some(verifier)) = (&request.code, &request.code_verifier) else {
        return invalid_grant();
    };
    let Some(pending) = profiles.oauth_codes.take(code) else {
        return invalid_grant();
    };
    let client_matches = request
        .client_id
        .as_ref()
        .is_none_or(|id| *id == pending.client_id);
    let redirect_matches = request
        .redirect_uri
        .as_ref()
        .is_none_or(|uri| *uri == pending.redirect_uri);
    if !client_matches || !redirect_matches || !pkce_matches(verifier, &pending.code_challenge) {
        return invalid_grant();
    }
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "access_token": pending.access_token,
            "token_type": "Bearer",
            "scope": pending.scopes.join(" "),
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApproveRequest {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    state: Option<String>,
    /// Granted scopes; `None` means the user denied the request.
    scopes: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApproveResponse {
    redirect_url: String,
}

/// Consent decision from the web app (session-authenticated). Approval mints
/// a PAT and returns the client redirect carrying a one-time code.
pub(crate) async fn approve(
    Extension(state): Extension<Arc<AppState>>,
    Extension(profiles): Extension<Arc<WebProfiles>>,
    Json(request): Json<ApproveRequest>,
) -> ApiResult<Json<ApproveResponse>> {
    let client = validate_client(&request.client_id, &request.redirect_uri)
        .ok_or_else(|| ApiError::BadRequest("Unknown client or redirect URI".into()))?;
    let mut redirect = url::Url::parse(&request.redirect_uri)
        .map_err(|_| ApiError::BadRequest("Invalid redirect URI".into()))?;

    match request.scopes {
        None => {
            redirect
                .query_pairs_mut()
                .append_pair("error", "access_denied");
        }
        Some(scopes) => {
            // `/mcp` without a profile header serves the default profile, so
            // that is where the token must live.
            let default_id = profiles
                .registry
                .default_id()
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let active_id = state.profile_binding.get().map(|(_, id)| *id);
            if active_id != Some(default_id) {
                return Err(ApiError::BadRequest(
                    "Switch to your default profile to connect an MCP client".into(),
                ));
            }
            let scopes = crate::api::agent_access::validate_requested_scopes(&scopes)?;
            let access_token = generate_token();
            let prefix = token_prefix(&access_token)
                .ok_or_else(|| ApiError::Internal("Generated token has invalid format".into()))?
                .to_string();
            state
                .pat_repository
                .create(NewPersonalAccessToken {
                    name: format!("{} (OAuth)", client.name),
                    token_prefix: prefix,
                    token_hash: hash_token(&access_token),
                    scopes_json: serde_json::to_string(&scopes)
                        .map_err(|e| ApiError::Internal(e.to_string()))?,
                    expires_at: None,
                })
                .await?;
            let code = random_token();
            profiles.oauth_codes.insert(
                code.clone(),
                PendingCode {
                    client_id: request.client_id,
                    redirect_uri: request.redirect_uri,
                    code_challenge: request.code_challenge,
                    access_token,
                    scopes,
                    expires_at: Instant::now() + CODE_TTL,
                },
            );
            redirect.query_pairs_mut().append_pair("code", &code);
        }
    }
    if let Some(state) = &request.state {
        redirect.query_pairs_mut().append_pair("state", state);
    }
    Ok(Json(ApproveResponse {
        redirect_url: redirect.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_allowlist() {
        assert!(redirect_allowed("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_allowed(
            "https://chatgpt.com/connector_platform_oauth_redirect"
        ));
        assert!(redirect_allowed("http://localhost:33418/callback"));
        assert!(redirect_allowed("http://127.0.0.1:5000/cb"));
        assert!(!redirect_allowed("http://claude.ai/api/mcp/auth_callback"));
        assert!(!redirect_allowed("http://example.com/cb"));
        assert!(!redirect_allowed("https://claude.ai/cb#frag"));
        assert!(!redirect_allowed("javascript:alert(1)"));
    }

    #[test]
    fn client_id_round_trips_and_pins_redirects() {
        let client = ClientInfo {
            name: "Claude".into(),
            redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
        };
        let id = encode_client_id(&client);
        assert!(validate_client(&id, "https://claude.ai/api/mcp/auth_callback").is_some());
        assert!(validate_client(&id, "https://claude.ai/other").is_none());
        assert!(
            validate_client("wfc_garbage", "https://claude.ai/api/mcp/auth_callback").is_none()
        );

        // A forged client id cannot smuggle in a non-HTTPS redirect.
        let forged = encode_client_id(&ClientInfo {
            name: "x".into(),
            redirect_uris: vec!["http://example.com/cb".into()],
        });
        assert!(validate_client(&forged, "http://example.com/cb").is_none());
    }

    #[test]
    fn pkce_s256() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("wrong", challenge));
    }

    #[test]
    fn codes_are_single_use_and_expire() {
        let codes = OAuthCodes::default();
        let pending = |ttl| PendingCode {
            client_id: "c".into(),
            redirect_uri: "r".into(),
            code_challenge: "x".into(),
            access_token: "t".into(),
            scopes: vec![],
            expires_at: Instant::now() + ttl,
        };
        codes.insert("a".into(), pending(CODE_TTL));
        assert!(codes.take("a").is_some());
        assert!(codes.take("a").is_none());
        codes.insert("b".into(), pending(Duration::ZERO));
        assert!(codes.take("b").is_none());
    }

    #[test]
    fn scope_defaults_to_read_only() {
        let read: Vec<String> = AgentScope::READ_SCOPES
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_eq!(requested_or_default_scopes(None), read);
        assert_eq!(requested_or_default_scopes(Some("bogus")), read);
        assert_eq!(
            requested_or_default_scopes(Some("accounts:read bogus")),
            vec!["accounts:read".to_string()]
        );
    }

    #[test]
    fn origin_prefers_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "internal:8088".parse().unwrap());
        assert_eq!(public_origin(&headers), "http://internal:8088");
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        headers.insert("x-forwarded-host", "wf.example.com".parse().unwrap());
        assert_eq!(public_origin(&headers), "https://wf.example.com");
    }
}
