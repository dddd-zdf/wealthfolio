//! Commit Category Assignments tool (MCP-only).
//!
//! `commit_category_assignments` persists reviewed category proposals (the
//! shape `propose_transaction_categories` returns under `proposals`) as real
//! activity-taxonomy assignments. This is the write step the in-app assistant
//! performs through its confirmation widget (the frontend calls the bulk-assign
//! mutation). MCP callers have no widget, so this tool lets a scoped token
//! commit previously-reviewed proposals directly.
//!
//! Category keys are taxonomy-scoped (`groceries`, not `cat_groceries`) and are
//! resolved to category IDs against the live taxonomy at call time — the same
//! key lookup `propose_transaction_categories` builds. The commit goes through
//! the same service method as the widget's Apply (`bulk_assign_categories`),
//! including its per-item validation and atomic write.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wealthfolio_spending::activity_assignments::BulkCategoryAssignment;

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};

/// Max assignments per `commit_category_assignments` call. Mirrors the
/// widget's bulk-apply cap (`MAX_BULK_CATEGORY_ASSIGNMENTS` in the server),
/// so a buggy or hostile client with write scope can't trigger an unbounded
/// bulk write.
const MAX_COMMIT_ASSIGNMENTS: usize = 1_000;

/// One reviewed category assignment, supplied by the agent after the proposals
/// from `propose_transaction_categories` (or `list_categorization_context`
/// reasoning) have been reviewed and confirmed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryAssignmentInput {
    /// Activity ID from the proposals.
    pub activity_id: String,
    /// Taxonomy ID containing `category_key`. Category keys are taxonomy-scoped.
    pub taxonomy_id: String,
    /// Category key from the taxonomy (e.g. "groceries").
    pub category_key: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitCategoryAssignmentsArgs {
    pub assignments: Vec<CategoryAssignmentInput>,
}

/// Summary of a committed category assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedCategoryAssignment {
    pub activity_id: String,
    pub taxonomy_id: String,
    pub category_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitCategoryAssignmentsOutput {
    pub applied: Vec<CommittedCategoryAssignment>,
}

/// Drop the assignment payload from audit args — never persist activity ids or
/// category choices in `mcp_audit_log`. Reports only the batch size.
fn redact_assignments(args: &serde_json::Value) -> serde_json::Value {
    let mut redacted = serde_json::Map::new();
    if let Some(assignments) = args.get("assignments").and_then(|a| a.as_array()) {
        redacted.insert(
            "assignments".to_string(),
            serde_json::json!(format!("[{} assignments]", assignments.len())),
        );
    }
    serde_json::Value::Object(redacted)
}

/// Build the `(taxonomy_id, category_key) -> category_id` lookup from the
/// live activity-scope taxonomies. Same construction as the propose tool's
/// `key_lookup`.
fn build_key_lookup(
    env: &dyn AgentEnvironment,
) -> Result<HashMap<(String, String), String>, AgentToolError> {
    let taxonomies = env
        .taxonomy_service()
        .get_taxonomies_with_categories()
        .map_err(|e| AgentToolError::ExecutionFailed(e.to_string()))?;
    let mut lookup = HashMap::new();
    for entry in taxonomies.iter().filter(|t| t.taxonomy.scope == "activity") {
        for cat in &entry.categories {
            lookup.insert(
                (entry.taxonomy.id.clone(), cat.key.clone()),
                cat.id.clone(),
            );
        }
    }
    Ok(lookup)
}

/// Resolve agent-supplied `(activity_id, taxonomy_id, category_key)` inputs to
/// the service's [`BulkCategoryAssignment`] shape. Unknown category keys fail
/// the whole call — same all-or-nothing behavior as the service's own
/// validation, so a typo can't silently commit a partial batch.
fn resolve_assignments(
    inputs: &[CategoryAssignmentInput],
    key_lookup: &HashMap<(String, String), String>,
) -> Result<Vec<BulkCategoryAssignment>, AgentToolError> {
    let mut items = Vec::with_capacity(inputs.len());
    let mut unknown = Vec::new();
    for input in inputs {
        let activity_id = input.activity_id.trim();
        if activity_id.is_empty() {
            return Err(AgentToolError::InvalidInput(
                "Assignment is missing an activity_id".to_string(),
            ));
        }
        match key_lookup.get(&(input.taxonomy_id.clone(), input.category_key.clone())) {
            Some(category_id) => items.push(BulkCategoryAssignment {
                activity_id: activity_id.to_string(),
                taxonomy_id: input.taxonomy_id.clone(),
                category_id: category_id.clone(),
            }),
            None => unknown.push(format!(
                "{} / {}",
                input.taxonomy_id, input.category_key
            )),
        }
    }
    if !unknown.is_empty() {
        return Err(AgentToolError::InvalidInput(format!(
            "Unknown taxonomy/category keys: {}",
            unknown.join(", ")
        )));
    }
    Ok(items)
}

/// Commit reviewed category assignments.
pub struct CommitCategoryAssignments;

