//! Fork-only: tools left out of MCP `tools/list` to keep clients' context small.
//!
//! Each listed tool costs every MCP client its name, description and schema in
//! context. These were unused or duplicated by another tool in the server's MCP
//! audit log (2026-09-22 to 2026-10-15). They stay registered, so a client that
//! still calls one by name keeps working, and the in-app assistant is unaffected.

pub const MCP_UNLISTED_TOOLS: &[&str] = &[
    // Draft flow; MCP clients write through mutate_activities and the CSV import.
    "record_activity",
    "record_activities",
    "commit_activity_draft",
    "commit_activity_drafts",
    // Rule drafts; commit_categorization_rule saves a rule directly.
    "create_categorization_rule",
    "commit_reviewed_categorization_rule",
    // Same job as commit_category_assignments.
    "commit_transaction_categories",
    // Never called.
    "prepare_asset_classification",
    "commit_asset_classification_draft",
    // Same job as mutate_activities.
    "prepare_activity_updates",
    "commit_activity_updates",
    // Same job as upsert_manual_quotes.
    "prepare_quote_import",
    "commit_quote_import",
    // Never called; link_transfer_activities with unlink=true unpairs.
    "find_transfer_matches",
    "unlink_transfer_activities",
    // Asset classification and portfolios: barely or never used over MCP.
    "list_asset_taxonomies",
    "get_asset_taxonomy_assignments",
    "get_portfolios",
];

pub fn is_listed_over_mcp(name: &str) -> bool {
    !MCP_UNLISTED_TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentToolCatalog;

    #[test]
    fn unlisted_tools_are_registered_in_the_mcp_catalog() {
        let catalog = AgentToolCatalog::mcp_catalog();
        for name in MCP_UNLISTED_TOOLS {
            assert!(catalog.get(name).is_some(), "{name} is not an MCP tool");
        }
    }
}
