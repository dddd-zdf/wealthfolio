//! Fork MCP tests: legacy transfer payloads and the MCP-only write tools.
//! A child of `agent_access` so they share its server fixtures.

use super::*;

#[tokio::test]
async fn mcp_valuation_history_includes_inactive_accounts() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    let (status, token) = create_pat(
        &server,
        &cookie,
        serde_json::json!({ "name": "history reader", "scopes": ["holdings:read"] }),
    )
    .await;
    assert_eq!(status, 201);
    let pat = token["token"].as_str().unwrap();
    let session = mcp_initialize(&server, pat).await;

    // Seed stored history directly so the read test does not depend on a
    // background rebuild or market data.
    let profile = api_post(
        &server,
        &cookie,
        "profiles/get_profile_state",
        serde_json::json!({}),
    )
    .await;
    let db_path = server
        ._tmp
        .path()
        .join("profiles")
        .join(profile["session"]["profileId"].as_str().unwrap())
        .join("app.db");
    assert!(db_path.is_file());
    let db = wealthfolio_storage_sqlite::db::DbAccess::plaintext(db_path.to_str().unwrap())
        .connect_rusqlite()
        .unwrap();
    db.execute_batch(
        "INSERT INTO accounts (id, name, currency, is_active, is_archived) VALUES
            ('inactive', 'Inactive account', 'USD', false, false),
            ('archived', 'Archived account', 'USD', false, true);
         INSERT INTO daily_account_valuation (
            id, account_id, valuation_date, account_currency, base_currency,
            fx_rate_to_base, cash_balance, investment_market_value, total_value,
            cost_basis, net_contribution, cash_balance_base, total_value_base
         ) VALUES
            ('inactive-history', 'inactive', '2026-04-01', 'USD', 'USD',
             '1', '125', '0', '125', '0', '0', '125', '125'),
            ('archived-history', 'archived', '2026-04-01', 'USD', 'USD',
             '1', '50', '0', '50', '0', '0', '50', '50');",
    )
    .unwrap();

    let result = mcp_call_tool(
        &server,
        pat,
        &session,
        "get_valuation_history",
        serde_json::json!({ "startDate": "2026-04-01", "endDate": "2026-04-01" }),
    )
    .await;
    let valuations = result["valuations"].as_array().unwrap();
    assert_eq!(valuations.len(), 1, "{result}");
    assert_eq!(valuations[0]["date"], "2026-04-01");
    assert_eq!(valuations[0]["totalValue"], 125.0);
}

/// Calls an MCP tool over the stateful session; returns the parsed `result`.
async fn mcp_call(
    server: &TestServer,
    pat: &str,
    session: &str,
    id: u64,
    tool: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let response = mcp_post(
        server,
        Some(pat),
        Some(session),
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        }),
    )
    .await;
    assert_eq!(response.status(), 200);
    parse_sse_data(&response.text().await.unwrap())["result"].clone()
}

/// Fork write grants plus the upstream reviewed categorization workflow.
fn full_write_scopes() -> Vec<&'static str> {
    READ_ONLY_SCOPES
        .iter()
        .copied()
        .chain([
            "activities:draft",
            "activities:write",
            "classification:suggest",
            "classification:write",
            "categorization:write",
            "accounts:write",
            "spending:write",
        ])
        .collect()
}

