use crate::AuthFile;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub label: String,
    pub source_path: PathBuf,
    pub auth_file: AuthFile,
}
