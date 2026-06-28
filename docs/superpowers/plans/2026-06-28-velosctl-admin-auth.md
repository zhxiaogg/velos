# velosctl ↔ server Admin Authentication — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce an admin identity and authenticate the `velosctl ↔ server` path using the GitHub PAT model — first-run admin setup in the web UI, password login, named CLI tokens carried by `velosctl`.

**Architecture:** Extend `velos-auth` with argon2 password handling, a single admin account, and one opaque-token primitive (UI session + CLI token, same mechanism, hashed in the `Store`). The server gains an `Identity::Admin`, setup/login/token endpoints, an initialization gate (only `status`+`setup` reachable while uninitialized), and resolves identity through a `TokenVerifier` seam (OIDC-ready). `velosctl` persists `{server, token}` and resolves both with `flag > env > config` precedence. The web UI replaces its bootstrap-worker hack with setup/login/session-token flows.

**Tech Stack:** Rust 2024 (axum 0.7, thiserror, anyhow, chrono, uuid, sha2, argon2), SQLite-backed `Store`, React + TS + Tailwind + @tanstack/react-query.

## Global Constraints

- Clippy lints are **deny** in production code: `unwrap_used`, `expect_used`, `panic`, `wildcard_enum_match_arm`. Test modules opt out with `#![cfg_attr(test, allow(...))]` at file top (follow the existing `#[allow(clippy::unwrap_used)]` on test modules).
- No wildcard `match` arms on enums — exhaustive matches only.
- Errors: `thiserror` typed errors in library crates (`velos-auth`), `anyhow` in binaries (`velosctl`, server `main.rs`).
- Secrets never `Display`/`Debug`/log their contents — use/extend the `Secret` newtype.
- Protocol ≠ storage: persist opaque docs + index columns via `StoredObject`; do not expose secret-bearing kinds through the REST router.
- Edition 2024; formatting per `rustfmt.toml`. Pre-PR gate is `make check` (`cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`).
- Internal crate deps go through `[workspace.dependencies]` in root `Cargo.toml`.

## Reference: existing types (do not redefine)

From `velos-auth` (`crates/auth/src/lib.rs`):
- `pub struct Secret(String)` with `new`, `generate`, `expose() -> &str`, `hash() -> String` (SHA-256 hex), custom `Debug` printing `Secret(***)`.
- `pub enum AuthError { Invalid, Expired, Store(StoreError) }` (`thiserror`).
- `pub enum Identity { Worker(String) }`.
- `pub trait AuthService: Send + Sync { mint_bootstrap_token, verify_bootstrap, issue_credential, authenticate, revoke_credential }`.
- `pub struct StoreAuthenticator { store: Arc<dyn Store> }`, `StoreAuthenticator::new(store)`.
- private helpers: `fn write(&self, kind, name, document) -> Result<(), AuthError>`, `fn split_credential(presented) -> Option<(&str, Secret)>`, consts `KIND_TOKEN`, `KIND_CRED`.

From `velos-store` (`crates/store/src/lib.rs`):
- `pub trait Store: Send + Sync` with `next_resource_version`, `put(&StoredObject)`, `get(kind,name) -> Result<Option<StoredObject>>`, `delete(kind,name) -> Result<Option<StoredObject>>`, `list(kind,&Selector)`, etc.
- `pub struct StoredObject { kind, name, uid, resource_version, node_name: Option<String>, labels: HashMap<String,String>, document: serde_json::Value }`.
- `SqliteStore::in_memory() -> Result<Self, StoreError>`.

From server (`crates/server/src/lib.rs`):
- `struct AppState { store: Arc<dyn Store>, auth: Option<Arc<dyn AuthService>> }`.
- `enum ApiError { NotFound, BadRequest(String), Unauthorized, Forbidden, Conflict(String), Internal(String) }` (impl `IntoResponse`).
- `fn bearer(&HeaderMap) -> Option<String>`.
- `async fn mint_token`, `async fn register`, `async fn require_auth(State<Arc<dyn AuthService>>, Request, Next)`, `fn named_path`, `fn api_routes() -> Router<AppState>`, `fn app(store)`, `fn app_with_auth(store, auth)`.

---

## Task 1: argon2 password handling + admin account in `velos-auth`

**Files:**
- Modify: `crates/auth/Cargo.toml` (add `argon2`)
- Modify: `crates/auth/src/lib.rs`

**Interfaces:**
- Consumes: `Secret`, `AuthError`, `StoreAuthenticator`, `Store`, the private `write` helper.
- Produces:
  - `const KIND_ADMIN: &str = "AdminAccount";`
  - On `StoreAuthenticator`: `pub fn is_initialized(&self) -> Result<bool, AuthError>`
  - On `StoreAuthenticator`: `pub fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError>` (fails closed with `AuthError::Invalid` if already initialized or username empty)
  - On `StoreAuthenticator`: `pub fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError>`
  - New error variant `AuthError::AlreadyInitialized` (`#[error("already initialized")]`)
  - Free fn `fn hash_password(password: &Secret) -> Result<String, AuthError>` and `fn verify_password_hash(hash: &str, password: &Secret) -> bool` (argon2 PHC string).

- [ ] **Step 1: Add the argon2 dependency**

In `crates/auth/Cargo.toml` under `[dependencies]` add:

```toml
argon2 = "0.5"
```

- [ ] **Step 2: Write the failing tests**

Append to the `mod tests` block in `crates/auth/src/lib.rs`:

```rust
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p velos-auth admin_setup_is_single_shot_and_password_round_trips password_hash_is_salted_and_not_plaintext`
Expected: FAIL — `is_initialized`/`setup_admin`/`hash_password` not found.

- [ ] **Step 4: Implement password hashing helpers**

Add near the top of `crates/auth/src/lib.rs` imports:

```rust
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
```

Add the error variant to `AuthError`:

```rust
    #[error("already initialized")]
    AlreadyInitialized,
```

Add free functions (module level):

```rust
/// argon2id PHC-string hash of a password (salt embedded).
fn hash_password(password: &Secret) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.expose().as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::Invalid)
}

/// Verify a password against a stored PHC string; false on any parse/verify error.
fn verify_password_hash(hash: &str, password: &Secret) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.expose().as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}
```

- [ ] **Step 5: Implement admin-account methods**

Add the kind constant alongside the others:

```rust
const KIND_ADMIN: &str = "AdminAccount";
```

Add an `impl StoreAuthenticator` block (or extend the existing one):

