//! Fork tests: tools attached by `crate::tools::fork::mcp_tools`.
//! A child of the upstream test module so they share its `PanicEnv` fixture.

use super::*;

const FORK_COMMIT_TOOLS: [&str; 3] = [
    "commit_category_assignments",
    "commit_reviewed_categorization_rule",
    "create_account",
];

#[tokio::test]
async fn fork_commit_tools_are_mcp_only_and_scope_gated() {
    let assistant = AgentToolCatalog::assistant_catalog();
    let mcp = AgentToolCatalog::mcp_catalog();
    for name in FORK_COMMIT_TOOLS {
        assert!(
            assistant.get(name).is_none(),
            "{name} reached the assistant"
        );
        assert!(mcp.get(name).is_some(), "{name} missing from MCP");
        let err = mcp
            .execute(
                Arc::new(PanicEnv::default()),
                &AgentScopeSet::new(),
                name,
                serde_json::json!({}),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentToolError::ScopeDenied { .. }),
            "tool {name} should be scope-denied, got: {err}"
        );
    }
}

#[test]
fn fork_tools_follow_upstream_commit_tools_in_order() {
    let catalog = AgentToolCatalog::mcp_catalog();
    let names: Vec<&str> = catalog.iter().map(|tool| tool.name()).collect();
    let position = |name: &str| names.iter().position(|n| *n == name).unwrap();
    assert_eq!(
        position("commit_category_assignments"),
        position("commit_transaction_categories") + 1
    );
    assert!(
        position("commit_reviewed_categorization_rule") < position("commit_transaction_categories")
    );
    assert!(position("recalculate_portfolio") < position("get_import_mapping"));
}

#[test]
fn mcp_catalog_registers_each_tool_name_once() {
    let catalog = AgentToolCatalog::mcp_catalog();
    let mut names = std::collections::BTreeSet::new();
    for tool in catalog.iter() {
        assert!(names.insert(tool.name()), "duplicate tool: {}", tool.name());
    }
    assert_eq!(
        catalog
            .iter()
            .filter(|tool| tool.name() == "link_transfer_activities")
            .count(),
        1
    );
}

#[tokio::test]
async fn batched_transfer_link_keeps_upstream_scopes_while_legacy_keeps_its_grant() {
    let catalog = AgentToolCatalog::mcp_catalog();
    let granted = AgentScopeSet::from_strs(["activities:write"]);
    let batch_error = catalog
        .execute(
            Arc::new(PanicEnv::default()),
            &granted,
            "link_transfer_activities",
            serde_json::json!({"pairs": []}),
        )
        .await
        .unwrap_err();
    assert!(matches!(batch_error, AgentToolError::ScopeDenied { .. }));

    // Malformed legacy arguments reach validation, without consulting services.
    let legacy_error = catalog
        .execute(
            Arc::new(PanicEnv::default()),
            &granted,
            "link_transfer_activities",
            serde_json::json!({"activityAId": "" , "activityBId": "b"}),
        )
        .await
        .unwrap_err();
    assert!(matches!(legacy_error, AgentToolError::InvalidInput(_)));
}
