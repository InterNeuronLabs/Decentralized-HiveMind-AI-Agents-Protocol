// node-client/src/wallet.rs
//
// Ed25519 keypair persisted at `~/.node-client/wallet.key` (chmod 600).
// Also stores CreditReceipts locally and verifies their signatures before persisting.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use common::identity::{verify_signature, NodeSigningKey};
use common::types::CreditReceipt;

// ---------------------------------------------------------------------------
// Key management
// ---------------------------------------------------------------------------

/// Load or create the node signing key at `wallet_path`.
pub fn load_or_create_key(wallet_path: &str) -> Result<NodeSigningKey> {
    let path = PathBuf::from(wallet_path);

    if path.exists() {
        let hex = fs::read_to_string(&path).context("failed to read wallet key file")?;
        NodeSigningKey::from_hex(hex.trim()).map_err(|e| anyhow::anyhow!("invalid wallet key: {e}"))
    } else {
        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("failed to create wallet directory")?;
        }

        let key = NodeSigningKey::generate();
        let hex = key.to_hex();

        fs::write(&path, &hex).context("failed to write wallet key")?;

        // Restrict to owner read/write only (0600)
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .context("failed to set wallet key permissions")?;

        tracing::info!(pubkey = %key.pubkey_hex(), path = %wallet_path, "created new wallet key");
        Ok(key)
    }
}

// ---------------------------------------------------------------------------
// Receipt store
// ---------------------------------------------------------------------------

/// Directory for storing CreditReceipts as JSON files.
pub struct ReceiptStore {
    dir: PathBuf,
}

impl ReceiptStore {
    pub fn new(wallet_dir: &Path) -> Result<Self> {
        let dir = wallet_dir.join("receipts");
        fs::create_dir_all(&dir).context("failed to create receipts directory")?;
        Ok(Self { dir })
    }

    /// Persist a receipt after verifying the orchestrator's signature.
    #[allow(dead_code)]
    pub fn save(&self, receipt: &CreditReceipt, orchestrator_pubkey_hex: &str) -> Result<()> {
        // Verify signature before storing
        let msg = receipt.signable_bytes();
        verify_signature(orchestrator_pubkey_hex, &msg, &receipt.orchestrator_sig_hex)
            .map_err(|e| anyhow::anyhow!("receipt signature invalid: {e}"))?;

        let filename = format!("{}.json", receipt.nonce_hex);
        let path = self.dir.join(filename);
        let json = serde_json::to_string_pretty(receipt)?;
        fs::write(&path, json).context("failed to write receipt")?;
        Ok(())
    }

    /// List all saved receipts.
    pub fn list_all(&self) -> Result<Vec<CreditReceipt>> {
        let mut receipts = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                let data = fs::read_to_string(entry.path())?;
                if let Ok(r) = serde_json::from_str::<CreditReceipt>(&data) {
                    receipts.push(r);
                }
            }
        }
        // Sort by issued_at ascending
        receipts.sort_by_key(|r| r.issued_at);
        Ok(receipts)
    }

    /// Total credits across all stored receipts.
    pub fn total_credits(&self) -> Result<f64> {
        Ok(self.list_all()?.iter().map(|r| r.credits).sum())
    }
}
