use anyhow::{Context, Result, bail};
use domain::{AccountProfile, AuthFile, HostTarget, ManagedHost, RemoteHost};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct HostOperator;

#[derive(Debug, Clone)]
pub struct RemoteHostSnapshot {
    pub profiles: Vec<AccountProfile>,
    pub active_auth: AuthFile,
}

impl Default for HostOperator {
    fn default() -> Self {
        Self::new()
    }
}

impl HostOperator {
    pub fn new() -> Self {
        Self
    }

    pub fn read_auth(&self, host: &ManagedHost) -> Result<AuthFile> {
        match &host.target {
            HostTarget::Local { auth_file_path } => {
                let text = fs::read_to_string(auth_file_path)?;
                Ok(serde_json::from_str(&text)?)
            }
            HostTarget::Remote(remote) => self.read_remote_auth(remote),
        }
    }

    pub fn write_auth(&self, host: &ManagedHost, auth_file: &AuthFile) -> Result<()> {
        match &host.target {
            HostTarget::Local { auth_file_path } => write_local_auth(auth_file_path, auth_file),
            HostTarget::Remote(remote) => self.write_remote_auth(remote, auth_file),
        }
    }

    pub fn sync_managed_data_dir(
        &self,
        remote: &RemoteHost,
        local_managed_data_dir: &Path,
    ) -> Result<()> {
        let managed_dir = remote_managed_data_dir_expr(remote);
        let destination = format!("{}:{}/", remote.ssh_alias, managed_dir);
        run_command(
            Command::new("ssh")
                .arg(&remote.ssh_alias)
                .arg(remote_bash(format!("mkdir -p {managed_dir}"))),
            "failed to prepare remote managed data directory",
        )?;
        run_command(
            Command::new("rsync")
                .arg("-az")
                .arg("--delete")
                .arg(format!("{}/", local_managed_data_dir.display()))
                .arg(destination),
            "failed to sync managed data directory over rsync",
        )?;
        Ok(())
    }

    pub fn inspect_remote_host(&self, remote: &RemoteHost) -> Result<RemoteHostSnapshot> {
        let active_auth = self.read_remote_auth(remote)?;
        let profiles = self.read_remote_profiles(remote)?;
        Ok(RemoteHostSnapshot {
            profiles,
            active_auth,
        })
    }

    fn read_remote_auth(&self, remote: &RemoteHost) -> Result<AuthFile> {
        let candidate_paths = remote_auth_candidates(remote);
        let mut failures = Vec::new();

        for remote_path in candidate_paths {
            match run_remote_capture(remote, format!("cat {}", shell_quote(&remote_path))) {
                Ok(output) => return Ok(serde_json::from_slice(&output)?),
                Err(err) => failures.push(format!("{remote_path}: {err}")),
            }
        }

        bail!(
            "failed to locate remote auth file for {}. tried:\n{}",
            remote.ssh_alias,
            failures.join("\n")
        )
    }

    fn write_remote_auth(&self, remote: &RemoteHost, auth_file: &AuthFile) -> Result<()> {
        let payload = format!("{}\n", serde_json::to_string_pretty(auth_file)?);
        let temp = tempfile::NamedTempFile::new()?;
        fs::write(temp.path(), payload)?;

        let remote_path = remote_path_expr(remote);
        let backup_path = format!("{remote_path}.bak.{}", unix_timestamp());
        let tmp_path = format!("{remote_path}.shapeshifter.tmp");

        run_command(
            Command::new("ssh")
                .arg(&remote.ssh_alias)
                .arg(remote_bash(format!(
                    "mkdir -p $(dirname {path}) && if [ -f {path} ]; then cp {path} {backup}; fi",
                    path = remote_path,
                    backup = backup_path,
                ))),
            "failed to prepare remote backup",
        )?;

        run_command(
            Command::new("scp")
                .arg(temp.path())
                .arg(format!("{}:{}", remote.ssh_alias, tmp_path)),
            "failed to copy auth file over scp",
        )?;

        run_command(
            Command::new("ssh")
                .arg(&remote.ssh_alias)
                .arg(remote_bash(format!(
                    "mv {tmp} {target}",
                    tmp = tmp_path,
                    target = remote_path,
                ))),
            "failed to atomically replace remote auth file",
        )?;

        Ok(())
    }

    fn read_remote_profiles(&self, remote: &RemoteHost) -> Result<Vec<AccountProfile>> {
        let accounts_dir = remote_managed_accounts_dir_expr(remote);
        let listing = run_remote_capture(
            remote,
            format!(
                "if [ -d {dir} ]; then find {dir} -maxdepth 1 -type f -name '*.json' | sort; fi",
                dir = shell_quote(&accounts_dir),
            ),
        )?;
        let mut profiles = Vec::new();
        for line in String::from_utf8_lossy(&listing).lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let bytes = run_remote_capture(remote, format!("cat {}", shell_quote(trimmed)))?;
            let auth_file: AuthFile = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid JSON in remote profile {trimmed}"))?;
            let label = Path::new(trimmed)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_string();
            profiles.push(AccountProfile {
                id: label.clone(),
                label,
                source_path: PathBuf::from(trimmed),
                auth_file,
            });
        }
        Ok(profiles)
    }
}

fn write_local_auth(path: &Path, auth_file: &AuthFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let backup = path.with_extension(format!("bak.{}", unix_timestamp()));
        fs::copy(path, backup)?;
    }
    let tmp_path = path.with_extension("shapeshifter.tmp");
    fs::write(
        &tmp_path,
        format!("{}\n", serde_json::to_string_pretty(auth_file)?),
    )?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn run_command(command: &mut Command, context: &str) -> Result<()> {
    let output = command.output().with_context(|| context.to_string())?;
    if !output.status.success() {
        bail!("{context}: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

fn run_remote_capture(remote: &RemoteHost, script: String) -> Result<Vec<u8>> {
    let output = Command::new("ssh")
        .arg(&remote.ssh_alias)
        .arg(remote_bash(script))
        .output()
        .with_context(|| format!("failed to start ssh for {}", remote.ssh_alias))?;
    if !output.status.success() {
        bail!(
            "ssh command failed for {}: {}",
            remote.ssh_alias,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn remote_path_expr(remote: &RemoteHost) -> String {
    remote_path_expr_for(&remote.auth_file_path)
}

fn remote_auth_candidates(remote: &RemoteHost) -> Vec<String> {
    let mut candidates = vec![remote_path_expr(remote)];
    for path in [
        "~/.local/share/opencode/auth.json",
        "~/.codex/auth.json",
        "~/AppData/Local/opencode/auth.json",
    ] {
        if !candidates.iter().any(|candidate| candidate == path) {
            candidates.push(path.to_string());
        }
    }
    candidates
}

fn remote_managed_data_dir_expr(remote: &RemoteHost) -> String {
    remote_path_expr_for(&remote.managed_data_dir)
}

fn remote_managed_accounts_dir_expr(remote: &RemoteHost) -> String {
    remote_path_expr_for(&remote.managed_data_dir.join("accounts"))
}

fn remote_path_expr_for(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value.starts_with("~/") || value.starts_with('/') || looks_like_windows_absolute_path(&value)
    {
        value.into_owned()
    } else {
        format!("~/{value}")
    }
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn remote_bash(script: String) -> String {
    format!("bash -lc '{}'", script.replace('\'', "'\"'\"'"))
}

fn shell_quote(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        format!("~/{}", shell_quote_inner(rest))
    } else {
        shell_quote_inner(value)
    }
}

fn shell_quote_inner(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