```rust
impl StoreAuthenticator {
    pub fn is_initialized(&self) -> Result<bool, AuthError> {
        Ok(self.store.get(KIND_ADMIN, "admin")?.is_some())
    }

    pub fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError> {
        if username.is_empty() {
            return Err(AuthError::Invalid);
        }
        if self.is_initialized()? {
            return Err(AuthError::AlreadyInitialized);
        }
        let hash = hash_password(password)?;
        self.write(
            KIND_ADMIN,
            "admin",
            serde_json::json!({ "username": username, "passwordHash": hash }),
        )
    }

    pub fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError> {
        let rec = self.store.get(KIND_ADMIN, "admin")?.ok_or(AuthError::Invalid)?;
        let stored_user = rec.document.get("username").and_then(|v| v.as_str());
        if stored_user != Some(username) {
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
}
```

> The admin account is stored under a single well-known name `"admin"` (single-admin scope). The `username` is validated on login but the row key is fixed, so `is_initialized` is a single `get`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p velos-auth`
Expected: PASS (all existing + 2 new tests).

- [ ] **Step 7: Commit**

```bash
git add crates/auth/Cargo.toml crates/auth/src/lib.rs Cargo.lock
git commit -m "feat(auth): argon2-backed single admin account with single-shot setup"
```

---

## Task 2: Admin tokens + `Identity::Admin` in `velos-auth`

**Files:**
- Modify: `crates/auth/src/lib.rs`

**Interfaces:**
- Consumes: `Secret`, `AuthError`, `Store`, `StoreAuthenticator`, `split_credential`, the `write` helper.
- Produces:
  - `Identity::Admin` variant added to the existing enum.
  - `pub struct MintedToken { pub id: String, pub token: String }` (`token` shown once = `id.secret`).
  - `pub struct AdminTokenInfo { pub id: String, pub label: String, pub kind: String, pub created_at: String, pub expires_at: String }`.
  - `const KIND_ADMIN_TOKEN: &str = "AdminToken";`
  - On `StoreAuthenticator`: `pub fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError>` (returns `id.secret`)
  - `pub fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError>`
  - `pub fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError>`
  - `pub fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError>`
  - `authenticate` extended: a valid admin token → `Some(Identity::Admin)`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests`:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p velos-auth admin_tokens_authenticate_expire_and_revoke`
Expected: FAIL — `mint_admin_session` etc. not found; `Identity::Admin` missing.

- [ ] **Step 3: Extend the `Identity` enum**

```rust
pub enum Identity {
    Worker(String),
    Admin,
}
```

- [ ] **Step 4: Add token structs + kind constant**

```rust
const KIND_ADMIN_TOKEN: &str = "AdminToken";

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
```

- [ ] **Step 5: Implement mint/list/revoke + a shared minting helper**

Add to `impl StoreAuthenticator`:

```rust
fn mint_admin_token(&self, label: &str, kind: &str, ttl_secs: i64) -> Result<MintedToken, AuthError> {
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
    Ok(MintedToken { id: id.clone(), token: format!("{id}.{}", secret.expose()) })
}

pub fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError> {
    Ok(self.mint_admin_token("session", "session", ttl_secs)?.token)
}

pub fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError> {
    self.mint_admin_token(label, "cli", ttl_secs)
}