/// End-to-end `tools/call` for the MCP-only write tools:
/// `create_account` persists an account through the account service, and
/// `commit_categorization_rule` persists a rule through the rules service.
/// Rejection cases (unknown category key, missing scope) return `isError`
/// results without persisting anything.
#[tokio::test]
async fn mcp_create_account_and_commit_rule_roundtrip() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    let (status, created) = create_pat(
        &server,
        &cookie,
        serde_json::json!({ "name": "writer", "scopes": full_write_scopes() }),
    )
    .await;
    assert_eq!(status, 201);
    let pat = created["token"].as_str().unwrap().to_string();
    let session = mcp_initialize(&server, &pat).await;

    // The fork's direct legacy rule contract is intentionally narrower than
    // the reviewed upstream workflow: categorization:write is sufficient.
    let (status, legacy_created) = create_pat(
        &server,
        &cookie,
        serde_json::json!({
            "name": "legacy rule writer",
            "scopes": ["categorization:write"]
        }),
    )
    .await;
    assert_eq!(status, 201);
    let legacy_pat = legacy_created["token"].as_str().unwrap().to_string();
    let legacy_session = mcp_initialize(&server, &legacy_pat).await;

    // create_account persists through the account service.
    let result = mcp_call(
        &server,
        &pat,
        &session,
        11,
        "create_account",
        serde_json::json!({ "name": "Test Cash", "accountType": "CASH", "currency": "CAD" }),
    )
    .await;
    assert_ne!(result["isError"], true);
    let account = &result["structuredContent"]["account"];
    assert_eq!(account["name"], "Test Cash");
    assert_eq!(account["accountType"], "CASH");
    assert_eq!(account["currency"], "CAD");
    assert!(account["id"].as_str().unwrap().len() > 8);

    // The new account shows up in get_accounts.
    let result = mcp_call(
        &server,
        &pat,
        &session,
        11,
        "get_accounts",
        serde_json::json!({}),
    )
    .await;
    let names: Vec<&str> = result["structuredContent"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Test Cash"), "{names:?}");

    // Unknown account type is rejected before anything is persisted.
    let result = mcp_call(
        &server,
        &pat,
        &session,
        12,
        "create_account",
        serde_json::json!({ "name": "Bogus", "accountType": "SAVINGS", "currency": "CAD" }),
    )
    .await;
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("SAVINGS"),
        "{}",
        result["content"][0]["text"]
    );

    // The deployed commit_categorization_rule contract resolves a category key
    // and saves immediately through the live taxonomy.
    let result = mcp_call(
        &server,
        &legacy_pat,
        &legacy_session,
        13,
        "commit_categorization_rule",
        serde_json::json!({
            "pattern": "T&T",
            "taxonomyId": "spending_categories",
            "categoryKey": "groceries",
        }),
    )
    .await;
    assert_ne!(result["isError"], true);
    let rule = &result["structuredContent"]["rule"];
    assert_eq!(rule["pattern"], "T&T");
    assert_eq!(rule["categoryPath"], "Groceries");
    assert!(rule["id"].as_str().unwrap().len() > 8);

    // Unknown category keys fail before persistence.
    let result = mcp_call(
        &server,
        &legacy_pat,
        &legacy_session,
        14,
        "commit_categorization_rule",
        serde_json::json!({
            "pattern": "T&T",
            "taxonomyId": "spending_categories",
            "categoryKey": "nope-not-a-key",
        }),
    )
    .await;
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown transaction category"),
        "{}",
        result["content"][0]["text"]
    );

    // The reviewed-draft path has a separate name and Spending scopes.
    let draft = mcp_call(
        &server,
        &pat,
        &session,
        15,
        "create_categorization_rule",
        serde_json::json!({
            "pattern": "Coffee",
            "taxonomyId": "spending_categories",
            "categoryKey": "groceries",
        }),
    )
    .await;
    assert_ne!(draft["isError"], true);
    let draft_rule = draft["structuredContent"]["rule"].clone();

    let result = mcp_call(
        &server,
        &pat,
        &session,
        16,
        "commit_reviewed_categorization_rule",
        serde_json::json!({ "rule": draft_rule }),
    )
    .await;
    assert_ne!(result["isError"], true);
    let reviewed_rule = &result["structuredContent"]["created"];
    assert_eq!(reviewed_rule["pattern"], "Coffee");
    assert_eq!(
        reviewed_rule["categoryId"],
        draft["structuredContent"]["rule"]["categoryId"]
    );

    // A read-only token is scope-denied on both write tools (nothing runs).
    let (status, created) = create_pat(
        &server,
        &cookie,
        serde_json::json!({ "name": "reader", "scopes": READ_ONLY_SCOPES }),
    )
    .await;
    assert_eq!(status, 201);
    let reader_pat = created["token"].as_str().unwrap().to_string();
    let reader_session = mcp_initialize(&server, &reader_pat).await;
    for tool in [
        "create_account",
        "commit_categorization_rule",
        "commit_reviewed_categorization_rule",
    ] {
        let result = mcp_call(
            &server,
            &reader_pat,
            &reader_session,
            20,
            tool,
            serde_json::json!({ "name": "X", "accountType": "CASH", "currency": "CAD",
                                "pattern": "X", "taxonomyId": "spending_categories",
                                "categoryKey": "groceries" }),
        )
        .await;
        assert_eq!(result["isError"], true, "{tool}");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("scope") || text.contains("denied"),
            "{tool}: {text}"
        );
    }
}

