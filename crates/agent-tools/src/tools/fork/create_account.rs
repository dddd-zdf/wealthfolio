//! Create Account tool (MCP-only).
//!
//! `create_account` creates a Wealthfolio account directly through the
//! account service — the same path the UI's "add account" flow uses
//! (`AccountServiceTrait::create_account`, including FX pair registration
//! and the accounts-changed domain event). MCP callers have no settings UI,
//! so this tool lets a scoped token create accounts without one.
//!
//! The account type is validated against the known `account_types`
//! constants (`SECURITIES`, `CASH`, `CREDIT_CARD`, `CRYPTOCURRENCY`);
//! name/currency emptiness is rejected up front, and the service's own
//! `NewAccount::validate` runs again at the persistence boundary.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wealthfolio_core::accounts::{account_types, NewAccount};

use crate::env::AgentEnvironment;
use crate::scope::AgentScope;
use crate::tool::{AgentTool, AgentToolAccess, AgentToolError, AgentToolResult};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountArgs {
    /// Display name for the new account (required, non-empty).
    pub name: String,
    /// Account type: SECURITIES, CASH, CREDIT_CARD, or CRYPTOCURRENCY.
    /// Case-insensitive; normalized to uppercase before validation.
    pub account_type: String,
    /// ISO currency code, e.g. "CAD" (required, non-empty).
    pub currency: String,
    /// Optional grouping label shown in the UI.
    #[serde(default)]
    pub group: Option<String>,
    /// Whether this becomes the default account. Defaults to false.
    #[serde(default)]
    pub is_default: bool,
}

/// DTO for the created account.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedAccountDto {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountOutput {
    pub account: CreatedAccountDto,
}

/// Normalize and validate the requested account type against the known
/// `account_types` constants. Rejects unknown types before anything is
/// persisted.
fn resolve_account_type(raw: &str) -> Result<&'static str, AgentToolError> {
    match raw.trim().to_ascii_uppercase().as_str() {
        account_types::SECURITIES => Ok(account_types::SECURITIES),
        account_types::CASH => Ok(account_types::CASH),
        account_types::CREDIT_CARD => Ok(account_types::CREDIT_CARD),
        account_types::CRYPTOCURRENCY => Ok(account_types::CRYPTOCURRENCY),
        other => Err(AgentToolError::InvalidInput(format!(
            "Unknown accountType '{other}'. Expected one of: SECURITIES, CASH, CREDIT_CARD, CRYPTOCURRENCY."
        ))),
    }
}

/// Build the `NewAccount` from validated args. Name/currency emptiness is
/// checked here for a clean error; `NewAccount::validate` enforces it again
/// at the persistence boundary.
fn build_new_account(args: &CreateAccountArgs) -> Result<NewAccount, AgentToolError> {
    let name = args.name.trim();
    if name.is_empty() {
        return Err(AgentToolError::InvalidInput(
            "Account name cannot be empty".to_string(),
        ));
    }
    let currency = args.currency.trim().to_ascii_uppercase();
    if currency.is_empty() {
        return Err(AgentToolError::InvalidInput(
            "Currency cannot be empty".to_string(),
        ));
    }
    let account_type = resolve_account_type(&args.account_type)?;
    Ok(NewAccount {
        id: None,
        name: name.to_string(),
        account_type: account_type.to_string(),
        group: args
            .group
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_string),
        currency,
        is_default: args.is_default,
        is_active: true,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        is_archived: false,
        tracking_mode: Default::default(),
    })
}

/// Create a Wealthfolio account.
pub struct CreateAccount;

#[async_trait::async_trait]
impl AgentTool for CreateAccount {
    fn name(&self) -> &'static str {
        "create_account"
    }

    fn description(&self) -> &'static str {
        "Create a Wealthfolio account (SECURITIES, CASH, CREDIT_CARD, or \
         CRYPTOCURRENCY). Pass name, accountType, and currency; group and \
         isDefault are optional. This MUTATES data — the account is created \
         immediately through the account service."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Display name for the new account." },
                "accountType": {
                    "type": "string",
                    "enum": ["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"],
                    "description": "Account type (case-insensitive)."
                },
                "currency": { "type": "string", "description": "ISO currency code, e.g. \"CAD\"." },
                "group": { "type": "string", "description": "Optional grouping label." },
                "isDefault": { "type": "boolean", "description": "Make this the default account. Defaults to false." }
            },
            "required": ["name", "accountType", "currency"]
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
        let args: CreateAccountArgs = serde_json::from_value(args)?;
        let new_account = build_new_account(&args)?;

        let account = env
            .account_service()
            .create_account(new_account)
            .await
            .map_err(|e| AgentToolError::ExecutionFailed(e.to_string()))?;

        let output = CreateAccountOutput {
            account: CreatedAccountDto {
                id: account.id,
                name: account.name,
                account_type: account.account_type,
                currency: account.currency,
                group: account.group,
                is_active: account.is_active,
            },
        };
        Ok(AgentToolResult {
            content: serde_json::to_value(output)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(name: &str, account_type: &str, currency: &str) -> CreateAccountArgs {
        CreateAccountArgs {
            name: name.to_string(),
            account_type: account_type.to_string(),
            currency: currency.to_string(),
            group: None,
            is_default: false,
        }
    }

    #[test]
    fn accepts_known_account_types_case_insensitively() {
        for (raw, expected) in [
            ("securities", account_types::SECURITIES),
            ("CASH", account_types::CASH),
            ("Credit_Card", account_types::CREDIT_CARD),
            ("cryptocurrency", account_types::CRYPTOCURRENCY),
        ] {
            assert_eq!(resolve_account_type(raw).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_unknown_account_type() {
        let err = resolve_account_type("SAVINGS").unwrap_err();
        assert!(matches!(err, AgentToolError::InvalidInput(_)));
        assert!(err.to_string().contains("SAVINGS"));
    }

    #[test]
    fn rejects_empty_name() {
        let err = build_new_account(&make_args("   ", "CASH", "CAD")).unwrap_err();
        assert!(matches!(err, AgentToolError::InvalidInput(_)));
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn rejects_empty_currency() {
        let err = build_new_account(&make_args("My Cash", "CASH", "  ")).unwrap_err();
        assert!(matches!(err, AgentToolError::InvalidInput(_)));
        assert!(err.to_string().contains("Currency"));
    }

    #[test]
    fn builds_sane_defaults() {
        let account = build_new_account(&make_args("My Cash", "cash", "cad")).unwrap();
        assert_eq!(account.name, "My Cash");
        assert_eq!(account.account_type, "CASH");
        assert_eq!(account.currency, "CAD");
        assert!(account.is_active);
        assert!(!account.is_default);
        assert!(!account.is_archived);
        assert!(account.id.is_none());
    }

    #[test]
    fn blank_group_is_dropped() {
        let mut args = make_args("My Cash", "CASH", "CAD");
        args.group = Some("   ".to_string());
        let account = build_new_account(&args).unwrap();
        assert!(account.group.is_none());
    }

    #[test]
    fn create_account_schema_requires_name_type_currency() {
        let schema = CreateAccount.input_schema();
        assert_eq!(
            schema["required"],
            serde_json::json!(["name", "accountType", "currency"])
        );
        assert_eq!(
            schema["properties"]["accountType"]["enum"],
            serde_json::json!(["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"])
        );
    }

    #[test]
    fn create_account_requires_accounts_write_scope() {
        assert_eq!(
            CreateAccount.required_scopes(),
            &[AgentScope::AccountsWrite]
        );
        assert_eq!(CreateAccount.access_level(), AgentToolAccess::Write);
    }
}
