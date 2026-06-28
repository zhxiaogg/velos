# Velos — Admin Authentication for velosctl ↔ apiserver

**Date:** 2026-06-28
**Status:** Approved design, pre-implementation

## Summary

Velos already has **worker** authentication (the `kubeadm`-style bootstrap-token →
per-worker credential flow in `velos-auth`). It has **no operator/admin identity**:
`velosctl` is an operator tool, but the only `Identity` is `Worker`, and the
bootstrap-token mint endpoint (`POST /auth/v1/tokens`) is currently **unauthenticated**
in `app_with_auth` — anyone who can reach the server can mint a token, register as a
worker, and obtain a credential.

This work introduces an **admin identity** and authenticates the `velosctl ↔ apiserver`
path using the **GitHub personal-access-token (PAT) model**:

1. **First run (web UI):** an operator sets up the admin account (username + password).
   This is the only bootstrap path and is single-shot.
2. **Returning (web UI):** the operator logs in with username/password; the browser
   holds a short-lived **session token**.
3. **Token creation (web UI):** the authenticated operator creates a **named CLI token**
   (shown exactly once).
4. **velosctl:** `velosctl login --token <tok>` validates and saves the token to
   `~/.velos/config`; every subsequent call sends it as a bearer. The CLI never sees
   the password.

The password never leaves the browser. `velosctl` only ever carries an opaque bearer
token.

### Decisions locked during design

| Topic | Decision |
|---|---|
| Authorization model | **Two-tier**: `Admin` vs `Worker`. Admin = full access; Worker = node-scoped (unchanged) |
| Admin count | **Single admin account** for now (username + password); multi-admin is a documented future extension |
| Bootstrap | **First-run setup via the web UI** (set admin username + password); single-shot |
| CLI auth | **PAT model** — admin creates a named CLI token in the UI; `velosctl` carries it as a bearer |
| Token primitive | **One opaque-token mechanism** for UI sessions and CLI tokens; persisted only as a hash; looked up in the `Store`; revocation = delete the row |
| Token format | **Opaque** (not JWT). JWT's only real win — stateless verification — is irrelevant to a single apiserver that already does store-lookup auth, and JWT breaks easy revocation. JWT is reserved for the future OIDC seam |
| Password hashing | **argon2** (salted) for the human password; `Secret`/SHA-256 retained for high-entropy random tokens |
| UI session storage | **Short-TTL opaque token in browser storage** (one code path, matches the CLI). HttpOnly cookie is a documented future hardening option |
| Token resolution (ctl) | **`--token` flag > `VELOS_TOKEN` env > `~/.velos/config`** |
| Server resolution (ctl) | **`--server` flag > `VELOS_SERVER` env > `~/.velos/config` > default**; `login` persists the server alongside the token |
| Initialization gate | While uninitialized, **only `/auth/v1/status` and `/auth/v1/setup` are reachable**; everything else fails closed (401) |
| OIDC | **Not now.** A `TokenVerifier` seam is added so an external OIDC IdP can be integrated later without endpoint changes |

## Why not full OAuth / OIDC now

OAuth 2.1 deprecated the resource-owner password grant — the only flow where a CLI
takes a username/password directly. The blessed CLI flows (Device Authorization,
Authorization Code + PKCE) and the Kubernetes OIDC model all **presuppose an external
Identity Provider**: the apiserver does not run an IdP, it *validates* JWTs the IdP
issues. Velos has no external IdP, and standing one up (or building our own OAuth
authorization server) is a large, security-critical subsystem that does not fit the
"username/password set up in the UI" flow. The PAT model gives us the operator
experience we want today; the `TokenVerifier` seam keeps the OIDC path open for later.

## Identity model

