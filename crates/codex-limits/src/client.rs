use crate::mapper::map_usage_payload;
use crate::models::UsagePayload;
use anyhow::{Context, Result};
use domain::{LimitsSnapshotSet, OAuthSession};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use std::time::Duration;

pub struct CodexLimitsClient {
    base_url: String,
    http: Client,
}

impl CodexLimitsClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = normalize_base_url(base_url.into());
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");
        Self { base_url, http }
    }

    pub fn usage_url(&self) -> String {
        format!("{}/codex/usage", self.base_url.trim_end_matches('/'))
    }

    pub fn fetch(&self, session: &OAuthSession) -> Result<LimitsSnapshotSet> {
        let response = self
            .http
            .get(self.usage_url())
            .headers(self.headers(session)?)
            .send()
            .context("failed to call codex usage endpoint")?
            .error_for_status()
            .context("codex usage endpoint returned an error")?;
        let payload: UsagePayload = response.json().context("failed to decode usage payload")?;
        Ok(map_usage_payload(payload))
    }

    fn headers(&self, session: &OAuthSession) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("codex_cli_rs/0.120.0 shapeshifter"),
        );
        headers.insert("originator", HeaderValue::from_static("codex_cli_rs"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", session.tokens.access_token))
                .context("invalid access token header")?,
        );
        if let Some(account_id) = session.chatgpt_account_id.as_deref() {
            headers.insert(
                "ChatGPT-Account-ID",
                HeaderValue::from_str(account_id).context("invalid account id header")?,
            );
        }
        Ok(headers)
    }
}

fn normalize_base_url(base_url: String) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/backend-api") {
        trimmed.to_string()
    } else if trimmed.starts_with("https://chatgpt.com")
        || trimmed.starts_with("https://chat.openai.com")
    {
        format!("{trimmed}/backend-api")
    } else {
        trimmed.to_string()
    }
}
