// orchestrator/src/config.rs
// All configuration loaded from environment variables only.
// Secrets are wrapped in secrecy::Secret so they cannot be accidentally logged.

use secrecy::Secret;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AppConfig {
    /// postgres://user:password@host:5432/cluster
    pub database_url: Secret<String>,
    /// redis://host:6379
    pub redis_url: Secret<String>,
    /// Hex-encoded 64-byte Ed25519 keypair (secret + public).
    pub orchestrator_signing_key: Secret<String>,
    /// Hex-encoded 64-byte Ed25519 admin keypair.
    pub admin_signing_key: Secret<String>,
    /// Path on disk to the mTLS CA private key PEM.
    pub orchestrator_ca_key_path: String,
    /// Solana RPC endpoint.
    pub solana_rpc_url: String,
    /// Deployed Anchor program ID (set after first deploy).
    pub cluster_token_program_id: Option<String>,
    /// TCP port the orchestrator listens on.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Environment: "development" | "staging" | "production"
    #[serde(default = "default_env")]
    pub app_env: String,
}

fn default_port() -> u16 {
    8080
}
fn default_env() -> String {
    "development".into()
}

impl AppConfig {
    pub fn from_env() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::Environment::default().separator("__"))
            .build()?
            .try_deserialize()
    }
}