Extend the existing sum type (Principle #2, make illegal states unrepresentable):

```rust
pub enum Identity {
    Worker(String),  // unchanged: node-scoped access
    Admin,           // new: full access
}
```

- **Admin** — full CRUD on every `/api/v1` resource; exclusive access to the privileged
  endpoints (bootstrap-token minting, CLI-token management).
- **Worker** — node-scoped access exactly as today (`require_auth` already enforces that
  a worker may only address its own `Worker`/`Lease` by name). Unchanged.

## Token primitive

All admin auth flows through **one** mechanism, reusing the existing store-lookup design:

- A random opaque token, persisted **only as a SHA-256 hash** via the existing `Secret`
  newtype (Principle #1; secrets never `Display`/`Debug`/log).
- Each token row carries: `label`, `kind` (`session` | `cli`), `createdAt`, `expiresAt`.
- **UI session token** and **CLI token** are the *same* primitive — they differ only in
  TTL and label. A session token is short-lived; a CLI token is long-lived and named.
- **Revocation = delete the row** (already a first-class operation in `velos-auth`).
- Stored under a store kind the REST router does not expose (as bootstrap tokens /
  worker credentials already are), keeping secrets off the public surface.

## Password handling (argon2)

- The admin password is hashed with **argon2** (salted) and stored under a new store
  kind (e.g. `AdminAccount`: `username`, `passwordHash`).
- `Secret` / SHA-256 is **retained** for the high-entropy random tokens (bootstrap,
  worker credentials, admin session/CLI tokens) — unsalted SHA-256 is fine for
  high-entropy secrets, weak for human passwords; argon2 is used only where a human
  password is involved.

### New `velos-auth` surface

Added to the `AuthService` trait (or a sibling admin trait kept behind the same deep
module — implementation detail for the plan):

```rust
fn is_initialized(&self) -> Result<bool, AuthError>;
fn setup_admin(&self, username: &str, password: &Secret) -> Result<(), AuthError>; // fails closed if already initialized
fn verify_password(&self, username: &str, password: &Secret) -> Result<(), AuthError>;
fn mint_admin_session(&self, ttl_secs: i64) -> Result<String, AuthError>;          // UI session token
fn mint_cli_token(&self, label: &str, ttl_secs: i64) -> Result<MintedToken, AuthError>; // shown once
fn list_admin_tokens(&self) -> Result<Vec<AdminTokenInfo>, AuthError>;             // metadata only, never the secret
fn revoke_admin_token(&self, id: &str) -> Result<(), AuthError>;
```

`authenticate` is extended to return `Identity::Admin` for a valid admin token (session
or CLI) and continues to return `Identity::Worker` for worker credentials.

## apiserver endpoints

| Method & path | Auth | Purpose |
|---|---|---|
| `GET /auth/v1/status` | open (always) | `{ "initialized": bool }` — clients decide setup vs login |
| `POST /auth/v1/setup` | open **only while uninitialized** | Set admin username + password; **single-shot** (409 once initialized) |
| `POST /auth/v1/login` | open (post-init) | username + password → short-TTL **session token** |
| `GET /auth/v1/me` | any valid token | Echo caller identity (`velosctl login` uses it to validate a pasted token) |
| `GET /auth/v1/admin/tokens` | **Admin** | List CLI-token metadata (never secrets) |
| `POST /auth/v1/admin/tokens` | **Admin** | Create a **named CLI token** — secret returned exactly once |
| `DELETE /auth/v1/admin/tokens/:id` | **Admin** | Revoke a token |
| `POST /auth/v1/tokens` | **Admin** (was open — hole closed) | Mint a worker bootstrap token |
| `POST /auth/v1/register` | bootstrap token (unchanged) | Worker join → worker credential |
| `/api/v1/*` | Admin (full) or Worker (scoped) | Resource CRUD |

### Initialization gate (fail closed, Principle #6)

A guard in `require_auth` (and on the auth-management handlers): **while uninitialized,
only `GET /auth/v1/status` and `POST /auth/v1/setup` are reachable; every other route
returns 401** with a body indicating the server is not initialized. `/auth/v1/status`
is the authoritative discriminator.

This is reject-by-construction, not reject-by-accident: even without the gate an
uninitialized server has no valid tokens (no admin → no bootstrap mint → no worker
registration → no worker credentials), but the explicit gate makes the invariant a
type/flow property rather than an emergent coincidence.

### TokenVerifier seam (OIDC-ready)

`require_auth` resolves an identity through a `TokenVerifier` trait rather than calling
the store directly:

```rust
pub trait TokenVerifier: Send + Sync {
    fn verify(&self, presented: &str) -> Option<Identity>;
}
```

Today's implementation does the store lookup. A future OIDC verifier (validate a JWT
against a provider's JWKS, map claims → `Identity`) drops in with **no endpoint
changes**. Deep module (Principle #3): narrow interface, swappable implementation.

## velosctl

- `velosctl login --token <tok> [--server <url>]` → `GET /auth/v1/me` (against the
  resolved server) to validate, then save **both** `server` and `token` to
  `~/.velos/config` (mode **0600**). Refuses to save an invalid token (fail closed).
  After `login`, plain commands like `velosctl get containers` need no flags — the saved
  server **and** token are used.
- `velosctl logout` → remove the saved credential (clears token; server may be retained
  or cleared — cleared, for a clean slate).
- **Token resolution precedence:** `--token` flag > `VELOS_TOKEN` env > `~/.velos/config`.
- **Server resolution precedence:** `--server` flag > `VELOS_SERVER` env >
  `~/.velos/config` > built-in default (`http://127.0.0.1:8080`). The existing
  `--server` default moves to the end of this chain so a saved server takes effect.
- Config shape: `~/.velos/config` holds `{ "server": <url>, "token": <secret> }`.
- Existing `velosctl token create` (bootstrap mint) keeps working; it now requires admin
  auth and so resolves a token the same way.
- Pure config read/write/precedence logic lives in `velosctl::lib` (unit-tested);
  filesystem and reqwest wiring stay in `main.rs` (functional core / impure edge,
  Principle #5).

## Web UI (React)

Three additions to the existing dashboard:

1. **Setup screen** — shown when `GET /auth/v1/status` reports `initialized: false`.
   Collects admin username + password → `POST /auth/v1/setup`.
2. **Login screen** — username + password → `POST /auth/v1/login`; store the session
   token in browser storage and send it as a bearer on all API calls.
3. **Tokens page** — create a **named CLI token** (display the secret once, with a copy
   affordance and a "you won't see this again" warning), list existing tokens, revoke.

Unauthenticated API responses (401) → redirect to login or setup based on
`/auth/v1/status`.

## Error handling & fail-closed posture

- Setup is single-shot: a second `setup` while initialized → **409 Conflict**.
- Unknown / expired / revoked token → **401**.
- Admin-only endpoint with a worker (or no) token → **403 / 401**.
- Uninitialized server, non-setup route → **401** (initialization gate).
- Secrets never logged (`Secret` newtype); the CLI/UI token is shown **exactly once** at
  creation and only its hash is persisted.
- Bounded actions (e.g. token-list truncation, if any) logged with counts
  (no-silent-caps).

## Testing

**`velos-auth` unit tests** (alongside source):
- argon2 password round-trip; wrong password fails closed.
- `setup_admin` succeeds once, then fails closed (already initialized).
- `authenticate` resolves admin session/CLI tokens → `Identity::Admin`, worker
  credentials → `Identity::Worker`, garbage → `None`.
- Token expiry and revocation (post-revoke `authenticate` → `None`).

**apiserver tests** (`crates/apiserver`):
- Full happy path: `setup` → `login` → create CLI token → authenticated `/api/v1` as
  admin → revoke → subsequent call 401.
- Initialization gate: before `setup`, `/api/v1/*`, `/auth/v1/login`, `/auth/v1/tokens`,
  `/auth/v1/register` all return 401; `/auth/v1/status` and `/auth/v1/setup` succeed.
- `setup` twice → 409.
- Bootstrap mint (`POST /auth/v1/tokens`) now requires admin: worker/no token → 401/403.
- Worker scoping unchanged (regression).

**velosctl unit tests** (`crates/velosctl`):
- Config read/write round-trip (server + token); mode 0600.
- Token resolution precedence (flag > env > file), pure-function tested.
- Server resolution precedence (flag > env > file > default), pure-function tested.

**e2e** (`velos-tests`):
- Boot apiserver with admin auth, run a scripted setup → login → mint CLI token →
  velosctl uses it → worker bootstrap+register still works end to end.

## Scope / non-goals

- **Single admin account** only; multi-admin / per-user accounts are future work.
- **No RBAC** beyond the two-tier admin/worker split.
- **No external IdP / OIDC** implementation now — only the `TokenVerifier` seam.
- **No HttpOnly-cookie session** now — bearer-in-browser-storage; cookie is a documented
  future hardening.
- Password reset / rotation flows beyond the single-shot setup are future work.
