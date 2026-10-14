//! Fork-only MCP commits, kept separate from upstream catalog additions.
use super::{data_admin, CommitCategorizationRule, CommitCategoryAssignments, CreateAccount};
use crate::tool::AgentTool;
use std::sync::Arc;

pub(super) fn tools() -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(CommitCategoryAssignments),
        Arc::new(CommitCategorizationRule),
        Arc::new(CreateAccount),
        Arc::new(data_admin::MutateActivities),
        Arc::new(data_admin::UpsertManualQuotes),
        Arc::new(data_admin::UpdateAccountSettings),
        Arc::new(data_admin::SetAssetQuoteMode),
        Arc::new(data_admin::SetTermDeposit),
        Arc::new(data_admin::RecalculatePortfolio),
    ]
}
