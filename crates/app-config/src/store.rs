use crate::AppConfig;
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

#[derive(Default)]
pub struct AppConfigStore;

impl AppConfigStore {
    pub fn new() -> Self {
        Self
    }

    pub fn path(&self) -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("shapeshifter/config.toml")
    }

    pub fn load(&self) -> Result<AppConfig> {
        let path = self.path();
        if !path.exists() {
            return Ok(AppConfig::default());
        }
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }
}
