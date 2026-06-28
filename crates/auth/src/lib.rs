//! Bootstrap-token worker registration and credential authentication.
//!
//! Fail closed throughout (Principle #6): unknown/expired/revoked tokens and
//! missing credentials are rejected with no side effects. Secrets live in a
//! [`Secret`] newtype that never `Display`s or logs its contents (Principle #1);
//! only salted-free SHA-256 hashes are persisted (protocol ≠ storage,
//! Principle #7 — the wire credential is distinct from the stored hash).

use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use velos_store::{Selector, Store, StoreError, StoredObject};

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
    #[error("already initialized")]
    AlreadyInitialized,
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// argon2id PHC-string hash of a password (salt embedded). Used for the human
/// admin password only; high-entropy random tokens keep the SHA-256 path.
fn hash_password(password: &Secret) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.expose().as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::Invalid)
}

/// Verify a password against a stored PHC string; `false` on any parse/verify error.
fn verify_password_hash(hash: &str, password: &Secret) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.expose().as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Extract a string field from a stored document, or empty string if absent.
fn str_field(doc: &serde_json::Value, key: &str) -> String {
    doc.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// The result of authenticating a bearer credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    Worker(String),
    Admin,
}

/// A freshly minted bootstrap token. The `secret` is shown exactly once.
#[derive(Debug)]
pub struct BootstrapToken {
    pub token_id: String,
    pub secret: Secret,
    pub expires_at: DateTime<Utc>,
}

/// Auth operations the server depends on. A trait so the implementation
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

    /// Whether the admin account has been set up. Until it is, the server
    /// reaches only `status`/`setup` (the initialization gate).
    fn is_initialized(&self) -> Result<bool, AuthError>;
    /// Create the single admin account; fails closed if already initialized.
    fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError>;
    /// Verify an admin `username`+`password`, failing closed.
    fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError>;
    /// Mint a short-lived admin session token (the UI's bearer).
    fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError>;
    /// Mint a long-lived, named admin CLI token; the secret is returned once.
    fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError>;
    /// List admin-token metadata (never the secrets).
    fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError>;
    /// Revoke an admin token by id; its next call fails closed.
    fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError>;
}

/// A freshly minted admin token; `token` (= `id.secret`) is shown exactly once.
#[derive(Debug)]
pub struct MintedToken {
    pub id: String,
    pub token: String,
}

/// Listable metadata for an admin token. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminTokenInfo {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub created_at: String,
    pub expires_at: String,
}

/// Resolves a presented bearer credential to an [`Identity`], or `None` (fail
/// closed). The server depends on this so an external OIDC verifier can be
/// substituted later without touching any endpoint (deep-module seam).
pub trait TokenVerifier: Send + Sync {
    fn verify(&self, presented: &str) -> Option<Identity>;
}

impl TokenVerifier for StoreAuthenticator {
    fn verify(&self, presented: &str) -> Option<Identity> {
        self.authenticate(presented)
    }
}

/// Persists hashed tokens/credentials in the `Store` under kinds that the REST
/// router does not expose, keeping secrets off the public API surface.
pub struct StoreAuthenticator {
    store: Arc<dyn Store>,
}

const KIND_TOKEN: &str = "BootstrapToken";
const KIND_CRED: &str = "WorkerCredential";
const KIND_ADMIN: &str = "AdminAccount";
const KIND_ADMIN_TOKEN: &str = "AdminToken";

