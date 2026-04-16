use codex_limits::CodexLimitsClient;
use domain::{AuthTokens, ChatgptPlanType, OAuthSession};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn fetches_and_maps_live_shape() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            let body = r#"{
              "account_id": "acct-1",
              "email": "user@example.com",
              "plan_type": "team",
              "rate_limit": {
                "primary_window": { "used_percent": 13, "limit_window_seconds": 18000, "reset_at": 1776330882 },
                "secondary_window": { "used_percent": 75, "limit_window_seconds": 604800, "reset_at": 1776719679 }
              },
              "additional_rate_limits": null
            }"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let client = CodexLimitsClient::new(format!("http://{addr}/backend-api"));
    let session = OAuthSession {
        issuer: "https://auth.openai.com".into(),
        client_id: "client".into(),
        email: None,
        chatgpt_account_id: Some("acct-1".into()),
        plan: ChatgptPlanType::Unknown,
        tokens: AuthTokens {
            access_token: "token".into(),
            ..AuthTokens::default()
        },
    };
    let limits = client.fetch(&session).unwrap();
    handle.join().unwrap();

    assert_eq!(limits.plan_type, ChatgptPlanType::Team);
    assert_eq!(limits.primary_limit.primary.as_ref().unwrap().label, "5h");
    assert_eq!(
        limits.primary_limit.secondary.as_ref().unwrap().label,
        "weekly"
    );
}
