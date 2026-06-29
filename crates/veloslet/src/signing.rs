//! Stable self-signed code signing for the worker app bundle.
//!
//! macOS pins an app's Local Network privacy grant to its code-signature
//! Designated Requirement (DR). Ad-hoc signing (`codesign --sign -`) makes the
//! DR depend on the **cdhash**, which changes on every build — so the grant
//! breaks on every reinstall and there is no supported way to reset it.
//!
//! Signing with a *persistent* self-signed certificate instead makes the DR
//! `identifier "<bundle-id>" and certificate root = H"<cert>"` — a function of
//! the (stable) bundle identifier and the (reused) certificate, independent of
//! the cdhash. The grant therefore survives every reinstall.
//!
//! The certificate is generated once and persisted under the codesign dir; only
//! the private key is sensitive (written `0600`). Signing happens in-process via
//! the `apple-codesign` crate — no keychain, no `codesign` subprocess, no GUI
//! authorization prompt.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use apple_codesign::{
    CertificateProfile, SettingsScope, SigningSettings, UnifiedSigner,
    create_self_signed_code_signing_certificate,
};
use x509_certificate::{CapturedX509Certificate, InMemorySigningKeyPair, KeyAlgorithm};

/// Persisted certificate (PEM) within the codesign dir.
const CERT_FILE: &str = "signing.crt";
/// Persisted private key (PKCS#8 DER, `0600`) within the codesign dir.
const KEY_FILE: &str = "signing.key";

/// A loaded signing identity: a self-signed certificate and its private key.
pub struct SigningIdentity {
    cert: CapturedX509Certificate,
    key: InMemorySigningKeyPair,
}

/// Load the persistent signing identity from `dir`, generating and persisting a
/// new one on first use.
///
/// Idempotent: once created, the same identity is reused on every install so the
/// bundle's Designated Requirement — and thus any macOS privacy grant keyed to
/// it — stays constant across reinstalls.
pub fn ensure_identity(dir: &Path) -> Result<SigningIdentity> {
    let cert_path = dir.join(CERT_FILE);
    let key_path = dir.join(KEY_FILE);

    if !cert_path.exists() || !key_path.exists() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        // RSA-2048 matches Apple's own signing certs, so macOS accepts the
        // resulting signature without question. The cert is long-lived (10y);
        // its identity, not its validity window, is what we depend on.
        let (cert, key) = create_self_signed_code_signing_certificate(
            KeyAlgorithm::Rsa,
            CertificateProfile::AppleDevelopment,
            "",
            "Velos Worker Code Signing",
            "XX",
            chrono::Duration::days(3650),
        )
        .map_err(|e| anyhow::anyhow!("generating signing certificate: {e}"))?;
        write_private(&key_path, key.to_pkcs8_one_asymmetric_key_der().as_slice())?;
        std::fs::write(&cert_path, cert.encode_pem())
            .with_context(|| format!("writing {}", cert_path.display()))?;
    }

    let cert = CapturedX509Certificate::from_pem(
        std::fs::read(&cert_path).with_context(|| format!("reading {}", cert_path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("loading signing certificate: {e}"))?;
    let key = InMemorySigningKeyPair::from_pkcs8_der(
        std::fs::read(&key_path).with_context(|| format!("reading {}", key_path.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("loading signing key: {e}"))?;
    Ok(SigningIdentity { cert, key })
}

/// Sign an `.app` bundle in place with `identity`, forcing `identifier` as the
/// signing identifier. Recursively seals the nested Mach-O executable and the
/// bundle. Consumes `identity` (its key is borrowed by the signer for the call).
pub fn sign_bundle(bundle: &Path, identifier: &str, identity: SigningIdentity) -> Result<()> {
    let SigningIdentity { cert, key } = identity;
    let mut settings = SigningSettings::default();
    settings.set_signing_key(&key, cert);
    settings.set_binary_identifier(SettingsScope::Main, identifier);
    UnifiedSigner::new(settings)
        .sign_path_in_place(bundle)
        .map_err(|e| anyhow::anyhow!("signing {}: {e}", bundle.display()))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 600 {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[cfg_attr(test, allow(clippy::unwrap_used))]
mod tests {
    use super::*;

    #[test]
    fn ensure_identity_persists_and_reuses() {
        let tmp = std::env::temp_dir().join(format!("velos-signing-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        // First call generates + persists.
        let first = ensure_identity(&tmp).unwrap();
        let cert_pem = std::fs::read(tmp.join(CERT_FILE)).unwrap();
        assert!(std::fs::metadata(tmp.join(KEY_FILE)).is_ok());
        // Private key is 0600.
        let mode = std::fs::metadata(tmp.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        // Second call reuses the same cert (identity stays stable).
        let second = ensure_identity(&tmp).unwrap();
        assert_eq!(std::fs::read(tmp.join(CERT_FILE)).unwrap(), cert_pem);
        assert_eq!(first.cert.encode_pem(), second.cert.encode_pem());

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
