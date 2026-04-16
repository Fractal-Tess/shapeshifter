use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("http auth request failed: {0}")]
    Http(String),
    #[error("oauth callback listener failed: {0}")]
    Callback(String),
    #[error("oauth callback timed out")]
    CallbackTimeout,
    #[error("oauth state mismatch")]
    StateMismatch,
    #[error("oauth authorization failed: {0}")]
    Authorization(String),
    #[error("browser open failed: {0}")]
    BrowserOpen(String),
}
