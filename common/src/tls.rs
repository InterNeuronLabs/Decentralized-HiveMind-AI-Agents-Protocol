// common/src/tls.rs
// Self-signed CA + per-node certificate issuance for mTLS.
// Orchestrator generates a CA at startup; nodes receive a signed cert at registration.

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("certificate generation failed: {0}")]
    Rcgen(#[from] rcgen::Error),
}

/// A CA keypair + self-signed certificate used to sign node certs.
pub struct ClusterCa {
    cert: rcgen::Certificate,
    key_pair: KeyPair,
}

impl ClusterCa {
    /// Generate a new self-signed CA.
    pub fn generate() -> Result<Self, TlsError> {
        let key_pair = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, "Cluster CA");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "os-project");
        let cert = params.self_signed(&key_pair)?;
        Ok(Self { cert, key_pair })
    }

    /// PEM-encoded CA certificate (safe to distribute publicly).
    pub fn cert_pem(&self) -> String {
        self.cert.pem()
    }

    /// PEM-encoded CA private key (keep in `private/keys/orchestrator-ca.key`).
    pub fn key_pem(&self) -> String {
        self.key_pair.serialize_pem()
    }

    /// Issue a signed certificate for a node identified by `node_id`.
    pub fn issue_node_cert(&self, node_id: &str) -> Result<IssuedCert, TlsError> {
        let node_key_pair = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        let san = format!("{node_id}.node.cluster");
        let san_ia5: rcgen::Ia5String = san.as_str().try_into()?;
        params.subject_alt_names = vec![SanType::DnsName(san_ia5)];
        params.distinguished_name.push(DnType::CommonName, node_id);
        let node_cert = params.signed_by(&node_key_pair, &self.cert, &self.key_pair)?;
        let cert_pem = node_cert.pem();
        let key_pem = node_key_pair.serialize_pem();
        Ok(IssuedCert { cert_pem, key_pem })
    }
}

/// A node's signed TLS certificate + private key (delivered once at registration).
pub struct IssuedCert {
    /// PEM certificate signed by the cluster CA.
    pub cert_pem: String,
    /// PEM private key — node must store this securely (chmod 600).
    pub key_pem: String,
}
