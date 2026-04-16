use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::io::Write;

const GITHUB_REPO: &str = "Fractal-Tess/shapeshifter";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub download_url: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub fn current_version() -> &'static str {
    CURRENT_VERSION
}

pub fn check_for_update() -> Result<Option<ReleaseInfo>> {
    let url = format!(
        "https://api.github.com/repos/{GITHUB_REPO}/releases/latest"
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent("shapeshifter-tui")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client.get(&url).send()?;
    if !resp.status().is_success() {
        bail!("GitHub API returned {}", resp.status());
    }

    let release: GhRelease = resp.json()?;

    // Compare versions: strip 'v' prefix from tag
    let remote_ver = release.tag_name.trim_start_matches('v');
    if !is_newer(remote_ver, CURRENT_VERSION) {
        return Ok(None);
    }

    let asset_name = tui_asset_name();
    let download_url = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .map(|a| a.browser_download_url.clone());

    Ok(Some(ReleaseInfo {
        tag: release.tag_name,
        name: release.name.unwrap_or_default(),
        download_url,
    }))
}

pub fn download_and_replace(release: &ReleaseInfo) -> Result<()> {
    let url = release
        .download_url
        .as_ref()
        .context("no download URL for this platform")?;

    let client = reqwest::blocking::Client::builder()
        .user_agent("shapeshifter-tui")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        bail!("download failed: HTTP {}", resp.status());
    }

    let bytes = resp.bytes()?;
    let current_exe = std::env::current_exe().context("cannot determine current executable path")?;

    // Write to a temp file next to the current binary, then atomic rename
    let tmp_path = current_exe.with_extension("update-tmp");
    {
        let mut f = fs::File::create(&tmp_path)
            .context("failed to create temp file for update")?;
        f.write_all(&bytes)?;
    }

    // Set executable permission on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755))?;
    }

    // Rename old binary as backup, then move new one in place
    let backup_path = current_exe.with_extension("old");
    let _ = fs::remove_file(&backup_path);
    fs::rename(&current_exe, &backup_path)
        .context("failed to back up current binary")?;
    if let Err(e) = fs::rename(&tmp_path, &current_exe) {
        // Restore backup on failure
        let _ = fs::rename(&backup_path, &current_exe);
        return Err(e).context("failed to replace binary with update");
    }
    let _ = fs::remove_file(&backup_path);

    Ok(())
}

fn tui_asset_name() -> String {
    if cfg!(target_os = "windows") {
        "shapeshifter-windows-x86_64-tui.exe".to_string()
    } else {
        "shapeshifter-linux-x86_64-tui".to_string()
    }
}

fn is_newer(remote: &str, current: &str) -> bool {
    // Try semver comparison, fall back to string comparison
    let parse = |s: &str| -> Option<(u64, u64, u64)> {
        // Handle versions like "0.1.0-abc123" by taking just the semver part
        let clean = s.split('-').next().unwrap_or(s);
        let parts: Vec<&str> = clean.split('.').collect();
        match parts.len() {
            1 => Some((parts[0].parse().ok()?, 0, 0)),
            2 => Some((parts[0].parse().ok()?, parts[1].parse().ok()?, 0)),
            3 => Some((
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            )),
            _ => None,
        }
    };

    match (parse(remote), parse(current)) {
        (Some(r), Some(c)) => r > c,
        _ => remote != current,
    }
}
