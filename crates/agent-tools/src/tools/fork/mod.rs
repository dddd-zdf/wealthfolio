//! Fork-only MCP tools and the one hook that attaches them to upstream's
//! MCP catalog (`AgentToolCatalog::mcp_catalog`).
//!
//! Upstream tool files stay unchanged. Where the fork's public contract
//! differs from upstream's, an adapter here takes the upstream tool's place
//! by name:
//! - `commit_categorization_rule` is the fork's direct-write tool; upstream's
//!   reviewed-draft tool is exposed as `commit_reviewed_categorization_rule`.
//! - `link_transfer_activities` accepts the fork's flat payload as well as
//!   upstream's batch (see `transfer_compat`).

pub mod commit_categories;
pub mod create_account;
pub mod data_admin;
pub mod direct_categorization_rule;
pub mod transfer_compat;

use std::sync::Arc;

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};
use crate::tools::CommitCategorizationRule as UpstreamReviewedCategorizationRule;

pub use commit_categories::CommitCategoryAssignments;
pub use create_account::CreateAccount;
pub use direct_categorization_rule::CommitDirectCategorizationRule;

/// Upstream commit tool after which the fork's commit tools are listed.
const LAST_UPSTREAM_COMMIT_TOOL: &str = "commit_transaction_categories";

/// Fork-only commit tools, in catalog order.
fn commit_tools() -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(CommitCategoryAssignments),
        Arc::new(CommitDirectCategorizationRule),
        Arc::new(CreateAccount),
        Arc::new(data_admin::MutateActivities),
        Arc::new(data_admin::UpsertManualQuotes),
        Arc::new(data_admin::UpdateAccountSettings),
        Arc::new(data_admin::SetAssetQuoteMode),
        Arc::new(data_admin::SetTermDeposit),
        Arc::new(data_admin::RecalculatePortfolio),
    ]
}

/// Upstream's MCP tools with the fork's adapters in place and the fork's
/// commit tools after upstream's.
pub(crate) fn mcp_tools(upstream: Vec<Arc<dyn AgentTool>>) -> Vec<Arc<dyn AgentTool>> {
    let mut tools: Vec<Arc<dyn AgentTool>> = Vec::with_capacity(upstream.len() + 9);
    let mut fork_commits = Some(commit_tools());
    for tool in upstream {
        let is_last_commit = tool.name() == LAST_UPSTREAM_COMMIT_TOOL;
        tools.push(match tool.name() {
            "commit_categorization_rule" => Arc::new(ReviewedCategorizationRule),
            "link_transfer_activities" => Arc::new(transfer_compat::LinkTransferActivities),
            _ => tool,
        });
        if is_last_commit {
            tools.extend(fork_commits.take().into_iter().flatten());
        }
    }
    tools.extend(fork_commits.into_iter().flatten());
    tools
}

/// Upstream's reviewed-draft rule commit under the fork's name for it.
pub struct ReviewedCategorizationRule;

#[async_trait::async_trait]
impl AgentTool for ReviewedCategorizationRule {
    fn name(&self) -> &'static str {
        "commit_reviewed_categorization_rule"
    }

    fn description(&self) -> &'static str {
        UpstreamReviewedCategorizationRule.description()
    }

    fn input_schema(&self) -> serde_json::Value {
        UpstreamReviewedCategorizationRule.input_schema()
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        UpstreamReviewedCategorizationRule.required_scopes()
    }

    fn required_call_scopes(&self, args: &serde_json::Value) -> &'static [AgentScope] {
        UpstreamReviewedCategorizationRule.required_call_scopes(args)
    }

    fn access_level(&self) -> AgentToolAccess {
        UpstreamReviewedCategorizationRule.access_level()
    }

    fn is_additive(&self) -> bool {
        UpstreamReviewedCategorizationRule.is_additive()
    }

    fn sanitize_args_for_audit(&self, args: &serde_json::Value) -> serde_json::Value {
        UpstreamReviewedCategorizationRule.sanitize_args_for_audit(args)
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        UpstreamReviewedCategorizationRule.call(env, args).await
    }
}
