//! Data-administration tools (MCP-only).
//!
//! These tools let a scoped MCP token maintain history through the same
//! service paths the UI uses, instead of editing the database directly:
//!
//! - `mutate_activities` — create / update / delete activities in one batch
//!   (`ActivityServiceTrait::bulk_mutate_activities`, the UI's bulk editor
//!   path). Supports `subtype` (e.g. `DIVIDEND_IN_KIND`) and `metadata`
//!   (e.g. `{"flow":{"is_external":true}}`).
//! - `link_transfer_activities` — pair (or unpair) a TRANSFER_IN with a
//!   TRANSFER_OUT as an internal transfer.
//! - `upsert_manual_quotes` — save MANUAL price points (e.g. private-fund NAVs).
//! - `update_account_settings` — rename, regroup, hide/show, archive or change
//!   the tracking mode of an account.
//! - `set_asset_quote_mode` — switch an asset between MARKET and MANUAL pricing.
//! - `recalculate_portfolio` — rebuild valuations from saved prices.
//!
//! Every call goes through the MCP audit log like any other tool.

use std::sync::Arc;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use wealthfolio_core::accounts::{AccountUpdate, TrackingMode};
use wealthfolio_core::activities::ActivityBulkMutationRequest;
use wealthfolio_core::events::DomainEvent;
use wealthfolio_core::quotes::{Quote, DATA_SOURCE_MANUAL};

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};

fn exec_err(e: impl std::fmt::Display) -> AgentToolError {
    AgentToolError::ExecutionFailed(e.to_string())
}

