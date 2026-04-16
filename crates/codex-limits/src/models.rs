use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct UsagePayload {
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub rate_limit: Option<UsageRateLimitDetails>,
    pub additional_rate_limits: Option<Vec<AdditionalUsageLimit>>,
}

#[derive(Debug, Deserialize)]
pub struct AdditionalUsageLimit {
    pub metered_feature: String,
    pub limit_name: String,
    pub rate_limit: Option<UsageRateLimitDetails>,
}

#[derive(Debug, Deserialize)]
pub struct UsageRateLimitDetails {
    pub primary_window: Option<UsageWindow>,
    pub secondary_window: Option<UsageWindow>,
}

#[derive(Debug, Deserialize)]
pub struct UsageWindow {
    pub used_percent: f64,
    pub limit_window_seconds: Option<i64>,
    pub reset_at: i64,
}
