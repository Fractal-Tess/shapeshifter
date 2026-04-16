use anyhow::{Context, Result, bail};
use domain::{AccountProfile, AuthFile, HostTarget, ManagedHost, RemoteHost};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ProfileStore {
    active_auth_dir: PathBuf,
    managed_data_dir: PathBuf,
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileStore {
    pub fn new() -> Self {
        Self {
            active_auth_dir: default_auth_storage_dir(),
            managed_data_dir: default_managed_data_dir(),
        }
    }

    pub fn from_dirs(active_auth_dir: PathBuf, managed_data_dir: PathBuf) -> Self {
        Self {
            active_auth_dir,
            managed_data_dir,
        }
    }

    pub fn auth_storage_dir(&self) -> &Path {
        &self.active_auth_dir
    }

    pub fn managed_data_dir(&self) -> &Path {
        &self.managed_data_dir
    }

    pub fn auth_file_path(&self) -> PathBuf {
        self.active_auth_dir.join("auth.json")
    }

    pub fn accounts_dir(&self) -> PathBuf {
        self.managed_data_dir.join("accounts")
    }

    pub fn hosts_file_path(&self) -> PathBuf {
        self.accounts_dir().join(".hosts")
    }

    pub fn load_current_auth(&self) -> Result<AuthFile> {
        self.load_auth_file(&self.auth_file_path())
    }

    pub fn load_auth_file(&self, path: &Path) -> Result<AuthFile> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read auth file {}", path.display()))?;
        let auth: AuthFile = serde_json::from_str(&text)
            .with_context(|| format!("invalid JSON in {}", path.display()))?;
        Ok(auth)
    }

    pub fn list_profiles(&self) -> Result<Vec<AccountProfile>> {
        self.ensure_managed_data_ready()?;
        let accounts_dir = self.accounts_dir();
        fs::create_dir_all(&accounts_dir)?;

        let mut profiles = Vec::new();
        for entry in fs::read_dir(&accounts_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let auth_file = self.load_auth_file(&path)?;
            let label = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_string();
            profiles.push(AccountProfile {
                id: label.clone(),
                label,
                source_path: path,
                auth_file,
            });
        }

        profiles.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(profiles)
    }

    pub fn save_profile(&self, profile_name: &str, auth_file: &AuthFile) -> Result<PathBuf> {
        self.ensure_managed_data_ready()?;
        let trimmed = profile_name.trim();
        if trimmed.is_empty() {
            bail!("profile name cannot be empty");
        }

        let destination = self.accounts_dir().join(format!("{trimmed}.json"));
        fs::create_dir_all(self.accounts_dir())?;
        fs::write(
            &destination,
            format!("{}\n", serde_json::to_string_pretty(auth_file)?),
        )?;
        Ok(destination)
    }

    pub fn delete_profile(&self, profile_name: &str) -> Result<()> {
        self.ensure_managed_data_ready()?;
        let trimmed = profile_name.trim();
        if trimmed.is_empty() {
            bail!("profile name cannot be empty");
        }

        let path = self.accounts_dir().join(format!("{trimmed}.json"));
        if !path.exists() {
            bail!("profile `{trimmed}` does not exist");
        }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn load_hosts(&self) -> Result<Vec<ManagedHost>> {
        self.ensure_managed_data_ready()?;
        let local = ManagedHost {
            id: "local".into(),
            label: "local".into(),
            target: HostTarget::Local {
                auth_file_path: self.auth_file_path(),
            },
        };

        let hosts_path = self.hosts_file_path();
        if !hosts_path.exists() {
            return Ok(vec![local]);
        }

        let mut hosts = vec![local];
        let contents = fs::read_to_string(hosts_path)?;
        hosts.extend(
            contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| {
                    parse_remote_host(line, self.auth_storage_dir(), self.managed_data_dir())
                })
                .collect::<Vec<_>>(),
        );
        Ok(hosts)
    }

    fn ensure_managed_data_ready(&self) -> Result<()> {
        fs::create_dir_all(self.accounts_dir())?;
        migrate_legacy_accounts_if_needed(self.auth_storage_dir(), self.managed_data_dir())
    }
}

