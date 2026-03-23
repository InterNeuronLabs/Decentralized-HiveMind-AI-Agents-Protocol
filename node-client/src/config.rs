// node-client/src/config.rs

use anyhow::{Context, Result};
use secrecy::Secret;
use std::env;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NodeConfig {
    /// Base URL of the orchestrator (e.g. "https://orchestrator.example.com")
    pub orchestrator_url: String,
    /// Path to persist the ed25519 wallet key. Default: ~/.node-client/wallet.key
    pub wallet_path: String,
    /// Path to the cluster CA cert (PEM) for mTLS — optional in dev mode.
    pub ca_cert_path: Option<String>,
    /// Path to this node's client cert (PEM) issued by the CA — optional in dev mode.
    pub node_cert_path: Option<String>,
    /// Path to this node's client key (PEM) — optional in dev mode.
    pub node_key_path: Option<String>,
    /// Which model to serve (e.g. llama3-8b)
    pub model_id: String,
    /// Path to the GGUF model file (used when llama-direct feature is on)
    pub model_path: String,
    /// Address of a local llama-server if using subprocess mode. E.g. http://127.0.0.1:8080
    pub llama_server_url: String,
    /// Redis stream URL for picking up tasks (required only for `start`)
    pub redis_url: Option<Secret<String>>,
    /// Node tier: nano | edge | pro | cluster
    pub node_tier: String,
    /// Optional Solana wallet pubkey for receiving credits on-chain
    pub solana_wallet: Option<String>,
}

impl NodeConfig {
    pub fn from_env() -> Result<Self> {
        let home = env::var("HOME").unwrap_or_else(|_| "/root".into());
        Ok(Self {
            orchestrator_url: env::var("ORCHESTRATOR_URL")
                .context("ORCHESTRATOR_URL is required")?,
            wallet_path: env::var("WALLET_PATH")
                .unwrap_or_else(|_| format!("{home}/.node-client/wallet.key")),
            ca_cert_path: env::var("CA_CERT_PATH").ok(),
            node_cert_path: env::var("NODE_CERT_PATH").ok(),
            node_key_path: env::var("NODE_KEY_PATH").ok(),
            model_id: env::var("MODEL_ID").unwrap_or_else(|_| "llama3-8b".into()),
            model_path: env::var("MODEL_PATH").unwrap_or_default(),
            llama_server_url: env::var("LLAMA_SERVER_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            redis_url: env::var("REDIS_URL").ok().map(Secret::new),
            node_tier: env::var("NODE_TIER").unwrap_or_else(|_| "edge".into()),
            solana_wallet: env::var("SOLANA_WALLET").ok(),
        })
    }
}
