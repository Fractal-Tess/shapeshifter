use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedHost {
    pub id: String,
    pub label: String,
    pub target: HostTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostTarget {
    Local { auth_file_path: PathBuf },
    Remote(RemoteHost),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHost {
    pub ssh_alias: String,
    pub auth_file_path: PathBuf,
    pub managed_data_dir: PathBuf,
}