fn parse_remote_host(
    line: &str,
    local_auth_storage_dir: &Path,
    local_managed_data_dir: &Path,
) -> ManagedHost {
    let (alias, auth_file_path, managed_data_dir) = match line.split_once(char::is_whitespace) {
        Some((alias, rest)) if !rest.trim().is_empty() => {
            let rest = rest.trim();
            match rest.split_once(char::is_whitespace) {
                Some((auth_path, managed_dir)) if !managed_dir.trim().is_empty() => (
                    alias.trim(),
                    PathBuf::from(auth_path.trim()),
                    PathBuf::from(managed_dir.trim()),
                ),
                _ => {
                    let auth_file_path = PathBuf::from(rest);
                    let managed_data_dir =
                        default_remote_managed_data_dir(local_managed_data_dir, &auth_file_path);
                    (alias.trim(), auth_file_path, managed_data_dir)
                }
            }
        }
        _ => {
            let auth_file_path = default_remote_auth_path(local_auth_storage_dir);
            let managed_data_dir =
                default_remote_managed_data_dir(local_managed_data_dir, &auth_file_path);
            (line.trim(), auth_file_path, managed_data_dir)
        }
    };

    ManagedHost {
        id: alias.to_string(),
        label: alias.to_string(),
        target: HostTarget::Remote(RemoteHost {
            ssh_alias: alias.to_string(),
            auth_file_path,
            managed_data_dir,
        }),
    }
}

fn default_auth_storage_dir() -> PathBuf {
    if let Ok(value) = env::var("SHAPESHIFTER_CODEX_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let home_dir = dirs::home_dir();
    let data_local_dir = dirs::data_local_dir();
    let candidates = candidate_auth_storage_dirs(home_dir.as_deref(), data_local_dir.as_deref());
    if let Some(existing) = candidates.iter().find(|path| path.exists()) {
        return existing.clone();
    }

    if cfg!(windows) {
        windows_default_storage_dir(home_dir.as_deref(), data_local_dir.as_deref())
    } else {
        unix_default_storage_dir(home_dir.as_deref())
    }
}

fn default_managed_data_dir() -> PathBuf {
    if let Ok(value) = env::var("SHAPESHIFTER_DATA_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shapeshifter")
}

fn candidate_auth_storage_dirs(
    home_dir: Option<&Path>,
    data_local_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(home_dir) = home_dir {
        candidates.push(home_dir.join(".codex"));
        candidates.push(home_dir.join(".local").join("share").join("opencode"));
    }

    if let Some(data_local_dir) = data_local_dir {
        candidates.push(data_local_dir.join("opencode"));
    }

    dedup_paths(candidates)
}

fn windows_default_storage_dir(home_dir: Option<&Path>, data_local_dir: Option<&Path>) -> PathBuf {
    if let Some(home_dir) = home_dir {
        return home_dir.join(".local").join("share").join("opencode");
    }
    if let Some(data_local_dir) = data_local_dir {
        return data_local_dir.join("opencode");
    }
    PathBuf::from(".").join("opencode")
}

fn unix_default_storage_dir(home_dir: Option<&Path>) -> PathBuf {
    home_dir
        .map(|home_dir| home_dir.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".").join(".codex"))
}

fn default_remote_auth_path(local_storage_dir: &Path) -> PathBuf {
    let auth_path = local_storage_dir.join("auth.json");
    if let Some(home_dir) = dirs::home_dir() {
        if let Ok(relative_path) = auth_path.strip_prefix(&home_dir) {
            return relative_path.to_path_buf();
        }
    }
    auth_path
}

fn default_remote_managed_data_dir(
    local_managed_data_dir: &Path,
    remote_auth_file_path: &Path,
) -> PathBuf {
    let auth_parent = remote_auth_file_path
        .parent()
        .unwrap_or(remote_auth_file_path);
    if auth_parent.ends_with(Path::new(".local/share/opencode")) {
        return auth_parent
            .parent()
            .map(|parent| parent.join("shapeshifter"))
            .unwrap_or_else(|| PathBuf::from(".local/share/shapeshifter"));
    }
    if auth_parent.ends_with(Path::new("AppData/Local/opencode")) {
        return auth_parent
            .parent()
            .map(|parent| parent.join("shapeshifter"))
            .unwrap_or_else(|| PathBuf::from("AppData/Local/shapeshifter"));
    }
    local_managed_data_dir.to_path_buf()
}

