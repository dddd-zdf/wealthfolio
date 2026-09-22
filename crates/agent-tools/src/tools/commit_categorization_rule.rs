//! Commit Categorization Rule tool (MCP-only).
//!
//! `commit_categorization_rule` persists a categorization rule directly,
//! without the in-app assistant's confirmation widget. It reuses the draft
//! tool's validation wholesale — `CreateCategorizationRule::build_output`
//! resolves `(taxonomy_id, category_key)` against the live taxonomy,
//! validates the pattern/match type, and (for account-scoped rules) the
//! account — then the validated `NewCategorizationRule` is saved through
//! `CategorizationRulesService::create`, the same service method the widget
//! confirmation path uses.
//!
//! Like the other commit tools, this MUTATES data: there is no second
//! confirmation step. Audit args redact the pattern/name, mirroring the
//! draft tool.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};
use crate::tools::create_categorization_rule::{
    CreateCategorizationRule, CreateCategorizationRuleArgs,
};

/// DTO for the persisted rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedRuleDto {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub match_type: String,
    pub taxonomy_id: Option<String>,
    pub category_id: Option<String>,
    pub category_path: String,
    pub is_global: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitCategorizationRuleOutput {
    pub rule: CommittedRuleDto,
    pub message: String,
}

/// Persist a categorization rule without widget confirmation.
pub struct CommitCategorizationRule;

#[async_trait::async_trait]
impl AgentTool for CommitCategorizationRule {
    fn name(&self) -> &'static str {
        "commit_categorization_rule"
    }

    fn description(&self) -> &'static str {
        "Persist a categorization rule directly (no confirmation widget). \
         Pass pattern, taxonomyId, and categoryKey — e.g. pattern \"T&T\" \
         with categoryKey \"groceries\" categorizes future matching \
         transactions as Groceries. matchType defaults to \"contains\"; \
         accountId optionally scopes the rule to one account. This MUTATES \
         data — the rule is saved immediately."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Short rule name shown in settings. Default: derive from pattern, e.g. \"T&T → Groceries\"."
                },
                "pattern": {
                    "type": "string",
                    "description": "Substring/pattern matched against transaction notes. contains/starts_with/exact are case-insensitive; regex is a Rust regex and is case-sensitive unless it uses an inline flag like (?i)."
                },
                "matchType": {
                    "type": "string",
                    "enum": ["contains", "starts_with", "exact", "regex"],
                    "description": "Default \"contains\". Use stricter modes only if needed."
                },
                "categoryKey": {
                    "type": "string",
                    "description": "Category key from the activity-scope taxonomies (e.g. \"groceries\")."
                },
                "taxonomyId": {
                    "type": "string",
                    "description": "Taxonomy ID containing categoryKey. Required because category keys are taxonomy-scoped."
                },
                "activityType": {
                    "type": "string",
                    "description": "Optional activity-type narrowing (e.g. WITHDRAWAL). Usually omit."
                },
                "accountId": {
                    "type": "string",
                    "description": "Optional account ID to scope the rule to one account. Omit for a global rule."
                }
            },
            "required": ["pattern", "taxonomyId", "categoryKey"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::CategorizationWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    fn sanitize_args_for_audit(&self, args: &serde_json::Value) -> serde_json::Value {
        // Same redaction as the draft tool: `pattern` and `name` echo real
        // merchant/transaction text, so they never land in the audit log.
        let mut redacted = serde_json::Map::new();
        if let Some(obj) = args.as_object() {
            for key in ["matchType", "categoryKey", "taxonomyId", "activityType"] {
                if let Some(value) = obj.get(key) {
                    redacted.insert(key.to_string(), value.clone());
                }
            }
            if obj.contains_key("pattern") {
                redacted.insert("pattern".to_string(), serde_json::json!("[redacted]"));
            }
            if obj.contains_key("name") {
                redacted.insert("name".to_string(), serde_json::json!("[redacted]"));
            }
        }
        serde_json::Value::Object(redacted)
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let args: CreateCategorizationRuleArgs = serde_json::from_value(args)?;

        // Reuse the draft tool's full validation: live-taxonomy key
        // resolution, pattern/match-type checks, and account existence for
        // account-scoped rules. Unknown category keys fail here, before
        // anything is persisted.
        let draft = CreateCategorizationRule::build_output(env.as_ref(), args).await?;

        // The widget confirmation path saves through this same service
        // method (which re-validates scope, pattern, and amount conditions).
        let saved = env
            .categorization_rules_service()
            .create(draft.rule)
            .await
            .map_err(|e| AgentToolError::ExecutionFailed(e.to_string()))?;

        let message = format!(
            "Saved rule: anything matching \"{}\" will be {}.",
            saved.pattern, draft.category_path
        );
        let output = CommitCategorizationRuleOutput {
            rule: CommittedRuleDto {
                id: saved.id,
                name: saved.name,
                pattern: saved.pattern,
                match_type: saved.match_type.as_str().to_string(),
                taxonomy_id: saved.taxonomy_id,
                category_id: saved.category_id,
                category_path: draft.category_path,
                is_global: saved.is_global,
                account_id: saved.account_id,
            },
            message,
        };
        Ok(AgentToolResult {
            content: serde_json::to_value(output)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_rule_schema_requires_pattern_taxonomy_key() {
        let schema = CommitCategorizationRule.input_schema();
        assert_eq!(
            schema["required"],
            serde_json::json!(["pattern", "taxonomyId", "categoryKey"])
        );
        assert_eq!(
            schema["properties"]["matchType"]["enum"],
            serde_json::json!(["contains", "starts_with", "exact", "regex"])
        );
    }

    #[test]
    fn commit_rule_requires_categorization_write_scope() {
        assert_eq!(
            CommitCategorizationRule.required_scopes(),
            &[AgentScope::CategorizationWrite]
        );
        assert_eq!(
            CommitCategorizationRule.access_level(),
            AgentToolAccess::Write
        );
    }

    #[test]
    fn audit_redaction_drops_pattern_and_name() {
        let args = serde_json::json!({
            "pattern": "T&T",
            "name": "T&T -> Groceries",
            "matchType": "contains",
            "categoryKey": "groceries",
            "taxonomyId": "spending_categories",
        });
        let redacted = CommitCategorizationRule.sanitize_args_for_audit(&args);
        assert_eq!(redacted["pattern"], serde_json::json!("[redacted]"));
        assert_eq!(redacted["name"], serde_json::json!("[redacted]"));
        assert_eq!(redacted["categoryKey"], serde_json::json!("groceries"));
        assert!(redacted.get("accountId").is_none());
    }

    #[test]
    fn args_deserialize_from_camel_case() {
        let args: CreateCategorizationRuleArgs = serde_json::from_value(serde_json::json!({
            "pattern": "T&T",
            "taxonomyId": "spending_categories",
            "categoryKey": "groceries",
            "matchType": "exact",
            "accountId": "acc-1",
        }))
        .unwrap();
        assert_eq!(args.pattern, "T&T");
        assert_eq!(args.match_type.as_deref(), Some("exact"));
        assert_eq!(args.account_id.as_deref(), Some("acc-1"));
    }
}
