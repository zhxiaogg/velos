//! Bootstrap-token worker registration and credential authentication.
//!
//! Fail closed throughout (Principle #6): unknown/expired/revoked tokens and
//! missing credentials are rejected with no side effects. Secrets live in a
//! [`Secret`] newtype that never `Display`s or logs its contents (Principle #1);
//! only salted-free SHA-256 hashes are persisted (protocol ≠ storage,
//! Principle #7 — the wire credential is distinct from the stored hash).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use velos_store::{Store, StoreError, StoredObject};

/// A credential secret. Constructable and hashable, but it never reveals itself
/// through `Debug`/`Display`/`Serialize`; callers must opt in via [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    /// Generate a fresh random secret.
    pub fn generate() -> Self {
        Secret(Uuid::new_v4().simple().to_string())
    }

    /// Explicitly read the secret (e.g. to return it once to the client).
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// SHA-256 hex digest, the only form ever persisted.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.0.as_bytes());
        hex::encode(h.finalize())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid token")]
    Invalid,
    #[error("token expired")]
    Expired,
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// The result of authenticating a bearer credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    Worker(String),
}

/// A freshly minted bootstrap token. The `secret` is shown exactly once.
#[derive(Debug)]
pub struct BootstrapToken {
    pub token_id: String,
    pub secret: Secret,
    pub expires_at: DateTime<Utc>,
}

/// Auth operations the apiserver depends on. A trait so the implementation
/// (SQLite-backed today) is swappable behind a deep-module seam.
pub trait AuthService: Send + Sync {
    /// Mint a bootstrap token valid for `ttl_secs`; persists only its hash.
    fn mint_bootstrap_token(&self, ttl_secs: i64) -> Result<BootstrapToken, AuthError>;
    /// Verify a presented `id.secret` bootstrap token, failing closed.
    fn verify_bootstrap(&self, presented: &str) -> Result<(), AuthError>;
    /// Issue a long-lived worker credential (`worker.secret`); persists its hash.
    fn issue_credential(&self, worker: &str) -> Result<String, AuthError>;
    /// Authenticate a presented `worker.secret` credential; `None` = rejected.
    fn authenticate(&self, presented: &str) -> Option<Identity>;
    /// Revoke a worker's credential (tombstone); its next call fails closed.
    fn revoke_credential(&self, worker: &str) -> Result<(), AuthError>;
}

/// Persists hashed tokens/credentials in the `Store` under kinds that the REST
/// router does not expose, keeping secrets off the public API surface.
pub struct StoreAuthenticator {
    store: Arc<dyn Store>,
}

const KIND_TOKEN: &str = "BootstrapToken";
const KIND_CRED: &str = "WorkerCredential";

impl StoreAuthenticator {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    fn write(&self, kind: &str, name: &str, document: serde_json::Value) -> Result<(), AuthError> {
        let rv = self.store.next_resource_version()?;
        self.store.put(&StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: rv,
            node_name: None,
            labels: Default::default(),
            document,
        })?;
        Ok(())
    }
}

fn split_credential(presented: &str) -> Option<(&str, Secret)> {
    let (id, secret) = presented.split_once('.')?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some((id, Secret::new(secret)))
}

impl AuthService for StoreAuthenticator {
    fn mint_bootstrap_token(&self, ttl_secs: i64) -> Result<BootstrapToken, AuthError> {
        let token_id = Uuid::new_v4().simple().to_string();
        let secret = Secret::generate();
        let expires_at = Utc::now() + Duration::seconds(ttl_secs);
        self.write(
            KIND_TOKEN,
            &token_id,
            serde_json::json!({
                "secretHash": secret.hash(),
                "expiresAt": expires_at.to_rfc3339(),
            }),
        )?;
        Ok(BootstrapToken {
            token_id,
            secret,
            expires_at,
        })
    }

    fn verify_bootstrap(&self, presented: &str) -> Result<(), AuthError> {
        let (id, secret) = split_credential(presented).ok_or(AuthError::Invalid)?;
        let rec = self.store.get(KIND_TOKEN, id)?.ok_or(AuthError::Invalid)?;
        let expires = rec
            .document
            .get("expiresAt")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .ok_or(AuthError::Invalid)?
            .with_timezone(&Utc);
        if Utc::now() >= expires {
            return Err(AuthError::Expired);
        }
        let stored = rec
            .document
            .get("secretHash")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::Invalid)?;
        if stored == secret.hash() {
            Ok(())
        } else {
            Err(AuthError::Invalid)
        }
    }

    fn issue_credential(&self, worker: &str) -> Result<String, AuthError> {
        let secret = Secret::generate();
        self.write(
            KIND_CRED,
            worker,
            serde_json::json!({ "tokenHash": secret.hash() }),
        )?;
        Ok(format!("{worker}.{}", secret.expose()))
    }

    fn authenticate(&self, presented: &str) -> Option<Identity> {
        let (worker, secret) = split_credential(presented)?;
        let rec = self.store.get(KIND_CRED, worker).ok()??;
        let stored = rec.document.get("tokenHash").and_then(|v| v.as_str())?;
        if stored == secret.hash() {
            Some(Identity::Worker(worker.to_string()))
        } else {
            None
        }
    }

    fn revoke_credential(&self, worker: &str) -> Result<(), AuthError> {
        self.store.delete(KIND_CRED, worker)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use velos_store::SqliteStore;

    fn auth() -> StoreAuthenticator {
        StoreAuthenticator::new(Arc::new(SqliteStore::in_memory().unwrap()))
    }

    #[test]
    fn secret_never_leaks_through_debug() {
        let s = Secret::new("super-secret");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert!(!format!("{s:?}").contains("super-secret"));
    }

    #[test]
    fn bootstrap_token_round_trip_then_expiry() {
        let a = auth();
        let tok = a.mint_bootstrap_token(60).unwrap();
        let presented = format!("{}.{}", tok.token_id, tok.secret.expose());
        assert!(a.verify_bootstrap(&presented).is_ok());

        // wrong secret fails closed
        let bad = format!("{}.{}", tok.token_id, "nope");
        assert!(matches!(a.verify_bootstrap(&bad), Err(AuthError::Invalid)));

        // expired token fails closed
        let expired = a.mint_bootstrap_token(-1).unwrap();
        let presented = format!("{}.{}", expired.token_id, expired.secret.expose());
        assert!(matches!(
            a.verify_bootstrap(&presented),
            Err(AuthError::Expired)
        ));
    }

    #[test]
    fn unknown_token_is_rejected() {
        let a = auth();
        assert!(matches!(
            a.verify_bootstrap("ghost.secret"),
            Err(AuthError::Invalid)
        ));
        assert!(a.verify_bootstrap("malformed").is_err());
    }

    #[test]
    fn credential_authenticates_then_revokes() {
        let a = auth();
        let cred = a.issue_credential("w1").unwrap();
        assert_eq!(a.authenticate(&cred), Some(Identity::Worker("w1".into())));

        // tampered secret rejected
        assert_eq!(a.authenticate("w1.wrong"), None);

        a.revoke_credential("w1").unwrap();
        assert_eq!(a.authenticate(&cred), None);
    }
}
