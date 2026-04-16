use domain::{AuthFile, AuthTokens, HostTarget, ManagedHost};
use host_ops::HostOperator;
use std::fs;

#[test]
fn writes_local_auth_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let auth_path = temp.path().join("auth.json");
    let host = ManagedHost {
        id: "local".into(),
        label: "local".into(),
        target: HostTarget::Local {
            auth_file_path: auth_path.clone(),
        },
    };
    let operator = HostOperator::new();

    let auth = AuthFile {
        auth_mode: Some("chatgpt".into()),
        tokens: AuthTokens {
            access_token: "abc".into(),
            ..AuthTokens::default()
        },
        ..AuthFile::default()
    };
    operator.write_auth(&host, &auth).unwrap();

    let text = fs::read_to_string(auth_path).unwrap();
    assert!(text.contains("\"access_token\": \"abc\""));
}