pub fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError> {
    let objs = self.store.list(KIND_ADMIN_TOKEN, &Default::default())?;
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

pub fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError> {
    self.store.delete(KIND_ADMIN_TOKEN, id)?;
    Ok(())
}
```

Add a small helper (module level) to avoid repeated `and_then` chains:

```rust
fn str_field(doc: &serde_json::Value, key: &str) -> String {
    doc.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
```

> `Selector::default()` lists all rows of the kind. Import `velos_store::Selector` is not needed if `&Default::default()` infers it from `list`'s signature; if inference fails, write `&Selector::default()` and add `Selector` to the `use velos_store::{...}` line.

- [ ] **Step 6: Extend `authenticate` to resolve admin tokens**

In `authenticate`, before the worker-credential lookup, try the admin-token table. Replace the body with:

```rust
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
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p velos-auth`
Expected: PASS. If clippy flags `wildcard_enum_match_arm` anywhere downstream, that is handled in Task 4.

- [ ] **Step 8: Commit**

```bash
git add crates/auth/src/lib.rs
git commit -m "feat(auth): admin session/CLI tokens and Identity::Admin resolution"
```

---

## Task 3: `TokenVerifier` seam in `velos-auth`

**Files:**
- Modify: `crates/auth/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub trait TokenVerifier: Send + Sync { fn verify(&self, presented: &str) -> Option<Identity>; }`
  - `impl TokenVerifier for StoreAuthenticator` delegating to `self.authenticate`.

- [ ] **Step 1: Write the failing test**

Append to `mod tests`:

```rust
#[test]
fn token_verifier_delegates_to_authenticate() {
    let a = auth();
    let session = a.mint_admin_session(60).unwrap();
    let v: &dyn TokenVerifier = &a;
    assert_eq!(v.verify(&session), Some(Identity::Admin));
    assert_eq!(v.verify("garbage"), None);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p velos-auth token_verifier_delegates_to_authenticate`
Expected: FAIL — `TokenVerifier` not found.

- [ ] **Step 3: Implement the trait + impl**

```rust
/// Resolves a presented bearer credential to an [`Identity`], or `None` (fail
/// closed). The server depends on this so an external OIDC verifier can be
/// substituted later without touching any endpoint.
pub trait TokenVerifier: Send + Sync {
    fn verify(&self, presented: &str) -> Option<Identity>;
}

impl TokenVerifier for StoreAuthenticator {
    fn verify(&self, presented: &str) -> Option<Identity> {
        self.authenticate(presented)
    }
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p velos-auth`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auth/src/lib.rs
git commit -m "feat(auth): TokenVerifier seam over identity resolution (OIDC-ready)"
```

---

## Task 4: server — `Identity::Admin`, initialization gate, admin-gated bootstrap mint

**Files:**
- Modify: `crates/server/src/lib.rs`

**Interfaces:**
- Consumes: `Identity` (now `Admin`+`Worker`), `AuthService`, `is_initialized` (via a new `AppState` field or a downcast — see Step 3).
- Produces: updated `require_auth` and `mint_token` guards; an `AppState`-level way to read `is_initialized`.

**Design note:** `AppState.auth` is `Option<Arc<dyn AuthService>>`. We need `is_initialized` and admin-token methods that live on `StoreAuthenticator`, not on the `AuthService` trait. Cleanest: **add the new methods to the `AuthService` trait** so they are available through the trait object. Extend the trait in `velos-auth` and implement them by calling the inherent methods.

- [ ] **Step 1: Extend the `AuthService` trait (in `velos-auth`)**

In `crates/auth/src/lib.rs`, add to `pub trait AuthService`:

```rust
    fn is_initialized(&self) -> Result<bool, AuthError>;
    fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError>;
    fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError>;
    fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError>;
    fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError>;
    fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError>;
    fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError>;
```

And in `impl AuthService for StoreAuthenticator`, add forwarding methods that delegate to the inherent ones (rename the inherent ones if needed to avoid recursion — simplest is to make the inherent `impl` methods the *only* definitions and move them into the trait impl). Concretely: move the bodies written in Tasks 1–2 from the inherent `impl StoreAuthenticator` block into `impl AuthService for StoreAuthenticator`. Keep `hash_password`/`verify_password_hash`/`str_field`/`mint_admin_token` as free/inherent helpers.

> After this step the methods are reachable through `Arc<dyn AuthService>`. Re-run `cargo test -p velos-auth` — still PASS.

- [ ] **Step 2: Write the failing server tests**

Append to the server `mod tests` (which already has `app`, `app_with_auth`, `body_json`, `tower::ServiceExt`). Add a helper to build an auth app and drive requests:

```rust
#[tokio::test]
async fn uninitialized_server_only_allows_status_and_setup() {
    let store = Arc::new(velos_store::SqliteStore::in_memory().unwrap());
    let auth = Arc::new(velos_auth::StoreAuthenticator::new(Arc::clone(&store)));
    let app = app_with_auth(store, auth);

    // status is open and reports uninitialized
    let resp = app.clone().oneshot(
        Request::builder().uri("/auth/v1/status").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["initialized"], serde_json::json!(false));

    // any /api/v1 call is rejected while uninitialized
    let resp = app.clone().oneshot(
        Request::builder().uri("/api/v1/containers").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // bootstrap mint is rejected while uninitialized
    let resp = app.oneshot(
        Request::builder().method("POST").uri("/auth/v1/tokens")
            .header("content-type", "application/json")
            .body(Body::from("{}")).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 3: Add `is_initialized` access to `require_auth` + the gate**

`require_auth` currently takes `State<Arc<dyn AuthService>>`. Replace its body so it (a) enforces the initialization gate for `/api/v1` (always — these routes only exist post-init) and (b) maps `Identity::Admin` to full access:

```rust
async fn require_auth(
    State(auth): State<Arc<dyn AuthService>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !auth.is_initialized().map_err(|e| ApiError::Internal(e.to_string()))? {
        return Err(ApiError::Unauthorized);
    }
    let token = bearer(request.headers()).ok_or(ApiError::Unauthorized)?;
    match auth.authenticate(&token).ok_or(ApiError::Unauthorized)? {
        Identity::Admin => Ok(next.run(request).await),
        Identity::Worker(who) => {
            let path = request.uri().path();
            if let Some((plural, name)) = named_path(path)
                && matches!(plural, "workers" | "leases")
                && name != who
            {
                return Err(ApiError::Forbidden);
            }
            Ok(next.run(request).await)
        }
    }
}
```

> Exhaustive match on `Identity` (no wildcard) satisfies `wildcard_enum_match_arm`.

- [ ] **Step 4: Gate bootstrap mint behind admin + init**

Change `mint_token` to require an authenticated admin:

```rust
async fn mint_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    require_admin(auth, &headers)?;
    let ttl = body
        .and_then(|Json(b)| b.get("ttlSeconds").and_then(Value::as_i64))
        .unwrap_or(24 * 3600);
    let tok = auth
        .mint_bootstrap_token(ttl)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "tokenId": tok.token_id,
        "secret": tok.secret.expose(),
        "expiresAt": tok.expires_at.to_rfc3339(),
    })))
}
```

Add a shared admin guard near `bearer`:

```rust
/// Require a valid admin token on `headers`, failing closed. Also enforces the
/// initialization gate: an uninitialized server has no admin and rejects.
fn require_admin(auth: &Arc<dyn AuthService>, headers: &HeaderMap) -> Result<(), ApiError> {
    if !auth.is_initialized().map_err(|e| ApiError::Internal(e.to_string()))? {
        return Err(ApiError::Unauthorized);
    }
    let token = bearer(headers).ok_or(ApiError::Unauthorized)?;
    match auth.authenticate(&token) {
        Some(Identity::Admin) => Ok(()),
        Some(Identity::Worker(_)) => Err(ApiError::Forbidden),
        None => Err(ApiError::Unauthorized),
    }
}
```

- [ ] **Step 5: Run the new test (status/setup endpoints come in Task 5)**

The test references `/auth/v1/status`, added in Task 5. To keep Task 4 independently green, temporarily run only the mint/gate assertions by splitting: run the existing suite to confirm no regression, and defer the full new test's `status` assertion until Task 5.

Run: `cargo test -p velos-server auth_flow_mint_register_then_scoped_access`
Expected: This existing test mints a token with no admin — it will now FAIL (mint requires admin). **Update that test in Task 5** when the setup/login flow exists. For now, confirm compilation: `cargo build -p velos-server`.

- [ ] **Step 6: Commit**

```bash
git add crates/auth/src/lib.rs crates/server/src/lib.rs
git commit -m "feat(server): admin identity, init gate, admin-gated bootstrap mint"
```

---

## Task 5: server — `status`, `setup`, `login`, `me` endpoints

**Files:**
- Modify: `crates/server/src/lib.rs`

**Interfaces:**
- Consumes: `AppState`, `ApiError`, `bearer`, `require_admin`, `Secret`, `Identity`.
- Produces handlers `auth_status`, `setup`, `login`, `whoami`; routes registered in `app_with_auth`.

**Constants:** `const SESSION_TTL_SECS: i64 = 12 * 3600;`

- [ ] **Step 1: Write/repair the failing tests**

Replace the existing `auth_flow_mint_register_then_scoped_access` test's opening so it sets up + logs in first, then add a full admin-flow test:

```rust
#[tokio::test]
async fn admin_setup_login_token_flow() {
    let store = Arc::new(velos_store::SqliteStore::in_memory().unwrap());
    let auth = Arc::new(velos_auth::StoreAuthenticator::new(Arc::clone(&store)));
    let app = app_with_auth(store, auth);

    // setup
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/auth/v1/setup")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"admin","password":"pw"}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // setup again -> 409
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/auth/v1/setup")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"x","password":"y"}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // login
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/auth/v1/login")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"admin","password":"pw"}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session = body_json(resp).await["token"].as_str().unwrap().to_string();

    // bad password -> 401
    let resp = app.clone().oneshot(
        Request::builder().method("POST").uri("/auth/v1/login")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"admin","password":"WRONG"}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // me -> admin
    let resp = app.clone().oneshot(
        Request::builder().uri("/auth/v1/me")
            .header("authorization", format!("Bearer {session}"))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["identity"], serde_json::json!("admin"));

    // admin can mint a bootstrap token now
    let resp = app.oneshot(
        Request::builder().method("POST").uri("/auth/v1/tokens")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {session}"))
            .body(Body::from("{}")).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p velos-server admin_setup_login_token_flow`
Expected: FAIL — routes 404.

- [ ] **Step 3: Implement the handlers**

```rust
/// `GET /auth/v1/status` — always open; lets clients pick setup vs login.
async fn auth_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let initialized = auth.is_initialized().map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "initialized": initialized })))
}

