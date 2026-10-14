//! Compatibility adapter for the fork's original transfer-link payload.
//!
//! Upstream grew `link_transfer_activities` into a batched operation and added
//! a separate `unlink_transfer_activities` tool. The fork already shipped the
//! flat `{activityAId, activityBId, unlink}` contract, so the catalog exposes
//! one adapter that delegates batched requests to the upstream implementation
//! and translates legacy requests at the service boundary.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};
use crate::tools::transfer_links::LinkTransferActivities as BatchedLinkTransferActivities;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyLinkTransferArgs {
    activity_a_id: String,
    activity_b_id: String,
    #[serde(default)]
    unlink: bool,
}

/// One registered transfer-link tool supporting both wire contracts.
pub struct LinkTransferActivities;

fn legacy_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "activityAId": { "type": "string" },
            "activityBId": { "type": "string" },
            "unlink": {
                "type": "boolean",
                "description": "Unpair instead of pair. Default false."
            }
        },
        "required": ["activityAId", "activityBId"]
    })
}

fn audit_args(args: &serde_json::Value) -> serde_json::Value {
    if args.get("pairs").is_some() {
        let pairs = args
            .get("pairs")
            .and_then(serde_json::Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .take(100)
                    .map(|pair| {
                        json!({
                            "activityAId": pair.get("activityAId"),
                            "activityBId": pair.get("activityBId"),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let pair_count = args
            .get("pairs")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        json!({ "pairs": pairs, "pairCount": pair_count })
    } else {
        json!({
            "activityAId": args.get("activityAId"),
            "activityBId": args.get("activityBId"),
            "unlink": args.get("unlink").cloned().unwrap_or(json!(false)),
        })
    }
}

fn ensure_non_empty(id: &str, field: &str) -> Result<String, AgentToolError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(AgentToolError::InvalidInput(format!("{field} is required")));
    }
    Ok(id.to_string())
}

async fn call_legacy(
    env: Arc<dyn AgentEnvironment>,
    args: LegacyLinkTransferArgs,
) -> Result<AgentToolResult, AgentToolError> {
    let activity_a_id = ensure_non_empty(&args.activity_a_id, "activityAId")?;
    let activity_b_id = ensure_non_empty(&args.activity_b_id, "activityBId")?;
    let service = env.activity_service();
    let outcome = if args.unlink {
        service
            .unlink_transfer_activities(activity_a_id, activity_b_id)
            .await
    } else {
        // Match upstream batched-link validation for the fork's one-pair
        // payload while retaining its original response shape.
        for id in [&activity_a_id, &activity_b_id] {
            let activity = service
                .get_activity(id)
                .map_err(|error| AgentToolError::ExecutionFailed(error.to_string()))?;
            if !activity.is_posted() {
                return Err(AgentToolError::InvalidInput(format!(
                    "Activity {id} is not posted; only posted transfers can be linked"
                )));
            }
        }
        service
            .link_transfer_activities(activity_a_id, activity_b_id)
            .await
    };
    let (a, b) = outcome.map_err(|error| AgentToolError::ExecutionFailed(error.to_string()))?;
    env.health_service().clear_cache().await;
    Ok(AgentToolResult {
        content: json!({
            "activities": [
                { "id": a.id, "accountId": a.account_id, "sourceGroupId": a.source_group_id },
                { "id": b.id, "accountId": b.account_id, "sourceGroupId": b.source_group_id },
            ],
            "linked": !args.unlink,
        }),
    })
}

#[async_trait::async_trait]
impl AgentTool for LinkTransferActivities {
    fn name(&self) -> &'static str {
        "link_transfer_activities"
    }

    fn description(&self) -> &'static str {
        "Link transfer pairs in a batch, or accept the legacy flat activityAId/activityBId payload (with unlink=true to unpair). Batched requests use the app's transfer-link validation and report per-pair results; the legacy form returns the historical activities/linked response. This MUTATES data."
    }

    fn input_schema(&self) -> serde_json::Value {
        let batched = BatchedLinkTransferActivities.input_schema();
        let legacy = legacy_schema();
        let mut properties = batched["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        properties.extend(
            legacy["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        json!({
            "type": "object",
            "properties": properties,
            "oneOf": [
                { "required": ["pairs"], "not": { "anyOf": [
                    { "required": ["activityAId"] },
                    { "required": ["activityBId"] },
                    { "required": ["unlink"] }
                ] } },
                { "required": ["activityAId", "activityBId"], "not": { "required": ["pairs"] } }
            ]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        // Keep the fork's original grant. Existing tokens with the write
        // scope remain usable; the separate upstream unlink tool advertises
        // its stricter read/draft/write requirements.
        &[AgentScope::ActivitiesWrite]
    }

    fn required_call_scopes(&self, args: &serde_json::Value) -> &'static [AgentScope] {
        if args.get("pairs").is_some() {
            BatchedLinkTransferActivities.required_scopes()
        } else {
            self.required_scopes()
        }
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    fn sanitize_args_for_audit(&self, args: &serde_json::Value) -> serde_json::Value {
        audit_args(args)
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        if args.get("pairs").is_some() {
            if ["activityAId", "activityBId", "unlink"]
                .iter()
                .any(|key| args.get(key).is_some())
            {
                return Err(AgentToolError::InvalidInput(
                    "Use either pairs or the legacy activityAId/activityBId form".to_string(),
                ));
            }
            BatchedLinkTransferActivities.call(env, args).await
        } else {
            call_legacy(env, serde_json::from_value(args)?).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_advertises_batched_and_legacy_forms() {
        let schema = LinkTransferActivities.input_schema();
        assert_eq!(schema["type"], "object");
        let forms = schema["oneOf"].as_array().expect("oneOf");
        assert_eq!(forms.len(), 2);
        assert!(forms
            .iter()
            .any(|form| form["required"] == json!(["pairs"])));
        assert!(forms
            .iter()
            .any(|form| form["required"] == json!(["activityAId", "activityBId"])));
    }

    #[test]
    fn audit_keeps_legacy_ids_and_drops_extra_fields() {
        let summary = audit_args(&json!({
            "activityAId": "a",
            "activityBId": "b",
            "unlink": true,
            "secret": "redact-me"
        }));
        assert_eq!(
            summary,
            json!({
                "activityAId": "a", "activityBId": "b", "unlink": true
            })
        );
    }
}
