use anyhow::{Context, Result};
use domain::{AuthFile, AuthTokens, HostTarget};
use host_ops::HostOperator;
use profile_store::ProfileStore;

pub fn run() -> Result<()> {
    let store = ProfileStore::new();
    let operator = HostOperator::new();
    let auth_service = codex_auth::CodexAuthService::new();
    let limits_client = codex_limits::CodexLimitsClient::new("https://chatgpt.com");

    let current_auth = store
        .load_current_auth()
        .with_context(|| format!("failed to load {}", store.auth_file_path().display()))?;
    let profiles = store
        .list_profiles()
        .context("failed to list saved profiles")?;
    let hosts = store.load_hosts().context("failed to load hosts")?;
    let session = current_auth.to_session(
        auth_service.default_issuer(),
        auth_service.default_client_id(),
    );
    let limits = limits_client
        .fetch(&session)
        .context("failed to fetch live limits")?;

    println!("Loaded {}", store.auth_file_path().display());
    println!("Found {} saved profiles", profiles.len());
    println!(
        "Fetched limits: plan={:?}, email={}",
        limits.plan_type,
        limits.email.as_deref().unwrap_or("unknown")
    );
    if let Some(primary) = limits.primary_limit.primary.as_ref() {
        println!(
            "Primary window: {} at {}%",
            primary.label, primary.used_percent
        );
    }
    if let Some(secondary) = limits.primary_limit.secondary.as_ref() {
        println!(
            "Secondary window: {} at {}%",
            secondary.label, secondary.used_percent
        );
    }

    if let Some(remote_host) = hosts
        .iter()
        .find(|host| matches!(host.target, HostTarget::Remote(_)))
    {
        let sample = AuthFile {
            auth_mode: Some("chatgpt".into()),
            tokens: AuthTokens {
                access_token: "system-check".into(),
                ..AuthTokens::default()
            },
            ..AuthFile::default()
        };

        let temp_host = match &remote_host.target {
            HostTarget::Remote(remote) => domain::ManagedHost {
                id: remote_host.id.clone(),
                label: format!("{}-temp-check", remote_host.label),
                target: HostTarget::Remote(domain::RemoteHost {
                    ssh_alias: remote.ssh_alias.clone(),
                    auth_file_path: std::path::PathBuf::from(
                        "~/tmp/shapeshifter-system-check-auth.json",
                    ),
                    managed_data_dir: remote.managed_data_dir.clone(),
                }),
            },
            HostTarget::Local { .. } => unreachable!(),
        };

        operator
            .write_auth(&temp_host, &sample)
            .context("failed to write remote system-check auth file")?;
        let roundtrip = operator
            .read_auth(&temp_host)
            .context("failed to read remote system-check auth file")?;
        println!(
            "Remote SSH roundtrip succeeded on {} with access_token={}",
            remote_host.label, roundtrip.tokens.access_token
        );
    }

    Ok(())
}