/// `POST /auth/v1/setup` — single-shot admin account creation. Body `{username,password}`.
async fn setup(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let username = req.get("username").and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest("username required".into()))?;
    let password = req.get("password").and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest("password required".into()))?;
    match auth.setup_admin(username, &velos_auth::Secret::new(password)) {
        Ok(()) => Ok(Json(serde_json::json!({ "initialized": true }))),
        Err(velos_auth::AuthError::AlreadyInitialized) => {
            Err(ApiError::Conflict("already initialized".into()))
        }
        Err(velos_auth::AuthError::Invalid) => Err(ApiError::BadRequest("invalid setup".into())),
        Err(e) => Err(ApiError::Internal(e.to_string())),
    }
}

/// `POST /auth/v1/login` — username+password -> short-TTL session token.
async fn login(
    State(state): State<AppState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let username = req.get("username").and_then(Value::as_str).unwrap_or_default();
    let password = req.get("password").and_then(Value::as_str).unwrap_or_default();
    auth.verify_password(username, &velos_auth::Secret::new(password))
        .map_err(|_| ApiError::Unauthorized)?;
    let token = auth.mint_admin_session(SESSION_TTL_SECS)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "token": token })))
}

/// `GET /auth/v1/me` — echo the caller's identity.
async fn whoami(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    let token = bearer(&headers).ok_or(ApiError::Unauthorized)?;
    let id = match auth.authenticate(&token).ok_or(ApiError::Unauthorized)? {
        Identity::Admin => serde_json::json!("admin"),
        Identity::Worker(w) => serde_json::json!({ "worker": w }),
    };
    Ok(Json(serde_json::json!({ "identity": id })))
}
```

Add the constant near the other consts:

```rust
const SESSION_TTL_SECS: i64 = 12 * 3600;
```

- [ ] **Step 4: Register the routes**

In `app_with_auth`, extend the open router:

```rust
    Router::new()
        .route("/auth/v1/status", get(auth_status))
        .route("/auth/v1/setup", post(setup))
        .route("/auth/v1/login", post(login))
        .route("/auth/v1/me", get(whoami))
        .route("/auth/v1/tokens", post(mint_token))
        .route("/auth/v1/register", post(register))
        .merge(protected)
        .fallback(serve_ui)
        .with_state(state)
```

> `mint_token` now also takes `HeaderMap` (Task 4) — its route signature is unchanged because axum extracts it from the handler args.

- [ ] **Step 5: Repair the legacy worker-flow test**

In `auth_flow_mint_register_then_scoped_access`, before minting the bootstrap token, perform setup + login and use the session token as the bearer on the `POST /auth/v1/tokens` call (mirror Step 1's setup+login, then add `.header("authorization", format!("Bearer {session}"))` to the mint request). The rest of the worker registration/scoped-access assertions stay.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p velos-server`
Expected: PASS (new flow + repaired legacy + Task 4 gate test).

- [ ] **Step 7: Commit**

```bash
git add crates/server/src/lib.rs
git commit -m "feat(server): status/setup/login/me endpoints with single-shot setup"
```

---

## Task 6: server — admin CLI-token endpoints

**Files:**
- Modify: `crates/server/src/lib.rs`

