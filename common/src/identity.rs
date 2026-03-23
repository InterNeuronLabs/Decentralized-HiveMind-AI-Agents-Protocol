// common/src/identity.rs
// Ed25519 keypair management with zeroize-on-drop guarantees.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use thiserror::Error;
use zeroize::ZeroizeOnDrop;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("invalid hex encoding: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("invalid key bytes: {0}")]
    InvalidKey(#[from] ed25519_dalek::SignatureError),
    #[error("signature verification failed")]
    VerificationFailed,
}

/// Wrapper around `SigningKey` that zeroes memory on drop.
#[derive(ZeroizeOnDrop)]
pub struct NodeSigningKey(pub SigningKey);

impl NodeSigningKey {
    /// Generate a fresh keypair.
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut OsRng))
    }

    /// Load from raw 64-byte secret key bytes.
    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self, IdentityError> {
        Ok(Self(SigningKey::from_keypair_bytes(bytes)?))
    }

    /// Hex-encode the 64-byte keypair (secret + public).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0.to_keypair_bytes())
    }

    /// Load from hex-encoded 64-byte keypair.
    pub fn from_hex(s: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(s)?;
        let arr: [u8; 64] = bytes
            .try_into()
            .map_err(|_| IdentityError::InvalidKey(ed25519_dalek::SignatureError::new()))?;
        Self::from_bytes(&arr)
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.0.verifying_key()
    }

    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.0.verifying_key().as_bytes())
    }

    /// Sign arbitrary bytes, returning hex-encoded signature.
    pub fn sign_hex(&self, msg: &[u8]) -> String {
        hex::encode(self.0.sign(msg).to_bytes())
    }
}

/// Verify a hex-encoded Ed25519 signature against a hex-encoded public key.
pub fn verify_signature(pubkey_hex: &str, msg: &[u8], sig_hex: &str) -> Result<(), IdentityError> {
    let pk_bytes = hex::decode(pubkey_hex)?;
    let pk_arr: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| IdentityError::InvalidKey(ed25519_dalek::SignatureError::new()))?;
    let vk = VerifyingKey::from_bytes(&pk_arr)?;

    let sig_bytes = hex::decode(sig_hex)?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| IdentityError::InvalidKey(ed25519_dalek::SignatureError::new()))?;
    let sig = Signature::from_bytes(&sig_arr);

    vk.verify(msg, &sig)
        .map_err(|_| IdentityError::VerificationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = NodeSigningKey::generate();
        let msg = b"hello cluster";
        let sig_hex = key.sign_hex(msg);
        let pubkey_hex = key.pubkey_hex();
        verify_signature(&pubkey_hex, msg, &sig_hex).expect("should verify");
    }

    #[test]
    fn wrong_message_fails_verification() {
        let key = NodeSigningKey::generate();
        let sig_hex = key.sign_hex(b"correct message");
        verify_signature(&key.pubkey_hex(), b"wrong message", &sig_hex)
            .expect_err("should not verify");
    }

    #[test]
    fn hex_roundtrip() {
        let key = NodeSigningKey::generate();
        let hex = key.to_hex();
        let key2 = NodeSigningKey::from_hex(&hex).expect("should load");
        assert_eq!(key.pubkey_hex(), key2.pubkey_hex());
    }
}