/// The fork's original flat payload remains usable through the single
/// registered link tool, including its historical response and unlink flag.
#[tokio::test]
async fn mcp_legacy_flat_transfer_link_payload_remains_compatible() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    disable_market_data_providers(&server, &cookie).await;
    let (pat, session) = activity_writer_session(&server, &cookie).await;
    let outgoing_account = create_eur_account(&server, &cookie, "Legacy outgoing").await;
    let incoming_account = create_eur_account(&server, &cookie, "Legacy incoming").await;
    let create_transfer = |account: &serde_json::Value, activity_type: &str| {
        api_post(
            &server,
            &cookie,
            "activities",
            serde_json::json!({
                "accountId": account["id"], "activityType": activity_type,
                "activityDate": "2026-05-01T12:00:00Z", "currency": "EUR", "amount": "42"
            }),
        )
    };
    let activity_a_id = create_transfer(&outgoing_account, "TRANSFER_OUT").await["id"].clone();
    let activity_b_id = create_transfer(&incoming_account, "TRANSFER_IN").await["id"].clone();
    let args = serde_json::json!({ "activityAId": activity_a_id, "activityBId": activity_b_id });
    let linked = mcp_call_tool(
        &server,
        &pat,
        &session,
        "link_transfer_activities",
        args.clone(),
    )
    .await;
    assert_eq!(linked["linked"], true, "{linked}");
    assert_eq!(
        linked["activities"].as_array().unwrap().len(),
        2,
        "{linked}"
    );

    let unlinked = mcp_call_tool(
        &server,
        &pat,
        &session,
        "link_transfer_activities",
        serde_json::json!({
            "activityAId": args["activityAId"],
            "activityBId": args["activityBId"],
            "unlink": true
        }),
    )
    .await;
    assert_eq!(unlinked["linked"], false, "{unlinked}");
    assert_eq!(
        unlinked["activities"].as_array().unwrap().len(),
        2,
        "{unlinked}"
    );
}

/// A rejected `/mcp` request (no or bad PAT) points OAuth-capable clients at
/// the protected-resource metadata.
#[tokio::test]
async fn mcp_unauthorized_names_resource_metadata() {
    let server = spawn_server(true, false).await;
    for bearer in [None, Some("wfp_notavalidtoken_notavalidtoken_notavalid")] {
        let response = mcp_post(&server, bearer, None, init_body()).await;
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let challenge = response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            challenge.starts_with("Bearer resource_metadata=\"http://")
                && challenge.ends_with("/.well-known/oauth-protected-resource/mcp\""),
            "{challenge}"
        );
    }
}

/// The server hands its domain event sink to the MCP tools, so a fork
/// recalculation request is queued instead of refused.
#[tokio::test]
async fn mcp_recalculate_portfolio_is_queued() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    let (status, token) = create_pat(
        &server,
        &cookie,
        serde_json::json!({ "name": "recalc", "scopes": ["activities:draft", "activities:write"] }),
    )
    .await;
    assert_eq!(status, 201);
    let pat = token["token"].as_str().unwrap().to_string();
    let session = mcp_initialize(&server, &pat).await;

    let result = mcp_call_tool(
        &server,
        &pat,
        &session,
        "recalculate_portfolio",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(result["queued"], true, "{result}");
}

