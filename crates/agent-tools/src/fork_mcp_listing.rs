//! Fork-only: the tools MCP `tools/list` shows, to keep clients' context small.
//!
//! Every listed tool costs each MCP client its name, description and schema in
//! context, so only routine portfolio checks and imports are listed (chosen from
//! the server's MCP audit log, 2026-09-22 to 2026-10-15, and the owner's
//! scripts). Every other tool stays registered: scripts can still call one by
//! name, and the in-app assistant is unaffected. Agents do occasional admin
//! work over SSH. A new upstream tool stays unlisted until it is added here.

pub const MCP_LISTED_TOOLS: &[&str] = &[
    // Portfolio checks.
    "get_accounts",
    "get_holdings",
    "get_cash_balances",
    "search_activities",
    "get_valuation_history",
    "get_performance",
    "get_net_worth",
    "get_health_status",
    "get_asset_allocation",
    "get_income",
    "get_market_quote",
    // Imports and corrections.
    "get_import_mapping",
    "prepare_activity_import",
    "commit_activity_import",
    "mutate_activities",
    "link_transfer_activities",
    "create_account",
    "update_account_settings",
];

pub fn is_listed_over_mcp(name: &str) -> bool {
    MCP_LISTED_TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentToolCatalog;

    #[test]
    fn listed_tools_are_registered_in_the_mcp_catalog() {
        let catalog = AgentToolCatalog::mcp_catalog();
        for name in MCP_LISTED_TOOLS {
            assert!(catalog.get(name).is_some(), "{name} is not an MCP tool");
        }
    }
}
