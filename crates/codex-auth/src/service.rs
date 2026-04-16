use crate::{AuthError, PkceVerifier};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use domain::{AuthFile, AuthTokens, OAuthSession, extract_account_id_from_access_token};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use url::Url;

const DEFAULT_ISSUER: &str = "https://auth.openai.com";
const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const DEFAULT_REDIRECT_PORT: u16 = 1455;

#[derive(Debug, Clone)]
pub struct AuthPrompt {
    pub authorize_url: Url,
    pub redirect_url: Url,
    pub pkce: PkceVerifier,
}

#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    pub verification_url: String,
    pub user_code: String,
    pub device_auth_id: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct BrowserAuthOptions {
    pub issuer: String,
    pub client_id: String,
    pub port: u16,
    pub open_browser: bool,
    pub timeout: Duration,
}

impl Default for BrowserAuthOptions {
    fn default() -> Self {
        Self {
            issuer: DEFAULT_ISSUER.into(),
            client_id: DEFAULT_CLIENT_ID.into(),
            port: DEFAULT_REDIRECT_PORT,
            open_browser: true,
            timeout: Duration::from_secs(15 * 60),
        }
    }
}

pub struct CodexAuthService {
    http: Client,
}

impl CodexAuthService {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    pub fn default_issuer(&self) -> &'static str {
        DEFAULT_ISSUER
    }

    pub fn default_client_id(&self) -> &'static str {
        DEFAULT_CLIENT_ID
    }

    pub fn begin_browser_login(
        &self,
        options: &BrowserAuthOptions,
    ) -> Result<AuthPrompt, AuthError> {
        let pkce = PkceVerifier::generate();
        let redirect_url = Url::parse(&format!("http://localhost:{}/auth/callback", options.port))
            .map_err(|err| AuthError::Callback(err.to_string()))?;
        let authorize_url = Url::parse_with_params(
            &format!("{}/oauth/authorize", options.issuer.trim_end_matches('/')),
            &[
                ("response_type", "code"),
                ("client_id", options.client_id.as_str()),
                ("redirect_uri", redirect_url.as_str()),
                ("scope", DEFAULT_SCOPES),
                ("code_challenge", pkce.code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("state", pkce.state.as_str()),
                ("originator", "codex_cli_rs"),
            ],
        )
        .map_err(|err| AuthError::Callback(err.to_string()))?;

        Ok(AuthPrompt {
            authorize_url,
            redirect_url,
            pkce,
        })
    }

    pub fn login_with_browser(
        &self,
        options: &BrowserAuthOptions,
    ) -> Result<OAuthSession, AuthError> {
        let prompt = self.begin_browser_login(options)?;
        if options.open_browser {
            webbrowser::open(prompt.authorize_url.as_str())
                .map_err(|err| AuthError::BrowserOpen(err.to_string()))?;
        }

        let query = wait_for_callback(&prompt, options.timeout)?;
        if query.get("state").map(String::as_str) != Some(prompt.pkce.state.as_str()) {
            return Err(AuthError::StateMismatch);
        }
        if let Some(error) = query.get("error") {
            return Err(AuthError::Authorization(error.clone()));
        }
        let code = query
            .get("code")
            .ok_or_else(|| AuthError::Authorization("missing authorization code".into()))?;
        self.exchange_code_for_tokens(
            &options.issuer,
            &options.client_id,
            prompt.redirect_url.as_str(),
            code,
            &prompt.pkce.code_verifier,
        )
    }

    pub fn request_device_code(
        &self,
        issuer: &str,
        client_id: &str,
    ) -> Result<DeviceCodePrompt, AuthError> {
        let response = self
            .http
            .post(format!(
                "{}/api/accounts/deviceauth/usercode",
                issuer.trim_end_matches('/')
            ))
            .json(&serde_json::json!({ "client_id": client_id }))
            .send()
            .map_err(|err| AuthError::Http(err.to_string()))?
            .error_for_status()
            .map_err(|err| AuthError::Http(err.to_string()))?;

        #[derive(Deserialize)]
        struct DeviceCodeResponse {
            device_auth_id: String,
            user_code: Option<String>,
            usercode: Option<String>,
            interval: Option<String>,
        }

        let body: DeviceCodeResponse = response
            .json()
            .map_err(|err| AuthError::Http(err.to_string()))?;
        Ok(DeviceCodePrompt {
            verification_url: format!("{}/codex/device", issuer.trim_end_matches('/')),
            user_code: body.user_code.or(body.usercode).unwrap_or_default(),
            device_auth_id: body.device_auth_id,
            interval_seconds: body
                .interval
                .and_then(|value| value.parse().ok())
                .unwrap_or(5),
        })
    }

    pub fn complete_device_login(
        &self,
        issuer: &str,
        client_id: &str,
        prompt: &DeviceCodePrompt,
        timeout: Duration,
    ) -> Result<OAuthSession, AuthError> {
        let deadline = Instant::now() + timeout;
        loop {
            let response = self
                .http
                .post(format!(
                    "{}/api/accounts/deviceauth/token",
                    issuer.trim_end_matches('/')
                ))
                .json(&serde_json::json!({
                    "device_auth_id": prompt.device_auth_id,
                    "user_code": prompt.user_code,
                }))
                .send()
                .map_err(|err| AuthError::Http(err.to_string()))?;

            match response.error_for_status() {
                Ok(ok_response) => {
                    let body: Value = ok_response
                        .json()
                        .map_err(|err| AuthError::Http(err.to_string()))?;
                    let authorization_code = body
                        .get("authorization_code")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            AuthError::Authorization(
                                "device auth response missing authorization_code".into(),
                            )
                        })?;
                    let code_verifier = body
                        .get("code_verifier")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            AuthError::Authorization(
                                "device auth response missing code_verifier".into(),
                            )
                        })?;
                    return self.exchange_code_for_tokens(
                        issuer,
                        client_id,
                        &format!("{}/deviceauth/callback", issuer.trim_end_matches('/')),
                        authorization_code,
                        code_verifier,
                    );
                }
                Err(err) => {
                    let status = err
                        .status()
                        .map(|status| status.as_u16())
                        .unwrap_or_default();
                    if (status == 403 || status == 404) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_secs(prompt.interval_seconds));
                        continue;
                    }
                    return Err(AuthError::Http(err.to_string()));
                }
            }
        }
    }

    pub fn refresh_session(&self, session: &OAuthSession) -> Result<OAuthSession, AuthError> {
        let refresh_token = session
            .tokens
            .refresh_token
            .as_deref()
            .ok_or_else(|| AuthError::Authorization("missing refresh token".into()))?;
        let response = self
            .http
            .post(format!(
                "{}/oauth/token",
                session.issuer.trim_end_matches('/')
            ))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", session.client_id.as_str()),
                ("refresh_token", refresh_token),
            ])
            .send()
            .map_err(|err| AuthError::Http(err.to_string()))?
            .error_for_status()
            .map_err(|err| AuthError::Http(err.to_string()))?;
        let tokens: TokenResponse = response
            .json()
            .map_err(|err| AuthError::Http(err.to_string()))?;
        Ok(self.session_from_token_response(&session.issuer, &session.client_id, tokens))
    }

    pub fn auth_file_from_session(&self, session: OAuthSession) -> AuthFile {
        AuthFile::from_oauth_session(session)
    }

    fn exchange_code_for_tokens(
        &self,
        issuer: &str,
        client_id: &str,
        redirect_uri: &str,
        code: &str,
        code_verifier: &str,
    ) -> Result<OAuthSession, AuthError> {
        let response = self
            .http
            .post(format!("{}/oauth/token", issuer.trim_end_matches('/')))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", client_id),
                ("code_verifier", code_verifier),
            ])
            .send()
            .map_err(|err| AuthError::Http(err.to_string()))?
            .error_for_status()
            .map_err(|err| AuthError::Http(err.to_string()))?;
        let tokens: TokenResponse = response
            .json()
            .map_err(|err| AuthError::Http(err.to_string()))?;
        Ok(self.session_from_token_response(issuer, client_id, tokens))
    }

    fn session_from_token_response(
        &self,
        issuer: &str,
        client_id: &str,
        tokens: TokenResponse,
    ) -> OAuthSession {
        let account_id = extract_account_id_from_access_token(&tokens.access_token);
        let email = tokens.id_token.as_deref().and_then(email_from_id_token);
        OAuthSession {
            issuer: issuer.into(),
            client_id: client_id.into(),
            email,
            chatgpt_account_id: account_id.clone(),
            plan: domain::ChatgptPlanType::Unknown,
            tokens: AuthTokens {
                id_token: tokens.id_token,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                account_id,
                expires_in: tokens.expires_in,
                scope: tokens.scope,
                token_type: tokens.token_type,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
    token_type: Option<String>,
}

fn wait_for_callback(
    prompt: &AuthPrompt,
    timeout: Duration,
) -> Result<HashMap<String, String>, AuthError> {
    let listener = TcpListener::bind((
        "127.0.0.1",
        prompt.redirect_url.port().unwrap_or(DEFAULT_REDIRECT_PORT),
    ))
    .map_err(|err| AuthError::Callback(err.to_string()))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| AuthError::Callback(err.to_string()))?;

    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(AuthError::CallbackTimeout);
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buffer = [0u8; 4096];
                let read = stream
                    .read(&mut buffer)
                    .map_err(|err| AuthError::Callback(err.to_string()))?;
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .ok_or_else(|| AuthError::Callback("invalid callback request".into()))?;
                let url = Url::parse(&format!("http://localhost{path}"))
                    .map_err(|err| AuthError::Callback(err.to_string()))?;

                let response = if url.query_pairs().any(|(key, _)| key == "error") {
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nOAuth failed\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nLogin completed. You can close this tab.\r\n"
                };
                let _ = stream.write_all(response.as_bytes());

                return Ok(url
                    .query_pairs()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect());
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(AuthError::Callback(err.to_string())),
        }
    }
}

fn email_from_id_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