/// The admin account lives under a single well-known row (single-admin scope).
const ADMIN_ROW: &str = "admin";

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

    /// Mint and persist an admin token (session or CLI), returning the one-time
    /// `id.secret`. Only the hash is stored.
    fn mint_admin_token(
        &self,
        label: &str,
        kind: &str,
        ttl_secs: i64,
    ) -> Result<MintedToken, AuthError> {
        let id = Uuid::new_v4().simple().to_string();
        let secret = Secret::generate();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl_secs);
        self.write(
            KIND_ADMIN_TOKEN,
            &id,
            serde_json::json!({
                "secretHash": secret.hash(),
                "label": label,
                "kind": kind,
                "createdAt": now.to_rfc3339(),
                "expiresAt": expires_at.to_rfc3339(),
            }),
        )?;
        Ok(MintedToken {
            token: format!("{id}.{}", secret.expose()),
            id,
        })
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
        let (id, secret) = split_credential(presented)?;

        // Admin token (session or CLI): unexpired + hash match -> Admin.
        if let Ok(Some(rec)) = self.store.get(KIND_ADMIN_TOKEN, id) {
            let stored = rec.document.get("secretHash").and_then(|v| v.as_str())?;
            let expires = rec
                .document
                .get("expiresAt")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?
                .with_timezone(&Utc);
            if stored == secret.hash() && Utc::now() < expires {
                return Some(Identity::Admin);
            }
            return None;
        }

        // Otherwise a worker credential.
        let rec = self.store.get(KIND_CRED, id).ok()??;
        let stored = rec.document.get("tokenHash").and_then(|v| v.as_str())?;
        if stored == secret.hash() {
            Some(Identity::Worker(id.to_string()))
        } else {
            None
        }
    }

    fn revoke_credential(&self, worker: &str) -> Result<(), AuthError> {
        self.store.delete(KIND_CRED, worker)?;
        Ok(())
    }

    fn is_initialized(&self) -> Result<bool, AuthError> {
        Ok(self.store.get(KIND_ADMIN, ADMIN_ROW)?.is_some())
    }

    fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError> {
        if username.is_empty() {
            return Err(AuthError::Invalid);
        }
        if self.is_initialized()? {
            return Err(AuthError::AlreadyInitialized);
        }
        let hash = hash_password(password)?;
        self.write(
            KIND_ADMIN,
            ADMIN_ROW,
            serde_json::json!({ "username": username, "passwordHash": hash }),
        )
    }

    fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError> {
        let rec = self
            .store
            .get(KIND_ADMIN, ADMIN_ROW)?
            .ok_or(AuthError::Invalid)?;
        if rec.document.get("username").and_then(|v| v.as_str()) != Some(username) {
            return Err(AuthError::Invalid);
        }
        let hash = rec
            .document
            .get("passwordHash")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::Invalid)?;
        if verify_password_hash(hash, password) {
            Ok(())
        } else {
            Err(AuthError::Invalid)
        }
    }

    fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError> {
        Ok(self.mint_admin_token("session", "session", ttl_secs)?.token)
    }

    fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError> {
        self.mint_admin_token(label, "cli", ttl_secs)
    }

    fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError> {
        let objs = self.store.list(KIND_ADMIN_TOKEN, &Selector::default())?;
        Ok(objs
            .into_iter()
            .map(|o| AdminTokenInfo {
                id: o.name,
                label: str_field(&o.document, "label"),
                kind: str_field(&o.document, "kind"),
                created_at: str_field(&o.document, "createdAt"),
                expires_at: str_field(&o.document, "expiresAt"),
            })
            .collect())
    }

    fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError> {
        self.store.delete(KIND_ADMIN_TOKEN, id)?;
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

    #[test]
    fn admin_setup_is_single_shot_and_password_round_trips() {
        let a = auth();
        assert!(!a.is_initialized().unwrap());

        a.setup_admin("admin", &Secret::new("hunter2")).unwrap();
        assert!(a.is_initialized().unwrap());

        // correct password verifies
        assert!(a.verify_password("admin", &Secret::new("hunter2")).is_ok());
        // wrong password fails closed
        assert!(a.verify_password("admin", &Secret::new("nope")).is_err());
        // wrong username fails closed
        assert!(a.verify_password("root", &Secret::new("hunter2")).is_err());

        // second setup is rejected
        assert!(matches!(
            a.setup_admin("admin2", &Secret::new("x")),
            Err(AuthError::AlreadyInitialized)
        ));
    }

    #[test]
    fn password_hash_is_salted_and_not_plaintext() {
        let h1 = hash_password(&Secret::new("same")).unwrap();
        let h2 = hash_password(&Secret::new("same")).unwrap();
        assert_ne!(h1, h2, "salts differ");
        assert!(!h1.contains("same"));
        assert!(verify_password_hash(&h1, &Secret::new("same")));
        assert!(!verify_password_hash(&h1, &Secret::new("different")));
    }

    #[test]
    fn admin_tokens_authenticate_expire_and_revoke() {
        let a = auth();
        let session = a.mint_admin_session(60).unwrap();
        assert_eq!(a.authenticate(&session), Some(Identity::Admin));

        let cli = a.mint_cli_token("laptop", 3600).unwrap();
        assert_eq!(a.authenticate(&cli.token), Some(Identity::Admin));

        // listing shows the cli token's metadata, never the secret
        let listed = a.list_admin_tokens().unwrap();
        assert!(listed.iter().any(|t| t.id == cli.id && t.label == "laptop"));

        // revoke -> fails closed
        a.revoke_admin_token(&cli.id).unwrap();
        assert_eq!(a.authenticate(&cli.token), None);

        // expired token fails closed
        let dead = a.mint_cli_token("old", -1).unwrap();
        assert_eq!(a.authenticate(&dead.token), None);

        // a worker credential still resolves as a worker, not admin
        let cred = a.issue_credential("w1").unwrap();
        assert_eq!(a.authenticate(&cred), Some(Identity::Worker("w1".into())));
    }

    #[test]
    fn token_verifier_delegates_to_authenticate() {
        let a = auth();
        let session = a.mint_admin_session(60).unwrap();
        let v: &dyn TokenVerifier = &a;
        assert_eq!(v.verify(&session), Some(Identity::Admin));
        assert_eq!(v.verify("garbage"), None);
    }
}
