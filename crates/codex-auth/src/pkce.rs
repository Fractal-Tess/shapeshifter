use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct PkceVerifier {
    pub state: String,
    pub code_verifier: String,
    pub code_challenge: String,
}

impl PkceVerifier {
    pub fn generate() -> Self {
        let verifier = uuid::Uuid::new_v4().simple().to_string();
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            state: uuid::Uuid::new_v4().to_string(),
            code_verifier: verifier,
            code_challenge: URL_SAFE_NO_PAD.encode(digest),
        }
    }
}
