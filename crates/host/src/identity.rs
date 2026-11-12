use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_rustls::rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};

#[derive(Serialize, Deserialize)]
pub struct Pairing {
    pub endpoint: String,
    pub token: String,
    pub certificate_sha256: String,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

pub fn initialize(directory: &Path, endpoint: &str) -> Result<()> {
    if directory.join("pairing.json").exists() {
        bail!("Identity already exists; refusing to overwrite it")
    }
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let identity =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
    let mut random = [0; 32];
    rand::thread_rng().fill_bytes(&mut random);
    let pairing = Pairing {
        endpoint: endpoint.into(),
        token: hex(&random),
        certificate_sha256: hex(&Sha256::digest(identity.cert.der())),
    };
    private_file(
        &directory.join("key.der"),
        &identity.key_pair.serialize_der(),
    )?;
    private_file(&directory.join("certificate.der"), identity.cert.der())?;
    private_file(
        &directory.join("pairing.json"),
        &serde_json::to_vec_pretty(&pairing)?,
    )?;
    Ok(())
}

pub fn load(directory: &Path) -> Result<(Pairing, Arc<ServerConfig>)> {
    let pairing: Pairing = serde_json::from_slice(
        &fs::read(directory.join("pairing.json")).context("Run gui-use-host init first")?,
    )?;
    let certificate = fs::read(directory.join("certificate.der"))?;
    anyhow::ensure!(
        pairing.certificate_sha256 == hex(&Sha256::digest(&certificate)),
        "Certificate fingerprint does not match pairing configuration"
    );
    anyhow::ensure!(pairing.token.len() >= 32, "Invalid pairing token");
    let key = fs::read(directory.join("key.der"))?;
    let provider = tokio_rustls::rustls::crypto::ring::default_provider();
    let config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
        )?;
    Ok((pairing, Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_are_private_pinned_and_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        initialize(temp.path(), "wss://localhost:8765").unwrap();
        let (pairing, _) = load(temp.path()).unwrap();
        assert_eq!(pairing.token.len(), 64);
        assert_eq!(pairing.certificate_sha256.len(), 64);
        assert_eq!(
            fs::metadata(temp.path().join("key.der"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(initialize(temp.path(), "wss://other").is_err());
    }
}