fn migrate_legacy_accounts_if_needed(
    active_auth_dir: &Path,
    managed_data_dir: &Path,
) -> Result<()> {
    let managed_accounts_dir = managed_data_dir.join("accounts");
    let managed_hosts_path = managed_accounts_dir.join(".hosts");
    let legacy_accounts_dir = active_auth_dir.join("accounts");
    let legacy_hosts_path = legacy_accounts_dir.join(".hosts");

    let managed_is_empty = fs::read_dir(&managed_accounts_dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true);
    if !managed_is_empty {
        return Ok(());
    }

    if !legacy_accounts_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(&managed_accounts_dir)?;
    for entry in fs::read_dir(&legacy_accounts_dir)? {
        let entry = entry?;
        let source = entry.path();
        let destination = managed_accounts_dir.join(entry.file_name());
        if source.is_file() {
            fs::copy(&source, destination)?;
        }
    }
    if !managed_hosts_path.exists() && legacy_hosts_path.exists() {
        fs::copy(legacy_hosts_path, managed_hosts_path)?;
    }
    Ok(())
}

fn dedup_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.iter().any(|existing| existing == &path) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::windows_default_storage_dir;
    use super::{
        candidate_auth_storage_dirs, default_remote_auth_path, default_remote_managed_data_dir,
        migrate_legacy_accounts_if_needed, unix_default_storage_dir,
    };
    use std::fs;
    use std::path::Path;

    #[test]
    fn includes_codex_and_opencode_candidates() {
        let candidates = candidate_auth_storage_dirs(
            Some(Path::new("/home/demo")),
            Some(Path::new("/home/demo/.local/share")),
        );

        assert!(candidates.contains(&Path::new("/home/demo/.codex").to_path_buf()));
        assert!(candidates.contains(&Path::new("/home/demo/.local/share/opencode").to_path_buf()));
    }

    #[test]
    fn derives_remote_auth_path_relative_to_home() {
        let home_dir = dirs::home_dir().unwrap_or_else(|| Path::new("/tmp/home").to_path_buf());
        let storage_dir = home_dir.join(".local").join("share").join("opencode");
        let path = default_remote_auth_path(&storage_dir);

        assert!(path.ends_with(Path::new(".local/share/opencode/auth.json")));
    }

    #[test]
    fn unix_default_prefers_dot_codex() {
        let path = unix_default_storage_dir(Some(Path::new("/home/demo")));
        assert_eq!(path, Path::new("/home/demo/.codex"));
    }

    #[test]
    fn derives_remote_managed_dir_from_opencode_path() {
        let path = default_remote_managed_data_dir(
            Path::new("/home/local/.local/share/shapeshifter"),
            Path::new(".local/share/opencode/auth.json"),
        );
        assert_eq!(path, Path::new(".local/share/shapeshifter"));
    }

    #[test]
    fn migrates_legacy_accounts_when_managed_store_empty() {
        let temp = tempfile::tempdir().unwrap();
        let auth_dir = temp.path().join(".codex");
        let managed_dir = temp.path().join("appdata");
        fs::create_dir_all(auth_dir.join("accounts")).unwrap();
        fs::write(auth_dir.join("accounts/alice.json"), "{}\n").unwrap();
        fs::write(auth_dir.join("accounts/.hosts"), "vd\n").unwrap();

        migrate_legacy_accounts_if_needed(&auth_dir, &managed_dir).unwrap();

        assert!(managed_dir.join("accounts/alice.json").exists());
        assert!(managed_dir.join("accounts/.hosts").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_prefers_xdg_style_opencode_under_home() {
        let home_dir = dirs::home_dir().expect("expected current Windows user home directory");
        let path = windows_default_storage_dir(Some(&home_dir), dirs::data_local_dir().as_deref());
        assert_eq!(path, home_dir.join(".local").join("share").join("opencode"));
    }
}
