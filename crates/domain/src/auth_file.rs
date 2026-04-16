use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthTokens {
    pub id_token: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_in: Option<u64>,
    pub scope: Option<String>,
    pub token_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub auth_mode: Option<String>,
    pub last_refresh: Option<String>,
    pub tokens: AuthTokens,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ChatgptPlanType {
    Free,
    Plus,
    Pro,
    Team,
    Enterprise,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthSession {
    pub issuer: String,
    pub client_id: String,
    pub email: Option<String>,
    pub chatgpt_account_id: Option<String>,
    pub plan: ChatgptPlanType,
    pub tokens: AuthTokens,
}

impl AuthFile {
    pub fn from_oauth_session(session: OAuthSession) -> Self {
        Self {
            openai_api_key: None,
            auth_mode: Some("chatgpt".into()),
            last_refresh: None,
            tokens: session.tokens,
        }
    }

    pub fn to_session(
        &self,
        issuer: impl Into<String>,
        client_id: impl Into<String>,
    ) -> OAuthSession {
        let email = self
            .tokens
            .id_token
            .as_deref()
            .and_then(claim_from_jwt)
            .and_then(|claims| {
                claims
                    .get("email")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let chatgpt_account_id = self.tokens.account_id.clone().or_else(|| {
            self.tokens
                .access_token
                .as_str()
                .pipe(extract_account_id_from_access_token)
        });

        OAuthSession {
            issuer: issuer.into(),
            client_id: client_id.into(),
            email,
            chatgpt_account_id,
            plan: ChatgptPlanType::Unknown,
            tokens: self.tokens.clone(),
        }
    }
}

pub fn extract_account_id_from_access_token(token: &str) -> Option<String> {
    let claims = claim_from_jwt(token)?;
    let auth = claims.get("https://api.openai.com/auth")?;
    auth.get("chatgpt_account_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn claim_from_jwt(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}
