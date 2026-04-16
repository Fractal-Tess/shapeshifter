use domain::ManagedHost;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub profiles_path: PathBuf,
    pub hosts: Vec<ManagedHost>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            profiles_path: PathBuf::from("profiles.json"),
            hosts: Vec::new(),
        }
    }
}
