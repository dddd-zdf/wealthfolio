//! Fork: health warning for imported or synced activities awaiting review.
//!
//! `HealthService` collects the activities with `needs_review_activities_from_data`;
//! `DataConsistencyCheck::analyze` turns them into one issue with
//! `needs_review_issue`.

use std::collections::HashMap;

use super::{compute_data_hash, ConsistencyIssueInfo, ConsistencyIssueType};
use crate::activities::Activity;
use crate::health::model::{AffectedItem, HealthCategory, HealthIssue, NavigateAction, Severity};
use crate::utils::time_utils::{activity_date_in_tz, parse_user_timezone_or_default};

/// One consistency entry per activity that needs review, labelled with its
/// account name and local date.
pub(crate) fn needs_review_activities_from_data(
    activities: &[Activity],
    account_name_map: &HashMap<String, String>,
    timezone: Option<&str>,
) -> Vec<ConsistencyIssueInfo> {
    let tz = parse_user_timezone_or_default(timezone.unwrap_or_default());
    activities
        .iter()
        .filter(|activity| activity.needs_review)
        .map(|activity| ConsistencyIssueInfo {
            issue_type: ConsistencyIssueType::ActivityNeedsReview,
            record_id: activity.id.clone(),
            description: account_name_map
                .get(&activity.account_id)
                .cloned()
                .unwrap_or_else(|| activity.account_id.clone()),
            account_id: Some(activity.account_id.clone()),
            asset_id: activity.asset_id.clone(),
            first_negative_date: None,
            cash_balance: None,
            total_value_at_date: None,
            account_currency: None,
            activity_date: Some(activity_date_in_tz(activity.activity_date, tz)),
            asset_symbol: None,
            asset_name: None,
            quantity: activity.quantity,
            proceeds: None,
            reason: None,
            activity_id: Some(activity.id.clone()),
            snapshot_date_raw: None,
            snapshot_source: None,
            snapshot_min_date: None,
            snapshot_max_date: None,
        })
        .collect()
}

/// The single warning that links to the review queue.
pub(super) fn needs_review_issue(review_issues: &[&ConsistencyIssueInfo]) -> HealthIssue {
    let count = review_issues.len();
    let record_ids: Vec<String> = review_issues
        .iter()
        .map(|issue| issue.record_id.clone())
        .collect();
    let data_hash = compute_data_hash(&record_ids);
    let affected_items = review_issues
        .iter()
        .map(|issue| {
            let date = issue
                .activity_date
                .map(|date| date.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "unknown date".to_string());
            AffectedItem::activity(
                issue.record_id.clone(),
                format!("{} — {}", issue.description, date),
            )
        })
        .collect();

    HealthIssue::builder()
        .id(format!("activities_need_review:{data_hash}"))
        .severity(Severity::Warning)
        .category(HealthCategory::DataConsistency)
        .code("data_activities_need_review")
        .param("count", count as u32)
        .title(if count == 1 {
            "Transaction needs review".to_string()
        } else {
            format!("{count} transactions need review")
        })
        .message(
            "Some imported or synced transactions are waiting for your review. Drafts aren't counted in balances or performance until you approve them. Approve, edit, or delete them in the review list.",
        )
        .affected_count(count as u32)
        .affected_items(affected_items)
        .navigate_action(NavigateAction {
            route: "/activities".to_string(),
            query: Some(serde_json::json!({ "needsReview": "true" })),
            label: "Review Transactions".to_string(),
        })
        .data_hash(data_hash)
        .build()
}
