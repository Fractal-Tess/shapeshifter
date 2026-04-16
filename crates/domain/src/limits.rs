use crate::ChatgptPlanType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitWindow {
    pub label: String,
    pub used_percent: f64,
    pub limit_window_seconds: Option<i64>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsSnapshot {
    pub limit_id: String,
    pub limit_name: Option<String>,
    pub plan_type: ChatgptPlanType,
    pub primary: Option<LimitWindow>,
    pub secondary: Option<LimitWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsSnapshotSet {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: ChatgptPlanType,
    pub primary_limit: LimitsSnapshot,
    pub additional_limits: Vec<LimitsSnapshot>,
}
