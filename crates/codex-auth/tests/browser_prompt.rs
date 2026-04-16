use codex_auth::BrowserAuthOptions;
use codex_auth::CodexAuthService;

#[test]
fn builds_browser_authorize_url() {
    let service = CodexAuthService::new();
    let options = BrowserAuthOptions::default();
    let prompt = service.begin_browser_login(&options).unwrap();
    let url = prompt.authorize_url.as_str();

    assert!(url.contains("/oauth/authorize"));
    assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
    assert!(url.contains("code_challenge_method=S256"));
}
