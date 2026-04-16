use crate::models::{UsagePayload, UsageWindow};
use chrono::{DateTime, Utc};
use domain::{ChatgptPlanType, LimitWindow, LimitsSnapshot, LimitsSnapshotSet};

pub fn map_usage_payload(payload: UsagePayload) -> LimitsSnapshotSet {
    let plan_type = map_plan(payload.plan_type.as_deref());
    let primary_limit = LimitsSnapshot {
        limit_id: "codex".into(),
        limit_name: None,
        plan_type,
        primary: payload
            .rate_limit
            .as_ref()
            .and_then(|limit| limit.primary_window.as_ref())
            .map(map_window),
        secondary: payload
            .rate_limit
            .as_ref()
            .and_then(|limit| limit.secondary_window.as_ref())
            .map(map_window),
    };
    let additional_limits = payload
        .additional_rate_limits
        .unwrap_or_default()
        .into_iter()
        .map(|entry| LimitsSnapshot {
            limit_id: entry.metered_feature,
            limit_name: Some(entry.limit_name),
            plan_type,
            primary: entry
                .rate_limit
                .as_ref()
                .and_then(|limit| limit.primary_window.as_ref())
                .map(map_window),
            secondary: entry
                .rate_limit
                .as_ref()
                .and_then(|limit| limit.secondary_window.as_ref())
                .map(map_window),
        })
        .collect();

    LimitsSnapshotSet {
        email: payload.email,
        account_id: payload.account_id,
        plan_type,
        primary_limit,
        additional_limits,
    }
}

fn map_window(window: &UsageWindow) -> LimitWindow {
    LimitWindow {
        label: window_label(window.limit_window_seconds),
        used_percent: window.used_percent,
        limit_window_seconds: window.limit_window_seconds,
        resets_at: DateTime::<Utc>::from_timestamp(window.reset_at, 0),
    }
}

fn window_label(seconds: Option<i64>) -> String {
    match seconds {
        Some(18_000) => "5h".into(),
        Some(604_800) => "weekly".into(),
        Some(other) => format!("{}m", other / 60),
        None => "window".into(),
    }
}

fn map_plan(raw: Option<&str>) -> ChatgptPlanType {
    match raw {
        Some("free") => ChatgptPlanType::Free,
        Some("plus") => ChatgptPlanType::Plus,
        Some("pro") => ChatgptPlanType::Pro,
        Some("team") => ChatgptPlanType::Team,
        Some("enterprise") => ChatgptPlanType::Enterprise,
        _ => ChatgptPlanType::Unknown,
    }
}