/// The fork's routes for valuation history (cached) and the 1D/1W intraday
/// curve answer with the expected payloads, and the curve rejects other ranges.
#[tokio::test]
async fn fork_valuation_routes_answer_and_validate_the_range() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    let (account, _asset_id, _pat, _session) = fund_holding_session(&server, &cookie).await;
    let account_id = account["id"].as_str().unwrap();
    let filter = serde_json::json!({ "type": "account", "accountId": account_id });

    let history =
        serde_json::json!({ "filter": filter, "startDate": "2026-08-01", "endDate": "2026-08-31" });
    let first = api_post(
        &server,
        &cookie,
        "valuations/history/query",
        history.clone(),
    )
    .await;
    assert!(
        first.as_array().is_some_and(|days| !days.is_empty()),
        "{first}"
    );
    // A repeat is served from the fork's cache with the same payload.
    let second = api_post(&server, &cookie, "valuations/history/query", history).await;
    assert_eq!(first, second);

    let intraday = api_post(
        &server,
        &cookie,
        "valuations/intraday/query",
        serde_json::json!({ "filter": filter, "range": "1D" }),
    )
    .await;
    assert!(intraday.is_array(), "{intraday}");

    let rejected = server
        .client
        .post(format!("{}/api/v1/valuations/intraday/query", server.base))
        .header(header::COOKIE, format!("wf_session={cookie}"))
        .json(&serde_json::json!({ "filter": filter, "range": "1M" }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
}

/// Both `initialize` and subsequent session `POST /mcp` requests (`tools/list`,
/// `tools/call`) omit rmcp's empty-data SSE priming event so `2025-11-25`
/// clients such as Gemini can parse every `data:` frame as JSON-RPC.
#[tokio::test]
async fn mcp_sse_responses_omit_empty_priming_frames() {
    let server = spawn_server(true, false).await;
    let cookie = login(&server).await;
    let (status, token) = create_pat(
        &server,
        &cookie,
        serde_json::json!({ "name": "gemini", "scopes": ["accounts:read", "holdings:read"] }),
    )
    .await;
    assert_eq!(status, 201);
    let pat = token["token"].as_str().unwrap();

    let assert_clean_sse = |sse: &str| -> serde_json::Value {
        assert!(!sse.contains("retry:"), "unexpected SSE retry line: {sse}");
        let data_lines: Vec<&str> = sse
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .collect();
        assert_eq!(data_lines.len(), 1, "expected single JSON-RPC event: {sse}");
        assert!(
            !data_lines[0].is_empty(),
            "SSE stream must not start with empty priming frame: {sse}"
        );
        serde_json::from_str(data_lines[0]).unwrap()
    };

    let init_resp = mcp_post(
        &server,
        Some(pat),
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
    )
    .await;
    assert_eq!(init_resp.status(), 200);
    let session = init_resp
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let init_json = assert_clean_sse(&init_resp.text().await.unwrap());
    assert_eq!(init_json["result"]["protocolVersion"], "2025-11-25");

    let notify = mcp_post(
        &server,
        Some(pat),
        Some(&session),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!(notify.status(), 202);

    let list_resp = mcp_post(
        &server,
        Some(pat),
        Some(&session),
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    assert_eq!(list_resp.status(), 200);
    let list_json = assert_clean_sse(&list_resp.text().await.unwrap());
    assert!(list_json["result"]["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));

    let call_resp = mcp_post(
        &server,
        Some(pat),
        Some(&session),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "get_accounts", "arguments": {} }
        }),
    )
    .await;
    assert_eq!(call_resp.status(), 200);
    let call_json = assert_clean_sse(&call_resp.text().await.unwrap());
    assert_eq!(call_json["result"]["isError"], false);
}