**Interfaces:**
- Consumes: `require_admin`, `AppState`, `MintedToken`, `AdminTokenInfo`.
- Produces handlers `list_tokens`, `create_token`, `revoke_token`; routes under `/auth/v1/admin/tokens`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn admin_can_create_list_and_revoke_cli_tokens() {
    let store = Arc::new(velos_store::SqliteStore::in_memory().unwrap());
    let auth = Arc::new(velos_auth::StoreAuthenticator::new(Arc::clone(&store)));
    let app = app_with_auth(store, auth);

    // setup + login (reuse helper inline)
    app.clone().oneshot(Request::builder().method("POST").uri("/auth/v1/setup")
        .header("content-type","application/json")
        .body(Body::from(r#"{"username":"admin","password":"pw"}"#)).unwrap()).await.unwrap();
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/auth/v1/login")
        .header("content-type","application/json")
        .body(Body::from(r#"{"username":"admin","password":"pw"}"#)).unwrap()).await.unwrap();
    let session = body_json(resp).await["token"].as_str().unwrap().to_string();

    // create a CLI token
    let resp = app.clone().oneshot(Request::builder().method("POST").uri("/auth/v1/admin/tokens")
        .header("content-type","application/json")
        .header("authorization", format!("Bearer {session}"))
        .body(Body::from(r#"{"label":"laptop"}"#)).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    let cli_token = created["token"].as_str().unwrap().to_string();
    let id = created["id"].as_str().unwrap().to_string();

    // the CLI token works on /api/v1
    let resp = app.clone().oneshot(Request::builder().uri("/api/v1/containers")
        .header("authorization", format!("Bearer {cli_token}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // list shows it (no secret)
    let resp = app.clone().oneshot(Request::builder().uri("/auth/v1/admin/tokens")
        .header("authorization", format!("Bearer {session}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    let list = body_json(resp).await;
    assert!(list["items"].as_array().unwrap().iter().any(|t| t["id"] == serde_json::json!(id)));

    // a non-admin (no token) cannot list
    let resp = app.clone().oneshot(Request::builder().uri("/auth/v1/admin/tokens")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // revoke -> CLI token no longer authenticates
    let resp = app.clone().oneshot(Request::builder().method("DELETE")
        .uri(format!("/auth/v1/admin/tokens/{id}"))
        .header("authorization", format!("Bearer {session}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app.oneshot(Request::builder().uri("/api/v1/containers")
        .header("authorization", format!("Bearer {cli_token}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p velos-server admin_can_create_list_and_revoke_cli_tokens`
Expected: FAIL — routes 404.

- [ ] **Step 3: Implement handlers**

```rust
const CLI_TOKEN_TTL_SECS: i64 = 365 * 24 * 3600;

async fn list_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    require_admin(auth, &headers)?;
    let items = auth.list_admin_tokens().map_err(|e| ApiError::Internal(e.to_string()))?;
    let items: Vec<Value> = items.into_iter().map(|t| serde_json::json!({
        "id": t.id, "label": t.label, "kind": t.kind,
        "createdAt": t.created_at, "expiresAt": t.expires_at,
    })).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

async fn create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    require_admin(auth, &headers)?;
    let label = req.get("label").and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest("label required".into()))?;
    let ttl = req.get("ttlSeconds").and_then(Value::as_i64).unwrap_or(CLI_TOKEN_TTL_SECS);
    let minted = auth.mint_cli_token(label, ttl).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": minted.id, "token": minted.token })))
}

async fn revoke_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let auth = state.auth.as_ref().ok_or(ApiError::NotFound)?;
    require_admin(auth, &headers)?;
    auth.revoke_admin_token(&id).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "revoked": id })))
}
```

- [ ] **Step 4: Register routes** in `app_with_auth` (open router chain — admin enforced inside each handler):

```rust
        .route("/auth/v1/admin/tokens", get(list_tokens).post(create_token))
        .route("/auth/v1/admin/tokens/:id", axum::routing::delete(revoke_token))
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p velos-server`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/server/src/lib.rs
git commit -m "feat(server): admin CLI-token create/list/revoke endpoints"
```

---

## Task 7: velosctl — config module (pure, tested)

**Files:**
- Modify: `crates/velosctl/Cargo.toml` (add `dirs`, `serde`)
- Modify: `crates/velosctl/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct Config { pub server: Option<String>, pub token: Option<String> }` (`Serialize`/`Deserialize`, `Default`).
  - `pub fn resolve_server(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> String` (precedence flag>env>cfg>default `http://127.0.0.1:8080`).
  - `pub fn resolve_token(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> Option<String>` (flag>env>cfg).
  - `pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8080";`

- [ ] **Step 1: Add deps**

In `crates/velosctl/Cargo.toml`:

```toml
serde = { version = "1.0", features = ["derive"] }
dirs = "5"
```

- [ ] **Step 2: Write failing tests** (append to `crates/velosctl/src/lib.rs` `mod tests`):

```rust
#[test]
fn server_precedence_flag_env_config_default() {
    let cfg = Config { server: Some("http://cfg:1".into()), token: None };
    assert_eq!(resolve_server(Some("http://flag:1"), Some("http://env:1"), &cfg), "http://flag:1");
    assert_eq!(resolve_server(None, Some("http://env:1"), &cfg), "http://env:1");
    assert_eq!(resolve_server(None, None, &cfg), "http://cfg:1");
    assert_eq!(resolve_server(None, None, &Config::default()), DEFAULT_SERVER);
}

#[test]
fn token_precedence_flag_env_config() {
    let cfg = Config { server: None, token: Some("cfgtok".into()) };
    assert_eq!(resolve_token(Some("flagtok"), Some("envtok"), &cfg).as_deref(), Some("flagtok"));
    assert_eq!(resolve_token(None, Some("envtok"), &cfg).as_deref(), Some("envtok"));
    assert_eq!(resolve_token(None, None, &cfg).as_deref(), Some("cfgtok"));
    assert_eq!(resolve_token(None, None, &Config::default()), None);
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p velosctl server_precedence_flag_env_config_default token_precedence_flag_env_config`
Expected: FAIL — `Config`/`resolve_*` not found.

- [ ] **Step 4: Implement** (add to `crates/velosctl/src/lib.rs`):

```rust
use serde::{Deserialize, Serialize};

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:8080";

/// Persisted velosctl credentials (`~/.velos/config`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Resolve the server URL: flag > env > config > built-in default.
pub fn resolve_server(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> String {
    flag.map(str::to_string)
        .or_else(|| env.map(str::to_string))
        .or_else(|| cfg.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

/// Resolve the bearer token: flag > env > config.
pub fn resolve_token(flag: Option<&str>, env: Option<&str>, cfg: &Config) -> Option<String> {
    flag.map(str::to_string)
        .or_else(|| env.map(str::to_string))
        .or_else(|| cfg.token.clone())
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p velosctl`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/velosctl/Cargo.toml crates/velosctl/src/lib.rs Cargo.lock
git commit -m "feat(velosctl): config model + server/token resolution precedence"
```

---

## Task 8: velosctl — `login`/`logout` commands + wire resolution

**Files:**
- Modify: `crates/velosctl/src/lib.rs` (config file IO helpers)
- Modify: `crates/velosctl/src/main.rs`

**Interfaces:**
- Consumes: `Config`, `resolve_server`, `resolve_token`, `DEFAULT_SERVER`.
- Produces:
  - `pub fn config_path() -> Option<std::path::PathBuf>` (`~/.velos/config`).
  - `pub fn load_config() -> Config` (missing/garbage → `Config::default()`).
  - `pub fn save_config(cfg: &Config) -> std::io::Result<()>` (creates `~/.velos` 0700, writes file 0600).
  - velosctl gains `login`/`logout` subcommands; `--server`/`--token` resolution applied for all commands.

- [ ] **Step 1: Implement config IO** (`crates/velosctl/src/lib.rs`):

```rust
use std::fs;
use std::path::PathBuf;

/// `~/.velos/config`, if a home directory is known.
pub fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".velos").join("config"))
}

/// Load config; any error (missing/garbage) yields defaults (fail soft on read).
pub fn load_config() -> Config {
    let Some(p) = config_path() else { return Config::default() };
    match fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

/// Persist config to `~/.velos/config` with 0700 dir / 0600 file perms.
pub fn save_config(cfg: &Config) -> std::io::Result<()> {
    let Some(p) = config_path() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"));
    };
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
        set_mode(dir, 0o700);
    }
    let body = serde_json::to_string_pretty(cfg).unwrap_or_else(|_| "{}".to_string());
    fs::write(&p, body)?;
    set_mode(&p, 0o600);
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &std::path::Path, _mode: u32) {}
```

> `serde_json::to_string_pretty` on a plain struct cannot fail in practice; `unwrap_or_else` keeps the `unwrap_used` lint satisfied without a panic path.

- [ ] **Step 2: Write a config-IO round-trip test** (uses a temp `HOME`):

```rust
#[test]
fn config_round_trips_through_disk() {
    // Isolate HOME so we don't touch the real ~/.velos.
    let tmp = std::env::temp_dir().join(format!("velosctl-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // SAFETY: single-threaded test process.
    unsafe { std::env::set_var("HOME", &tmp) };

    let cfg = Config { server: Some("http://h:9".into()), token: Some("tok".into()) };
    save_config(&cfg).unwrap();
    let back = load_config();
    assert_eq!(back.server.as_deref(), Some("http://h:9"));
    assert_eq!(back.token.as_deref(), Some("tok"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config_path().unwrap()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
```

Run: `cargo test -p velosctl config_round_trips_through_disk`
Expected: PASS (after Step 1).

- [ ] **Step 3: Add `login`/`logout` subcommands** (`crates/velosctl/src/main.rs`)

Add to `enum Command`:

```rust
    /// Save a server + token credential to ~/.velos/config.
    Login {
        #[arg(long)]
        token: String,
    },
    /// Remove the saved credential.
    Logout,
```

Change the global `--server` arg so its default is removed (resolution now owns the default):

```rust
    /// server base URL (overrides VELOS_SERVER and the saved config).
    #[arg(long, global = true)]
    server: Option<String>,
```

- [ ] **Step 4: Apply resolution in `main`**

At the top of `main`, after `Cli::parse()`:

```rust
    let cfg = velosctl::load_config();
    let server = velosctl::resolve_server(
        cli.server.as_deref(),
        std::env::var("VELOS_SERVER").ok().as_deref(),
        &cfg,
    );
    let token = velosctl::resolve_token(
        cli.token.as_deref(),
        std::env::var("VELOS_TOKEN").ok().as_deref(),
        &cfg,
    );
```

Replace all later `cli.server` uses with `server`, and `&cli.token` with `&token`.

- [ ] **Step 5: Implement the `Login`/`Logout` match arms**

```rust
        Command::Login { token } => {
            // Validate the token against the resolved server before saving.
            let url = format!("{}/auth/v1/me", server.trim_end_matches('/'));
            let resp = http.get(url).bearer_auth(&token).send().await?;
            if !resp.status().is_success() {
                bail!("token rejected by {server}: {}", resp.status());
            }
            let saved = velosctl::Config { server: Some(server.clone()), token: Some(token) };
            velosctl::save_config(&saved)?;
            println!("logged in to {server}");
        }
        Command::Logout => {
            velosctl::save_config(&velosctl::Config::default())?;
            println!("logged out");
        }
```

- [ ] **Step 6: Build + run the full velosctl suite**

Run: `cargo test -p velosctl && cargo build -p velosctl`
Expected: PASS / builds.

- [ ] **Step 7: Commit**

```bash
git add crates/velosctl/src/lib.rs crates/velosctl/src/main.rs
git commit -m "feat(velosctl): login/logout + persisted server/token resolution"
```

---

## Task 9: Web UI — replace bootstrap hack with session auth

**Files:**
- Rewrite: `web/src/auth.ts`
- Modify: `web/src/api.ts`

**Interfaces:**
- Produces (in `auth.ts`): `getStatus(): Promise<{initialized:boolean}>`, `setup(username,password): Promise<void>`, `login(username,password): Promise<void>`, `logout(): void`, `sessionToken(): string | null`, `onAuthChange(cb)` (simple listener so React can re-render).

- [ ] **Step 1: Rewrite `web/src/auth.ts`**

```ts
// Browser-side admin session handling.
//
// The dashboard authenticates as the Velos admin: it logs in with the
// username/password set up on first run and stores the returned short-lived
// session token in localStorage, sending it as a Bearer on every API call.

const STORAGE_KEY = "velos.session";
const listeners = new Set<() => void>();

export function sessionToken(): string | null {
  try { return localStorage.getItem(STORAGE_KEY); } catch { return null; }
}

function setToken(tok: string | null): void {
  try {
    if (tok) localStorage.setItem(STORAGE_KEY, tok);
    else localStorage.removeItem(STORAGE_KEY);
  } catch { /* ignore */ }
  listeners.forEach((l) => l());
}

export function onAuthChange(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export async function getStatus(): Promise<{ initialized: boolean }> {
  const r = await fetch("/auth/v1/status");
  if (!r.ok) throw new Error(`status ${r.status}`);
  return r.json();
}

async function postJson(path: string, body: unknown): Promise<Response> {
  return fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export async function setup(username: string, password: string): Promise<void> {
  const r = await postJson("/auth/v1/setup", { username, password });
  if (!r.ok) throw new Error((await r.json().catch(() => ({})))?.error ?? `setup ${r.status}`);
}

export async function login(username: string, password: string): Promise<void> {
  const r = await postJson("/auth/v1/login", { username, password });
  if (!r.ok) throw new Error("invalid username or password");
  const { token } = await r.json();
  setToken(token);
}

export function logout(): void {
  setToken(null);
}
```

- [ ] **Step 2: Update `web/src/api.ts`**

Replace the `getCredential`/`forgetCredential` import and `send` body:

```ts
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { logout, sessionToken } from "./auth";
import type { Container, Lease, List, RestartPolicy, Worker } from "./types";

const BASE = "/api/v1";

async function send<T>(path: string, init: RequestInit): Promise<T> {
  const tok = sessionToken();
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(tok ? { authorization: `Bearer ${tok}` } : {}),
      ...(init.headers ?? {}),
    },
  });

  // Session expired/invalid -> drop it; the app shell will show the login screen.
  if (res.status === 401) {
    logout();
    throw new Error("unauthorized");
  }
  if (!res.ok) {
    let detail = "";
    try { detail = (await res.json())?.error ?? ""; } catch { /* ignore */ }
    throw new Error(`${res.status} ${res.statusText}${detail ? ` — ${detail}` : ""}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}
```

(Keep the rest of `api.ts` unchanged.)

- [ ] **Step 3: Type-check the web build**

Run: `cd web && npx tsc -b`
Expected: No type errors (the App-shell wiring in Task 10 consumes the new exports; `tsc` may flag unused exports only if `noUnusedLocals` — exports are not locals, so this passes).

- [ ] **Step 4: Commit**

```bash
git add web/src/auth.ts web/src/api.ts
git commit -m "feat(web): session-token auth, drop bootstrap-worker hack"
```

---

## Task 10: Web UI — Setup / Login screens + auth gate in `App`

**Files:**
- Create: `web/src/views/AuthGate.tsx`
- Modify: `web/src/App.tsx`

**Interfaces:**
- Consumes: `getStatus`, `setup`, `login`, `sessionToken`, `onAuthChange` from `./auth`.
- Produces: `AuthGate` component that renders setup/login when there is no session, else its `children`.

- [ ] **Step 1: Create `web/src/views/AuthGate.tsx`**

```tsx
import { useEffect, useState } from "react";
import { getStatus, login, onAuthChange, sessionToken, setup } from "../auth";

export function AuthGate({ children }: { children: React.ReactNode }) {
  const [token, setTok] = useState<string | null>(sessionToken());
  const [initialized, setInitialized] = useState<boolean | null>(null);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => onAuthChange(() => setTok(sessionToken())), []);
  useEffect(() => { getStatus().then((s) => setInitialized(s.initialized)).catch(() => setInitialized(true)); }, []);

  if (token) return <>{children}</>;
  if (initialized === null) return <div className="p-8 text-slate-400">Loading…</div>;

  const isSetup = !initialized;
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      if (isSetup) { await setup(username, password); setInitialized(true); }
      await login(username, password);
    } catch (err) { setError(err instanceof Error ? err.message : String(err)); }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-slate-950 text-slate-100">
      <form onSubmit={submit} className="w-80 space-y-4 rounded-lg border border-slate-800 p-6">
        <h1 className="text-lg font-semibold">{isSetup ? "Set up Velos admin" : "Sign in"}</h1>
        <input className="w-full rounded bg-slate-900 px-3 py-2" placeholder="username"
          value={username} onChange={(e) => setUsername(e.target.value)} />
        <input className="w-full rounded bg-slate-900 px-3 py-2" type="password" placeholder="password"
          value={password} onChange={(e) => setPassword(e.target.value)} />
        {error && <p className="text-sm text-red-400">{error}</p>}
        <button className="w-full rounded bg-sky-600 px-3 py-2 font-medium hover:bg-sky-500" type="submit">
          {isSetup ? "Create admin & sign in" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
```

- [ ] **Step 2: Wrap the app in `AuthGate`** (`web/src/App.tsx`)

Import and wrap the existing top-level layout return value:

```tsx
import { AuthGate } from "./views/AuthGate";
// ...
// Wrap whatever the component currently returns:
return <AuthGate>{/* existing dashboard JSX */}</AuthGate>;
```

(If `App` returns a fragment, wrap that fragment with `<AuthGate>…</AuthGate>`.)

- [ ] **Step 3: Type-check + build**

Run: `cd web && npm run build`
Expected: build succeeds; `dist/` produced.

- [ ] **Step 4: Commit**

```bash
git add web/src/views/AuthGate.tsx web/src/App.tsx
git commit -m "feat(web): first-run setup and login gate"
```

---

## Task 11: Web UI — Tokens page (create / list / revoke CLI tokens)

**Files:**
- Create: `web/src/views/Tokens.tsx`
- Modify: `web/src/api.ts` (token hooks), `web/src/App.tsx` (nav entry)

**Interfaces:**
- Consumes: `send`/`http` from `api.ts` (admin endpoints), `@tanstack/react-query`.
- Produces: `useTokens`, `useCreateToken`, `useRevokeToken` hooks; `Tokens` view with a one-time secret display.

- [ ] **Step 1: Add token hooks to `web/src/api.ts`**

```ts
export interface AdminToken { id: string; label: string; kind: string; createdAt: string; expiresAt: string; }

function authHttp<T>(path: string, init: RequestInit = {}): Promise<T> {
  // /auth/v1 is not under BASE; call it directly with the session bearer.
  return sendAuth<T>(path, init);
}

async function sendAuth<T>(path: string, init: RequestInit): Promise<T> {
  const tok = sessionToken();
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(tok ? { authorization: `Bearer ${tok}` } : {}), ...(init.headers ?? {}) },
  });
  if (res.status === 401) { logout(); throw new Error("unauthorized"); }
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export function useTokens() {
  return useQuery({ queryKey: ["admin-tokens"], queryFn: () => authHttp<{ items: AdminToken[] }>("/auth/v1/admin/tokens"), select: (d) => d.items ?? [] });
}
export function useCreateToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (label: string) => authHttp<{ id: string; token: string }>("/auth/v1/admin/tokens", { method: "POST", body: JSON.stringify({ label }) }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["admin-tokens"] }),
  });
}
export function useRevokeToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => authHttp<void>(`/auth/v1/admin/tokens/${id}`, { method: "DELETE" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["admin-tokens"] }),
  });
}
```

Add `import { logout, sessionToken } from "./auth";` if not already present (Task 9 added it).

- [ ] **Step 2: Create `web/src/views/Tokens.tsx`**

```tsx
import { useState } from "react";
import { useCreateToken, useRevokeToken, useTokens } from "../api";

export function Tokens() {
  const { data: tokens } = useTokens();
  const create = useCreateToken();
  const revoke = useRevokeToken();
  const [label, setLabel] = useState("");
  const [secret, setSecret] = useState<string | null>(null);

  const onCreate = async () => {
    if (!label.trim()) return;
    const r = await create.mutateAsync(label.trim());
    setSecret(r.token);
    setLabel("");
  };

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <input className="rounded bg-slate-900 px-3 py-2" placeholder="token label (e.g. laptop)"
          value={label} onChange={(e) => setLabel(e.target.value)} />
        <button className="rounded bg-sky-600 px-3 py-2" onClick={onCreate}>Create CLI token</button>
      </div>
      {secret && (
        <div className="rounded border border-amber-600 bg-amber-950/40 p-3 text-sm">
          <p className="mb-1 font-medium text-amber-300">Copy this token now — it will not be shown again:</p>
          <code className="break-all">{secret}</code>
          <p className="mt-2 text-slate-400">Use it with: <code>velosctl login --token &lt;token&gt; --server &lt;url&gt;</code></p>
        </div>
      )}
      <table className="w-full text-sm">
        <thead><tr className="text-left text-slate-400"><th>Label</th><th>Kind</th><th>Expires</th><th /></tr></thead>
        <tbody>
          {(tokens ?? []).map((t) => (
            <tr key={t.id} className="border-t border-slate-800">
              <td className="py-1">{t.label}</td><td>{t.kind}</td><td>{t.expiresAt}</td>
              <td className="text-right"><button className="text-red-400" onClick={() => revoke.mutate(t.id)}>Revoke</button></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
```

- [ ] **Step 3: Add a nav entry / route for `Tokens`** in `web/src/App.tsx`

Follow the existing view-switching pattern (the app uses simple view state, not a router). Add a "Tokens" entry to whatever nav list drives `Overview`/`Containers`/`Workers`, rendering `<Tokens />` when selected. (Mirror the existing nav item structure exactly; if views are keyed by a string union, add `"tokens"` and a `case`/branch.)

- [ ] **Step 4: Build**

Run: `cd web && npm run build`
Expected: build succeeds.

- [ ] **Step 5: Commit**

```bash
git add web/src/views/Tokens.tsx web/src/api.ts web/src/App.tsx
git commit -m "feat(web): admin CLI-token management page"
```

---

## Task 12: End-to-end test in `velos-tests`

**Files:**
- Modify: `crates/tests/tests/e2e.rs` (add an admin-auth flow test)

**Interfaces:**
- Consumes: `velos_server::app_with_auth`, `velos_auth::StoreAuthenticator`, `velos_store::SqliteStore`, the existing e2e harness (inspect the file for how it boots a server / issues requests).

- [ ] **Step 1: Inspect the existing harness**

Run: `sed -n '1,60p' crates/tests/tests/e2e.rs` and reuse its server-boot helper. If it uses `app(store)` (no auth), add a parallel helper `spawn_auth()` that builds `app_with_auth` and binds an ephemeral `TcpListener` (`127.0.0.1:0`), returning the base URL.

- [ ] **Step 2: Write the e2e test**

```rust
#[tokio::test]
async fn admin_auth_end_to_end() {
    let store = std::sync::Arc::new(velos_store::SqliteStore::in_memory().unwrap());
    let auth = std::sync::Arc::new(velos_auth::StoreAuthenticator::new(std::sync::Arc::clone(&store)));
    let app = velos_server::app_with_auth(store, auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let http = reqwest::Client::new();

    // setup + login
    http.post(format!("{base}/auth/v1/setup")).json(&serde_json::json!({"username":"admin","password":"pw"})).send().await.unwrap();
    let session: String = http.post(format!("{base}/auth/v1/login"))
        .json(&serde_json::json!({"username":"admin","password":"pw"})).send().await.unwrap()
        .json::<serde_json::Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    // create a CLI token, then use it on /api/v1
    let cli: String = http.post(format!("{base}/auth/v1/admin/tokens"))
        .bearer_auth(&session).json(&serde_json::json!({"label":"ci"})).send().await.unwrap()
        .json::<serde_json::Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    let r = http.get(format!("{base}/api/v1/containers")).bearer_auth(&cli).send().await.unwrap();
    assert!(r.status().is_success());

    // worker bootstrap still works (admin mints, worker registers)
    let boot: serde_json::Value = http.post(format!("{base}/auth/v1/tokens"))
        .bearer_auth(&session).json(&serde_json::json!({"ttlSeconds":3600})).send().await.unwrap()
        .json().await.unwrap();
    let boot_tok = format!("{}.{}", boot["tokenId"].as_str().unwrap(), boot["secret"].as_str().unwrap());
    let reg = http.post(format!("{base}/auth/v1/register")).bearer_auth(&boot_tok)
        .json(&serde_json::json!({"name":"w1"})).send().await.unwrap();
    assert!(reg.status().is_success());
}
```

> If `crates/tests` lacks `reqwest`, add it to `[dev-dependencies]` (mirror `velosctl`'s features: `default-features = false, features = ["json"]`).

- [ ] **Step 2b: Ensure deps**

Run: `grep -n "reqwest\|velos-auth" crates/tests/Cargo.toml` and add what's missing to `[dev-dependencies]`/`[dependencies]`.

- [ ] **Step 3: Run the e2e test**

Run: `cargo test -p velos-tests admin_auth_end_to_end`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/tests/tests/e2e.rs crates/tests/Cargo.toml Cargo.lock
git commit -m "test(e2e): admin setup/login/CLI-token + worker bootstrap flow"
```

---

## Task 13: Docs + full gate

**Files:**
- Modify: `README.md` and/or `docs/getting-started.md` (document `velosctl login`, first-run UI setup)

- [ ] **Step 1: Document the auth flow**

Add a short "Authentication" section to `docs/getting-started.md`: first run opens the web UI → set admin username/password → create a CLI token → `velosctl login --token <token> --server <url>` → subsequent `velosctl` commands need no flags. Mention `VELOS_SERVER`/`VELOS_TOKEN` env overrides.

- [ ] **Step 2: Run the full pre-PR gate**

Run: `make check`
Expected: `cargo fmt --all --check` clean, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo test --workspace` green.

Fix any fmt/clippy issues (common: import ordering, an inadvertent wildcard match, an `unwrap` in non-test code → replace with `?`/`unwrap_or`).

- [ ] **Step 3: Commit**

```bash
git add README.md docs/getting-started.md
git commit -m "docs: document velosctl login and first-run admin setup"
```

---

## Self-Review (completed)

- **Spec coverage:** two-tier identity (Tasks 2,4) ✔; first-run UI setup (Tasks 5,10) ✔; PAT/CLI tokens (Tasks 2,6,11) ✔; one opaque-token primitive (Task 2) ✔; argon2 (Task 1) ✔; UI session token in browser storage (Task 9) ✔; token resolution flag>env>config + server persistence (Tasks 7,8) ✔; init gate (Tasks 4,5) ✔; admin-gated bootstrap mint / closed hole (Task 4) ✔; TokenVerifier OIDC seam (Task 3) ✔; testing across unit/server/ctl/e2e (Tasks 1–8,12) ✔; logout clears server+token (Task 8) ✔.
- **Placeholder scan:** all code steps contain real code; UI nav wiring (Tasks 10–11 Step "App") references the existing pattern rather than inventing one, since `App.tsx`'s exact view-switching must be read at edit time — implementer instruction is explicit.
- **Type consistency:** `MintedToken{id,token}`, `AdminTokenInfo{id,label,kind,created_at,expires_at}`, `Config{server,token}`, `resolve_server/resolve_token`, `require_admin`, `Identity::{Admin,Worker}` used consistently across Rust tasks; web `sessionToken`/`logout`/`setup`/`login`/`getStatus` consistent across `auth.ts`, `api.ts`, `AuthGate.tsx`, `Tokens.tsx`.