#[async_trait::async_trait]
impl AgentTool for CommitCategoryAssignments {
    fn name(&self) -> &'static str {
        "commit_category_assignments"
    }

    fn description(&self) -> &'static str {
        "Persist reviewed category assignments (the shape returned by \
         propose_transaction_categories under `proposals`) as real activity \
         categories. Pass each proposal's activityId, taxonomyId, and \
         categoryKey. This MUTATES data — only call it after the proposals \
         have been reviewed and confirmed. At most 1000 assignments per call."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "assignments": {
                    "type": "array",
                    "description": "Reviewed category assignments to persist.",
                    "maxItems": MAX_COMMIT_ASSIGNMENTS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "activityId": { "type": "string" },
                            "taxonomyId": { "type": "string" },
                            "categoryKey": { "type": "string", "description": "Category key from the taxonomy (e.g. \"groceries\")." }
                        },
                        "required": ["activityId", "taxonomyId", "categoryKey"]
                    }
                }
            },
            "required": ["assignments"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::CategorizationWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    fn sanitize_args_for_audit(&self, args: &serde_json::Value) -> serde_json::Value {
        redact_assignments(args)
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        // Reject oversized batches from the raw array length before
        // deserializing the whole payload — a buggy or hostile client with
        // write scope must not be able to force unbounded work.
        if let Some(len) = args
            .get("assignments")
            .and_then(|a| a.as_array())
            .map(Vec::len)
        {
            if len > MAX_COMMIT_ASSIGNMENTS {
                return Err(AgentToolError::InvalidInput(format!(
                    "Batch limited to {MAX_COMMIT_ASSIGNMENTS} assignments, got {len}"
                )));
            }
        }

        let args: CommitCategoryAssignmentsArgs = serde_json::from_value(args)?;

        let key_lookup = build_key_lookup(env.as_ref())?;
        let items = resolve_assignments(&args.assignments, &key_lookup)?;

        // Same service method the widget's Apply calls: validates every item
        // first (spending scope, cash-flow bucket, single-select taxonomy),
        // then writes atomically.
        let committed = env
            .cash_activity_service()
            .bulk_assign_categories(&items)
            .await
            .map_err(|e| AgentToolError::ExecutionFailed(e.to_string()))?;
        env.health_service().clear_cache().await;

        let output = CommitCategoryAssignmentsOutput {
            applied: committed
                .into_iter()
                .map(|a| CommittedCategoryAssignment {
                    activity_id: a.activity_id,
                    taxonomy_id: a.taxonomy_id,
                    category_id: a.category_id,
                })
                .collect(),
        };
        Ok(AgentToolResult {
            content: serde_json::to_value(output)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lookup() -> HashMap<(String, String), String> {
        let mut m = HashMap::new();
        m.insert(
            ("tax1".to_string(), "groceries".to_string()),
            "cat-g".to_string(),
        );
        m.insert(("tax1".to_string(), "coffee".to_string()), "cat-c".to_string());
        m
    }

    fn make_input(activity_id: &str, key: &str) -> CategoryAssignmentInput {
        CategoryAssignmentInput {
            activity_id: activity_id.to_string(),
            taxonomy_id: "tax1".to_string(),
            category_key: key.to_string(),
        }
    }

    #[test]
    fn resolves_keys_to_category_ids() {
        let items = resolve_assignments(
            &[make_input("a1", "groceries"), make_input("a2", "coffee")],
            &make_lookup(),
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].activity_id, "a1");
        assert_eq!(items[0].taxonomy_id, "tax1");
        assert_eq!(items[0].category_id, "cat-g");
        assert_eq!(items[1].category_id, "cat-c");
    }

    #[test]
    fn unknown_key_fails_the_whole_batch() {
        let err = resolve_assignments(&[make_input("a1", "nope")], &make_lookup()).unwrap_err();
        assert!(
            matches!(err, AgentToolError::InvalidInput(_)),
            "unexpected: {err:?}"
        );
        assert!(err.to_string().contains("tax1 / nope"));
    }

    #[test]
    fn missing_activity_id_is_rejected() {
        let err =
            resolve_assignments(&[make_input("   ", "groceries")], &make_lookup()).unwrap_err();
        assert!(matches!(err, AgentToolError::InvalidInput(_)));
    }

    #[test]
    fn audit_redaction_drops_assignment_payload() {
        let args = serde_json::json!({
            "assignments": [ { "activityId": "a1", "categoryKey": "groceries" } ],
        });
        assert_eq!(
            redact_assignments(&args)["assignments"],
            serde_json::json!("[1 assignments]")
        );
    }

    #[test]
    fn audit_redaction_drops_unknown_keys() {
        let args = serde_json::json!({
            "assignments": [],
            "note": "must not survive",
        });
        let redacted = redact_assignments(&args);
        assert!(redacted.get("note").is_none());
        assert_eq!(redacted.as_object().unwrap().len(), 1);
    }

    #[test]
    fn commit_schema_caps_batch_size() {
        let schema = CommitCategoryAssignments.input_schema();
        assert_eq!(
            schema["properties"]["assignments"]["maxItems"],
            serde_json::json!(MAX_COMMIT_ASSIGNMENTS)
        );
    }
}