/// Ask the host to rebuild valuations from saved prices. Returns whether the
/// request could be delivered (hosts without an event sink return false).
fn request_full_recalculation(env: &Arc<dyn AgentEnvironment>) -> bool {
    match env.domain_event_sink() {
        Some(sink) => {
            sink.emit(DomainEvent::PriceHistoryChanged);
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// mutate_activities
// ---------------------------------------------------------------------------

/// Create, update and delete activities in one batch.
pub struct MutateActivities;

#[async_trait::async_trait]
impl AgentTool for MutateActivities {
    fn name(&self) -> &'static str {
        "mutate_activities"
    }

    fn description(&self) -> &'static str {
        "Create, update and/or delete activities in one batch through the same \
         service path as the UI's bulk editor. `creates` take NewActivity objects \
         (accountId, activityType, activityDate, currency, optional asset {id | symbol, \
         exchangeMic}, quantity, unitPrice, amount, fee, subtype e.g. DIVIDEND_IN_KIND, \
         notes, metadata JSON string e.g. '{\"flow\":{\"is_external\":true}}'). \
         `updates` take full ActivityUpdate objects (id, accountId, activityType, \
         activityDate, currency plus the fields to set; omit asset to keep it). \
         `deleteIds` lists activity ids to delete. This MUTATES data immediately; \
         portfolio recalculation is triggered automatically. Returns created/updated/ \
         deleted ids and per-row errors."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "creates": {
                    "type": "array",
                    "description": "NewActivity objects to create.",
                    "items": { "type": "object" }
                },
                "updates": {
                    "type": "array",
                    "description": "ActivityUpdate objects (must include id, accountId, activityType, activityDate, currency).",
                    "items": { "type": "object" }
                },
                "deleteIds": {
                    "type": "array",
                    "description": "Activity ids to delete.",
                    "items": { "type": "string" }
                }
            }
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::ActivitiesWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    fn sanitize_args_for_audit(&self, args: &serde_json::Value) -> serde_json::Value {
        // Keep the audit row compact: record counts and the touched ids.
        let count = |key: &str| {
            args.get(key)
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        };
        let update_ids: Vec<serde_json::Value> = args
            .get("updates")
            .and_then(serde_json::Value::as_array)
            .map(|rows| rows.iter().filter_map(|r| r.get("id").cloned()).collect())
            .unwrap_or_default();
        serde_json::json!({
            "creates": count("creates"),
            "updates": count("updates"),
            "updateIds": update_ids,
            "deleteIds": args.get("deleteIds").cloned().unwrap_or(serde_json::Value::Null),
        })
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let request: ActivityBulkMutationRequest = serde_json::from_value(args)?;
        if request.creates.is_empty() && request.updates.is_empty() && request.delete_ids.is_empty()
        {
            return Err(AgentToolError::InvalidInput(
                "Nothing to do: provide creates, updates and/or deleteIds.".to_string(),
            ));
        }
        let result = env
            .activity_service()
            .bulk_mutate_activities(request)
            .await
            .map_err(exec_err)?;

        let ids = |rows: &[wealthfolio_core::activities::Activity]| -> Vec<String> {
            rows.iter().map(|a| a.id.clone()).collect()
        };
        Ok(AgentToolResult {
            content: serde_json::json!({
                "created": ids(&result.created),
                "updated": ids(&result.updated),
                "deleted": ids(&result.deleted),
                "createdMappings": result.created_mappings,
                "errors": result.errors,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// link_transfer_activities
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkTransferArgs {
    activity_a_id: String,
    activity_b_id: String,
    #[serde(default)]
    unlink: bool,
}

/// Pair or unpair two transfer activities as an internal transfer.
pub struct LinkTransferActivities;

#[async_trait::async_trait]
impl AgentTool for LinkTransferActivities {
    fn name(&self) -> &'static str {
        "link_transfer_activities"
    }

    fn description(&self) -> &'static str {
        "Pair a TRANSFER_IN and a TRANSFER_OUT (in different accounts) as one internal \
         transfer, or unpair them with unlink=true. Same service path as the UI's \
         'match transfer' action. This MUTATES data."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "activityAId": { "type": "string" },
                "activityBId": { "type": "string" },
                "unlink": { "type": "boolean", "description": "Unpair instead of pair. Default false." }
            },
            "required": ["activityAId", "activityBId"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::ActivitiesWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let args: LinkTransferArgs = serde_json::from_value(args)?;
        let service = env.activity_service();
        let outcome = if args.unlink {
            service
                .unlink_transfer_activities(args.activity_a_id, args.activity_b_id)
                .await
        } else {
            service
                .link_transfer_activities(args.activity_a_id, args.activity_b_id)
                .await
        };
        let (a, b) = outcome.map_err(exec_err)?;
        Ok(AgentToolResult {
            content: serde_json::json!({
                "activities": [
                    { "id": a.id, "accountId": a.account_id, "sourceGroupId": a.source_group_id },
                    { "id": b.id, "accountId": b.account_id, "sourceGroupId": b.source_group_id },
                ],
                "linked": !args.unlink,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// upsert_manual_quotes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManualQuoteInput {
    asset_id: String,
    date: String,
    close: Decimal,
    currency: String,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertManualQuotesArgs {
    quotes: Vec<ManualQuoteInput>,
    /// Rebuild valuations afterwards. Defaults to true.
    #[serde(default)]
    recalculate: Option<bool>,
}

/// Save MANUAL price points.
pub struct UpsertManualQuotes;

#[async_trait::async_trait]
impl AgentTool for UpsertManualQuotes {
    fn name(&self) -> &'static str {
        "upsert_manual_quotes"
    }

    fn description(&self) -> &'static str {
        "Save MANUAL daily prices (one per asset per date; an existing manual price for \
         the same day is replaced). Use for assets without a market data feed, e.g. \
         private-fund NAVs. Valuations are rebuilt afterwards unless recalculate=false. \
         This MUTATES data."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "quotes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "assetId": { "type": "string" },
                            "date": { "type": "string", "description": "YYYY-MM-DD" },
                            "close": { "type": "number" },
                            "currency": { "type": "string" },
                            "notes": { "type": "string" }
                        },
                        "required": ["assetId", "date", "close", "currency"]
                    }
                },
                "recalculate": { "type": "boolean", "description": "Default true." }
            },
            "required": ["quotes"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::ActivitiesWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let args: UpsertManualQuotesArgs = serde_json::from_value(args)?;
        if args.quotes.is_empty() {
            return Err(AgentToolError::InvalidInput("quotes is empty".to_string()));
        }
        // Validate everything before writing anything.
        let mut prepared = Vec::with_capacity(args.quotes.len());
        for q in &args.quotes {
            let day = NaiveDate::parse_from_str(q.date.trim(), "%Y-%m-%d").map_err(|e| {
                AgentToolError::InvalidInput(format!("Invalid date '{}': {e}", q.date))
            })?;
            if q.close <= Decimal::ZERO {
                return Err(AgentToolError::InvalidInput(format!(
                    "close must be positive for {} on {}",
                    q.asset_id, q.date
                )));
            }
            let asset_id = q.asset_id.trim();
            if asset_id.is_empty() {
                return Err(AgentToolError::InvalidInput(
                    "assetId is required".to_string(),
                ));
            }
            let currency = q.currency.trim().to_ascii_uppercase();
            if currency.is_empty() {
                return Err(AgentToolError::InvalidInput(
                    "currency is required".to_string(),
                ));
            }
            let timestamp = day
                .and_hms_opt(12, 0, 0)
                .expect("noon is a valid time")
                .and_utc();
            prepared.push(Quote {
                // MANUAL quotes get a deterministic id from the service.
                id: String::new(),
                asset_id: asset_id.to_string(),
                timestamp,
                open: q.close,
                high: q.close,
                low: q.close,
                close: q.close,
                adjclose: q.close,
                volume: Decimal::ZERO,
                currency,
                data_source: DATA_SOURCE_MANUAL.to_string(),
                created_at: chrono::Utc::now(),
                notes: q.notes.clone(),
            });
        }

        let service = env.quote_service();
        let mut saved = Vec::with_capacity(prepared.len());
        for quote in prepared {
            let stored = service.update_quote(quote).await.map_err(exec_err)?;
            saved.push(serde_json::json!({
                "id": stored.id,
                "assetId": stored.asset_id,
                "date": stored.timestamp.date_naive().to_string(),
                "close": stored.close,
            }));
        }

        let recalculation_requested = if args.recalculate.unwrap_or(true) {
            request_full_recalculation(&env)
        } else {
            false
        };
        Ok(AgentToolResult {
            content: serde_json::json!({
                "saved": saved,
                "recalculationRequested": recalculation_requested,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// update_account_settings
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAccountSettingsArgs {
    account_id: String,
    #[serde(default)]
    name: Option<String>,
    /// Empty string clears the group.
    #[serde(default)]
    group: Option<String>,
    /// false hides the account from lists but keeps it in calculations.
    #[serde(default)]
    is_active: Option<bool>,
    /// true removes the account (and its activities) from all calculations.
    #[serde(default)]
    is_archived: Option<bool>,
    #[serde(default)]
    tracking_mode: Option<TrackingMode>,
}

/// Update account settings without touching its activities.
pub struct UpdateAccountSettings;

#[async_trait::async_trait]
impl AgentTool for UpdateAccountSettings {
    fn name(&self) -> &'static str {
        "update_account_settings"
    }

    fn description(&self) -> &'static str {
        "Update an account's name, group, visibility (isActive=false hides it but keeps \
         it in calculations), archived flag (isArchived=true EXCLUDES it and its \
         activities from all calculations) or trackingMode (TRANSACTIONS | HOLDINGS). \
         Omitted fields are unchanged. This MUTATES data."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "accountId": { "type": "string" },
                "name": { "type": "string" },
                "group": { "type": "string", "description": "Empty string clears the group." },
                "isActive": { "type": "boolean" },
                "isArchived": { "type": "boolean" },
                "trackingMode": { "type": "string", "enum": ["TRANSACTIONS", "HOLDINGS"] }
            },
            "required": ["accountId"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::AccountsWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let args: UpdateAccountSettingsArgs = serde_json::from_value(args)?;
        let service = env.account_service();
        let current = service.get_account(&args.account_id).map_err(exec_err)?;

        let name = match args.name.as_deref().map(str::trim) {
            Some("") => {
                return Err(AgentToolError::InvalidInput(
                    "name cannot be empty".to_string(),
                ))
            }
            Some(name) => name.to_string(),
            None => current.name.clone(),
        };
        let group = match args.group.as_deref().map(str::trim) {
            Some("") => None,
            Some(group) => Some(group.to_string()),
            None => current.group.clone(),
        };

        let update = AccountUpdate {
            id: Some(current.id.clone()),
            name,
            account_type: current.account_type.clone(),
            group,
            is_default: current.is_default,
            is_active: args.is_active.unwrap_or(current.is_active),
            platform_id: current.platform_id.clone(),
            account_number: current.account_number.clone(),
            meta: current.meta.clone(),
            provider: current.provider.clone(),
            provider_account_id: current.provider_account_id.clone(),
            is_archived: Some(args.is_archived.unwrap_or(current.is_archived)),
            tracking_mode: Some(args.tracking_mode.unwrap_or(current.tracking_mode)),
        };
        let account = service.update_account(update).await.map_err(exec_err)?;
        Ok(AgentToolResult {
            content: serde_json::json!({
                "account": {
                    "id": account.id,
                    "name": account.name,
                    "group": account.group,
                    "isActive": account.is_active,
                    "isArchived": account.is_archived,
                    "trackingMode": account.tracking_mode,
                }
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// set_asset_quote_mode
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetAssetQuoteModeArgs {
    asset_id: String,
    quote_mode: String,
}

/// Switch an asset between MARKET and MANUAL pricing.
pub struct SetAssetQuoteMode;

#[async_trait::async_trait]
impl AgentTool for SetAssetQuoteMode {
    fn name(&self) -> &'static str {
        "set_asset_quote_mode"
    }

    fn description(&self) -> &'static str {
        "Switch an asset's pricing between MARKET (synced from the data provider) and \
         MANUAL (only manual prices are used). This MUTATES data."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "assetId": { "type": "string" },
                "quoteMode": { "type": "string", "enum": ["MARKET", "MANUAL"] }
            },
            "required": ["assetId", "quoteMode"]
        })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::AccountsWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let args: SetAssetQuoteModeArgs = serde_json::from_value(args)?;
        let mode = args.quote_mode.trim().to_ascii_uppercase();
        if mode != "MARKET" && mode != "MANUAL" {
            return Err(AgentToolError::InvalidInput(format!(
                "quoteMode must be MARKET or MANUAL, got '{}'",
                args.quote_mode
            )));
        }
        let asset = env
            .asset_service()
            .update_quote_mode(args.asset_id.trim(), &mode)
            .await
            .map_err(exec_err)?;
        Ok(AgentToolResult {
            content: serde_json::json!({
                "asset": { "id": asset.id, "quoteMode": mode }
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// recalculate_portfolio
// ---------------------------------------------------------------------------

/// Rebuild valuations for all accounts from saved prices.
pub struct RecalculatePortfolio;

#[async_trait::async_trait]
impl AgentTool for RecalculatePortfolio {
    fn name(&self) -> &'static str {
        "recalculate_portfolio"
    }

    fn description(&self) -> &'static str {
        "Queue a full rebuild of holdings and valuations for all accounts from saved \
         prices (no market data fetch). Runs in the background; returns immediately."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn required_scopes(&self) -> &'static [AgentScope] {
        &[AgentScope::ActivitiesWrite]
    }

    fn access_level(&self) -> AgentToolAccess {
        AgentToolAccess::Write
    }

    async fn call(
        &self,
        env: Arc<dyn AgentEnvironment>,
        _args: serde_json::Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        if !request_full_recalculation(&env) {
            return Err(AgentToolError::ExecutionFailed(
                "This host does not support recalculation requests.".to_string(),
            ));
        }
        Ok(AgentToolResult {
            content: serde_json::json!({ "queued": true }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutate_activities_audit_summary_is_compact() {
        let args = serde_json::json!({
            "creates": [{ "accountId": "a", "notes": "secret" }],
            "updates": [{ "id": "u1", "notes": "x" }, { "id": "u2" }],
            "deleteIds": ["d1"],
        });
        let summary = MutateActivities.sanitize_args_for_audit(&args);
        assert_eq!(summary["creates"], 1);
        assert_eq!(summary["updates"], 2);
        assert_eq!(summary["updateIds"], serde_json::json!(["u1", "u2"]));
        assert_eq!(summary["deleteIds"], serde_json::json!(["d1"]));
        assert!(summary.get("notes").is_none());
    }

    #[test]
    fn bulk_request_accepts_camel_case_payload() {
        let request: ActivityBulkMutationRequest = serde_json::from_value(serde_json::json!({
            "creates": [{
                "accountId": "acc",
                "activityType": "DIVIDEND",
                "subtype": "DIVIDEND_IN_KIND",
                "activityDate": "2026-01-30",
                "currency": "CAD",
                "asset": { "id": "asset-1" },
                "quantity": 9.7073,
                "unitPrice": 9.6257,
                "metadata": "{\"flow\":{\"is_external\":false}}"
            }],
            "deleteIds": ["x"]
        }))
        .unwrap();
        assert_eq!(request.creates.len(), 1);
        assert_eq!(
            request.creates[0].subtype.as_deref(),
            Some("DIVIDEND_IN_KIND")
        );
        assert_eq!(request.delete_ids, vec!["x".to_string()]);
    }

    #[test]
    fn manual_quote_args_parse() {
        let args: UpsertManualQuotesArgs = serde_json::from_value(serde_json::json!({
            "quotes": [{ "assetId": "a", "date": "2026-07-30", "close": 10.0, "currency": "cad" }]
        }))
        .unwrap();
        assert_eq!(args.quotes.len(), 1);
        assert!(args.recalculate.is_none());
    }

    #[test]
    fn account_settings_args_parse_tracking_mode() {
        let args: UpdateAccountSettingsArgs = serde_json::from_value(serde_json::json!({
            "accountId": "a", "isActive": false, "trackingMode": "TRANSACTIONS"
        }))
        .unwrap();
        assert_eq!(args.is_active, Some(false));
        assert_eq!(args.tracking_mode, Some(TrackingMode::Transactions));
    }
}
