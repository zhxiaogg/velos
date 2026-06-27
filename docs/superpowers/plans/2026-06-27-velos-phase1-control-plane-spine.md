# Velos Phase 1 — Control-Plane Spine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a working single-node control plane that stores resources in SQLite and serves full CRUD over a Kubernetes-shaped REST API, with the fluorite-generated wire types in place.

**Architecture:** A Rust crate workspace. `velos-models` generates wire types from fluorite `.fl` schemas. `velos-store` persists objects as opaque JSON documents plus index columns behind a `Store` trait (SQLite impl). `velos-apiserver` is an axum service exposing CRUD + a status subresource + label/field selectors, treating objects as opaque JSON (typed admission is a later phase). Watch, controllers, the worker daemon, and auth are out of scope for this phase.

**Tech Stack:** Rust (edition 2024), tokio, axum 0.7, rusqlite (bundled SQLite), serde / serde_json, fluorite (codegen 0.6.1), uuid, chrono, thiserror, anyhow, tracing.

## Global Constraints

- Rust edition: **2024**; stable toolchain.
- Workspace clippy lints — **deny**: `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::wildcard_enum_match_arm`. Production code must not use `unwrap`/`expect`/`panic`. Test modules opt out with `#[allow(clippy::unwrap_used)]` (and friends) on the `#[cfg(test)] mod tests` block.
- Error handling: `thiserror` for library error types; `anyhow` in binaries and build scripts.
- **fluorite is for wire/protocol types only.** Never persist fluorite-generated structs as storage rows — the store uses its own hand-written `StoredObject`. (Principle #7 in `CLAUDE.md`.)
- All wire JSON is **camelCase** (fluorite emits `#[serde(rename_all = "camelCase")]`); `.fl` field names are written in **snake_case**.
- fluorite uses a single namespace: every type name must be unique project-wide.
- Unit tests live inline under `#[cfg(test)] mod tests`. Pre-PR gate is `make check` (`cargo fmt --check` + `cargo clippy --all-targets --all-features -D warnings` + `cargo test --workspace`).
- Crate/dependency versions are pinned exactly as written in each task so code compiles on the first try.

---

## File Structure

```
velos/ (repo root = /Users/xiaoguang/work/repo/personal/sandbox)
├── Cargo.toml                         # workspace: members, workspace.package, workspace.lints
├── Makefile                           # make check / build / test / fmt / clippy / run
├── rust-toolchain.toml                # stable
├── .gitignore                         # /target, *.db
├── CLAUDE.md                          # (already present) design principles
└── crates/
    ├── models/
    │   ├── Cargo.toml
    │   ├── build.rs                   # runs fluorite_codegen
    │   ├── fluorite/
    │   │   └── velos.fl               # all v1 resource wire types
    │   └── src/lib.rs                 # include! generated code + convenience ctors + round-trip test
    ├── store/
    │   ├── Cargo.toml
    │   └── src/lib.rs                 # Store trait, Selector, StoredObject, StoreError, SqliteStore
    └── apiserver/
        ├── Cargo.toml
        ├── src/lib.rs                 # Router builder, handlers, ApiError, selector parsing
        └── src/main.rs                # binary: bind TCP + axum::serve
```

Responsibilities:
- **`velos-models`** — the single source of truth for wire types; depends on nothing else in the workspace.
- **`velos-store`** — generic, schema-agnostic persistence; knows nothing about HTTP or specific resource kinds.
- **`velos-apiserver`** — HTTP surface; depends on `velos-store`. Treats objects as opaque `serde_json::Value`, extracting only the indexed envelope fields (`metadata.name`, `metadata.labels`, `spec.nodeName`).

---

## Task 1: Workspace scaffold, lints, and Makefile

**Files:**
- Create: `Cargo.toml`
- Create: `Makefile`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `crates/models/Cargo.toml`, `crates/models/src/lib.rs`
- Create: `crates/store/Cargo.toml`, `crates/store/src/lib.rs`
- Create: `crates/apiserver/Cargo.toml`, `crates/apiserver/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling 3-crate workspace with shared lints; later tasks fill in each crate.

This task has no behavior to test; its deliverable is "the empty workspace compiles and `make check` is green." Steps create the files, then verify build + check.

- [ ] **Step 1: Create the workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
authors = ["xiaoguang <zhxiaog@outlook.com>"]
repository = "https://github.com/zhxiaogg/velos"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
wildcard_enum_match_arm = "deny"
```

- [ ] **Step 2: Create `rust-toolchain.toml`, `.gitignore`, `Makefile`**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`.gitignore`:
```
/target
*.db
```

`Makefile` (note: real tab characters required for recipe lines):
```make
.PHONY: build test fmt clippy check run

build:
	cargo build --workspace

test:
	cargo test --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --workspace

run:
	cargo run -p velos-apiserver
```

- [ ] **Step 3: Create the three crate stubs**

`crates/models/Cargo.toml`:
```toml
[package]
name = "velos-models"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]

[build-dependencies]
```

`crates/models/src/lib.rs`:
```rust
// fluorite-generated wire types are added in Task 2.
```

`crates/store/Cargo.toml`:
```toml
[package]
name = "velos-store"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
```

`crates/store/src/lib.rs`:
```rust
// Store trait and SQLite implementation are added in Tasks 3-4.
```

`crates/apiserver/Cargo.toml`:
```toml
[package]
name = "velos-apiserver"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
```

`crates/apiserver/src/lib.rs`:
```rust
// HTTP surface is added in Tasks 5-7.
```

- [ ] **Step 4: Verify the workspace builds and `make check` passes**

Run: `cargo build --workspace`
Expected: compiles with no errors (warnings about empty crates are acceptable).

Run: `make check`
Expected: `fmt --check` passes, `clippy` passes with no warnings, `cargo test` reports `0 tests` across the workspace and exits 0.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Makefile rust-toolchain.toml .gitignore crates/
git commit -m "scaffold: velos workspace with shared lints and three crates"
```

---

## Task 2: `velos-models` — fluorite wire types + round-trip test

**Files:**
- Modify: `crates/models/Cargo.toml`
- Create: `crates/models/build.rs`
- Create: `crates/models/fluorite/velos.fl`
- Modify: `crates/models/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: module `velos_models::velos` containing `ObjectMeta`, `Container`, `ContainerSpec`, `ContainerStatus`, `ContainerPhase`, `RestartPolicy`, `ResourceReqs`, `Worker`, `WorkerSpec`, `WorkerStatus`, `NodeResources`, `NodeCondition`, `NodeConditionType`, `Lease`, `LeaseSpec`. Each struct has a `::new(...)` constructor (positional, field order) from `derive_new`. Convenience constructors: `ObjectMeta::new_named(name) -> ObjectMeta`, `Container::pending(name, image) -> Container`.

- [ ] **Step 1: Write the failing test**

Append to `crates/models/src/lib.rs`:
```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::velos::*;

    #[test]
    fn container_round_trips_in_camel_case() {
        let c = Container::pending("c1", "alpine:latest");

        let json = serde_json::to_string(&c).unwrap();
        // camelCase keys, proving fluorite's rename_all is applied
        assert!(json.contains("\"resourceVersion\""), "json was: {json}");
        assert!(json.contains("\"creationTimestamp\""), "json was: {json}");
        assert!(json.contains("\"restartPolicy\""), "json was: {json}");

        let back: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p velos-models`
Expected: FAIL — compile error, `velos` module and `Container` do not exist yet.

- [ ] **Step 3: Add dependencies, build script, schema, and includes**

`crates/models/Cargo.toml`:
```toml
[package]
name = "velos-models"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
serde = { version = "1.0", features = ["serde_derive"] }
serde_json = "1.0"
derive-new = "0.7"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }

[build-dependencies]
fluorite_codegen = "0.6.1"
anyhow = "1.0"
```

`crates/models/build.rs`:
```rust
use fluorite_codegen::code_gen::rust::RustOptions;

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=fluorite");
    let out_dir = std::env::var("OUT_DIR")?;
    let options = RustOptions::new(out_dir).with_any_type("serde_json::Value");
    fluorite_codegen::compile_with_options(options, &["fluorite/"])?;
    Ok(())
}
```

`crates/models/fluorite/velos.fl` (note: `.fl` fields are snake_case; JSON becomes camelCase):
```rust
package velos;

struct ObjectMeta {
    name: String,
    uid: Uuid,
    labels: Map<String, String>,
    annotations: Map<String, String>,
    resource_version: u64,
    creation_timestamp: DateTimeUtc,
    deletion_timestamp: Option<DateTimeUtc>,
    finalizers: Vec<String>,
}

enum ContainerPhase {
    Pending,
    Scheduled,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

struct ResourceReqs {
    cpu: u32,
    memory_bytes: u64,
}

struct ContainerSpec {
    image: String,
    command: Vec<String>,
    env: Map<String, String>,
    resources: ResourceReqs,
    restart_policy: RestartPolicy,
    node_name: Option<String>,
}

struct ContainerStatus {
    phase: ContainerPhase,
    worker_name: Option<String>,
    container_id: Option<String>,
    started_at: Option<DateTimeUtc>,
    finished_at: Option<DateTimeUtc>,
    exit_code: Option<i32>,
    message: Option<String>,
}

struct Container {
    metadata: ObjectMeta,
    spec: ContainerSpec,
    status: ContainerStatus,
}

struct NodeResources {
    cpu: u32,
    memory_bytes: u64,
    max_containers: u32,
}

enum NodeConditionType {
    Ready,
}

struct NodeCondition {
    condition_type: NodeConditionType,
    status: bool,
    last_transition_time: DateTimeUtc,
    reason: String,
}

struct WorkerSpec {
    unschedulable: bool,
}

struct WorkerStatus {
    capacity: NodeResources,
    allocatable: NodeResources,
    conditions: Vec<NodeCondition>,
    addresses: Vec<String>,
    container_runtime_version: String,
}

struct Worker {
    metadata: ObjectMeta,
    spec: WorkerSpec,
    status: WorkerStatus,
}

struct LeaseSpec {
    holder_identity: String,
    renew_time: DateTimeUtc,
    lease_duration_seconds: u32,
}

struct Lease {
    metadata: ObjectMeta,
    spec: LeaseSpec,
}
```

Replace `crates/models/src/lib.rs` (keep the test module from Step 1 at the end):
```rust
//! Velos wire types, generated from `fluorite/velos.fl`.

pub mod velos {
    include!(concat!(env!("OUT_DIR"), "/velos/mod.rs"));
}

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use velos::{
    Container, ContainerPhase, ContainerSpec, ContainerStatus, ObjectMeta, ResourceReqs,
    RestartPolicy,
};

impl ObjectMeta {
    /// A fresh `ObjectMeta` with a random uid, resource_version 0, and now() timestamp.
    pub fn new_named(name: impl Into<String>) -> Self {
        ObjectMeta::new(
            name.into(),
            Uuid::new_v4(),
            HashMap::new(),
            HashMap::new(),
            0,
            Utc::now(),
            None,
            Vec::new(),
        )
    }
}

impl Container {
    /// A minimal `Pending` container for the given image.
    pub fn pending(name: impl Into<String>, image: impl Into<String>) -> Self {
        Container::new(
            ObjectMeta::new_named(name),
            ContainerSpec::new(
                image.into(),
                Vec::new(),
                HashMap::new(),
                ResourceReqs::new(1, 512 * 1024 * 1024),
                RestartPolicy::Never,
                None,
            ),
            ContainerStatus::new(ContainerPhase::Pending, None, None, None, None, None, None),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::velos::*;

    #[test]
    fn container_round_trips_in_camel_case() {
        let c = Container::pending("c1", "alpine:latest");

        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"resourceVersion\""), "json was: {json}");
        assert!(json.contains("\"creationTimestamp\""), "json was: {json}");
        assert!(json.contains("\"restartPolicy\""), "json was: {json}");

        let back: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p velos-models`
Expected: PASS — `container_round_trips_in_camel_case` ok. If fluorite emits an unexpected constructor arity, adjust the `::new(...)` argument lists to match the generated field order (the order is exactly the `.fl` field order).

- [ ] **Step 5: Commit**

```bash
git add crates/models/
git commit -m "feat(models): fluorite wire types for Container/Worker/Lease with round-trip test"
```

---

## Task 3: `velos-store` — Store trait, types, and `SqliteStore` create/get

**Files:**
- Modify: `crates/store/Cargo.toml`
- Modify: `crates/store/src/lib.rs`

**Interfaces:**
- Consumes: nothing (storage type is hand-written, not fluorite).
- Produces:
  - `struct StoredObject { kind: String, name: String, uid: uuid::Uuid, resource_version: u64, node_name: Option<String>, labels: std::collections::HashMap<String, String>, document: serde_json::Value }`
  - `struct Selector { labels: Vec<(String, String)>, node_name: Option<String> }` (derives `Default`)
  - `enum StoreError` (`thiserror`)
  - `trait Store: Send + Sync` with `next_resource_version(&self) -> Result<u64, StoreError>`, `put(&self, obj: &StoredObject) -> Result<(), StoreError>`, `get(&self, kind: &str, name: &str) -> Result<Option<StoredObject>, StoreError>`, `list(&self, kind: &str, selector: &Selector) -> Result<Vec<StoredObject>, StoreError>`, `delete(&self, kind: &str, name: &str) -> Result<bool, StoreError>`
  - `struct SqliteStore` with `open(path: &str) -> Result<Self, StoreError>` and `in_memory() -> Result<Self, StoreError>`, implementing `Store`.

This task implements everything except `list`/`delete` behavior, which Task 4 tests. (The trait declares all five methods here; `list`/`delete` get their dedicated tests in Task 4.)

- [ ] **Step 1: Write the failing test**

Append to `crates/store/src/lib.rs`:
```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn obj(kind: &str, name: &str, rv: u64) -> StoredObject {
        StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: rv,
            node_name: None,
            labels: HashMap::new(),
            document: serde_json::json!({ "metadata": { "name": name } }),
        }
    }

    #[test]
    fn resource_version_is_monotonic() {
        let s = SqliteStore::in_memory().unwrap();
        assert_eq!(s.next_resource_version().unwrap(), 1);
        assert_eq!(s.next_resource_version().unwrap(), 2);
        assert_eq!(s.next_resource_version().unwrap(), 3);
    }

    #[test]
    fn put_then_get_round_trips() {
        let s = SqliteStore::in_memory().unwrap();
        let o = obj("Container", "c1", 7);
        s.put(&o).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got, o);
        assert!(s.get("Container", "missing").unwrap().is_none());
    }

    #[test]
    fn put_upserts_on_same_kind_and_name() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj("Container", "c1", 1)).unwrap();
        let mut updated = obj("Container", "c1", 2);
        updated.document = serde_json::json!({ "metadata": { "name": "c1" }, "v": 2 });
        s.put(&updated).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got.resource_version, 2);
        assert_eq!(got.document["v"], 2);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p velos-store`
Expected: FAIL — compile error, `SqliteStore`/`StoredObject` not defined.

- [ ] **Step 3: Add dependencies and implement the store**

`crates/store/Cargo.toml`:
```toml
[package]
name = "velos-store"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
serde_json = "1.0"
thiserror = "1.0"
uuid = { version = "1", features = ["v4"] }
```

Replace `crates/store/src/lib.rs` (keep the Step 1 test module at the end):
```rust
//! Generic, schema-agnostic persistence for Velos objects.
//!
//! Objects are stored as opaque JSON documents plus index columns
//! (`name`, `uid`, `resource_version`, `node_name`, `labels`). The store knows
//! nothing about specific resource kinds. (Principle #7: storage != protocol.)

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("stored uid is not a valid uuid: {0}")]
    Uid(String),
    #[error("store lock poisoned")]
    Lock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredObject {
    pub kind: String,
    pub name: String,
    pub uid: Uuid,
    pub resource_version: u64,
    pub node_name: Option<String>,
    pub labels: HashMap<String, String>,
    pub document: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Selector {
    /// Equality label matches (all must hold).
    pub labels: Vec<(String, String)>,
    /// Field selector on `spec.nodeName`.
    pub node_name: Option<String>,
}

pub trait Store: Send + Sync {
    fn next_resource_version(&self) -> Result<u64, StoreError>;
    fn put(&self, obj: &StoredObject) -> Result<(), StoreError>;
    fn get(&self, kind: &str, name: &str) -> Result<Option<StoredObject>, StoreError>;
    fn list(&self, kind: &str, selector: &Selector) -> Result<Vec<StoredObject>, StoreError>;
    fn delete(&self, kind: &str, name: &str) -> Result<bool, StoreError>;
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn init(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS objects (
                kind             TEXT NOT NULL,
                name             TEXT NOT NULL,
                uid              TEXT NOT NULL,
                resource_version INTEGER NOT NULL,
                node_name        TEXT,
                labels           TEXT NOT NULL,
                document         TEXT NOT NULL,
                PRIMARY KEY (kind, name)
            );
            CREATE TABLE IF NOT EXISTS rv_seq (
                id    INTEGER PRIMARY KEY CHECK (id = 0),
                value INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO rv_seq (id, value) VALUES (0, 0);",
        )?;
        Ok(())
    }

    fn parse_uid(s: &str) -> Result<Uuid, StoreError> {
        Uuid::parse_str(s).map_err(|_| StoreError::Uid(s.to_string()))
    }
}

impl Store for SqliteStore {
    fn next_resource_version(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        conn.execute("UPDATE rv_seq SET value = value + 1 WHERE id = 0", [])?;
        let v: i64 = conn.query_row("SELECT value FROM rv_seq WHERE id = 0", [], |r| r.get(0))?;
        Ok(v as u64)
    }

    fn put(&self, obj: &StoredObject) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let labels = serde_json::to_string(&obj.labels)?;
        let document = serde_json::to_string(&obj.document)?;
        conn.execute(
            "INSERT INTO objects (kind, name, uid, resource_version, node_name, labels, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(kind, name) DO UPDATE SET
                uid              = excluded.uid,
                resource_version = excluded.resource_version,
                node_name        = excluded.node_name,
                labels           = excluded.labels,
                document         = excluded.document",
            rusqlite::params![
                obj.kind,
                obj.name,
                obj.uid.to_string(),
                obj.resource_version as i64,
                obj.node_name,
                labels,
                document,
            ],
        )?;
        Ok(())
    }

    fn get(&self, kind: &str, name: &str) -> Result<Option<StoredObject>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT uid, resource_version, node_name, labels, document
             FROM objects WHERE kind = ?1 AND name = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![kind, name])?;
        match rows.next()? {
            Some(row) => {
                let uid_s: String = row.get(0)?;
                let rv: i64 = row.get(1)?;
                let node_name: Option<String> = row.get(2)?;
                let labels_s: String = row.get(3)?;
                let document_s: String = row.get(4)?;
                Ok(Some(StoredObject {
                    kind: kind.to_string(),
                    name: name.to_string(),
                    uid: Self::parse_uid(&uid_s)?,
                    resource_version: rv as u64,
                    node_name,
                    labels: serde_json::from_str(&labels_s)?,
                    document: serde_json::from_str(&document_s)?,
                }))
            }
            None => Ok(None),
        }
    }

    fn list(&self, kind: &str, selector: &Selector) -> Result<Vec<StoredObject>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT name, uid, resource_version, node_name, labels, document
             FROM objects WHERE kind = ?1 ORDER BY name",
        )?;
        let raw = stmt.query_map(rusqlite::params![kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in raw {
            let (name, uid_s, rv, node_name, labels_s, document_s) = r?;
            if let Some(want) = &selector.node_name {
                if node_name.as_deref() != Some(want.as_str()) {
                    continue;
                }
            }
            let labels: HashMap<String, String> = serde_json::from_str(&labels_s)?;
            let matches = selector
                .labels
                .iter()
                .all(|(k, v)| labels.get(k).map(|x| x == v).unwrap_or(false));
            if !matches {
                continue;
            }
            out.push(StoredObject {
                kind: kind.to_string(),
                name,
                uid: Self::parse_uid(&uid_s)?,
                resource_version: rv as u64,
                node_name,
                labels,
                document: serde_json::from_str(&document_s)?,
            });
        }
        Ok(out)
    }

    fn delete(&self, kind: &str, name: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let n = conn.execute(
            "DELETE FROM objects WHERE kind = ?1 AND name = ?2",
            rusqlite::params![kind, name],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn obj(kind: &str, name: &str, rv: u64) -> StoredObject {
        StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: rv,
            node_name: None,
            labels: HashMap::new(),
            document: serde_json::json!({ "metadata": { "name": name } }),
        }
    }

    #[test]
    fn resource_version_is_monotonic() {
        let s = SqliteStore::in_memory().unwrap();
        assert_eq!(s.next_resource_version().unwrap(), 1);
        assert_eq!(s.next_resource_version().unwrap(), 2);
        assert_eq!(s.next_resource_version().unwrap(), 3);
    }

    #[test]
    fn put_then_get_round_trips() {
        let s = SqliteStore::in_memory().unwrap();
        let o = obj("Container", "c1", 7);
        s.put(&o).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got, o);
        assert!(s.get("Container", "missing").unwrap().is_none());
    }

    #[test]
    fn put_upserts_on_same_kind_and_name() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj("Container", "c1", 1)).unwrap();
        let mut updated = obj("Container", "c1", 2);
        updated.document = serde_json::json!({ "metadata": { "name": "c1" }, "v": 2 });
        s.put(&updated).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got.resource_version, 2);
        assert_eq!(got.document["v"], 2);
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p velos-store`
Expected: PASS — 3 tests ok.

- [ ] **Step 5: Commit**

```bash
git add crates/store/
git commit -m "feat(store): Store trait + SqliteStore with monotonic resourceVersion, put/get"
```

---

## Task 4: `velos-store` — `list` selectors and `delete`

**Files:**
- Modify: `crates/store/src/lib.rs` (add tests only; `list`/`delete` were implemented in Task 3)

**Interfaces:**
- Consumes: `Store`, `SqliteStore`, `Selector`, `StoredObject` from Task 3.
- Produces: verified `list` (kind filter + label equality + `node_name` field filter) and `delete` behavior.

- [ ] **Step 1: Write the failing test**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `crates/store/src/lib.rs`:
```rust
    fn obj_with(kind: &str, name: &str, node: Option<&str>, labels: &[(&str, &str)]) -> StoredObject {
        StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: 1,
            node_name: node.map(|s| s.to_string()),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            document: serde_json::json!({ "metadata": { "name": name } }),
        }
    }

    #[test]
    fn list_filters_by_kind() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[])).unwrap();
        s.put(&obj_with("Container", "c2", None, &[])).unwrap();
        s.put(&obj_with("Worker", "w1", None, &[])).unwrap();

        let containers = s.list("Container", &Selector::default()).unwrap();
        assert_eq!(containers.len(), 2);
        let workers = s.list("Worker", &Selector::default()).unwrap();
        assert_eq!(workers.len(), 1);
    }

    #[test]
    fn list_filters_by_label_equality() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[("team", "a")])).unwrap();
        s.put(&obj_with("Container", "c2", None, &[("team", "b")])).unwrap();

        let sel = Selector { labels: vec![("team".into(), "a".into())], node_name: None };
        let got = s.list("Container", &sel).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "c1");
    }

    #[test]
    fn list_filters_by_node_name() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", Some("node-7"), &[])).unwrap();
        s.put(&obj_with("Container", "c2", Some("node-8"), &[])).unwrap();
        s.put(&obj_with("Container", "c3", None, &[])).unwrap();

        let sel = Selector { labels: vec![], node_name: Some("node-7".into()) };
        let got = s.list("Container", &sel).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "c1");
    }

    #[test]
    fn delete_removes_and_reports() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[])).unwrap();
        assert!(s.delete("Container", "c1").unwrap());
        assert!(!s.delete("Container", "c1").unwrap());
        assert!(s.get("Container", "c1").unwrap().is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails (or passes if implementation is complete)**

Run: `cargo test -p velos-store`
Expected: PASS — `list`/`delete` were already implemented in Task 3, so the new tests should pass immediately. If any fail, fix the `list`/`delete` implementation (not the tests) until green. (TDD note: these tests pin behavior that Task 3's implementation must satisfy; treat a failure as a Task 3 bug.)

- [ ] **Step 3: (No new implementation expected)**

If green, skip. If red, correct `list`/`delete` in `impl Store for SqliteStore`.

- [ ] **Step 4: Run the full crate test suite**

Run: `cargo test -p velos-store`
Expected: PASS — 7 tests ok.

- [ ] **Step 5: Commit**

```bash
git add crates/store/
git commit -m "test(store): cover list selectors (kind/label/nodeName) and delete"
```

---

## Task 5: `velos-apiserver` — create + get handlers and router

**Files:**
- Modify: `crates/apiserver/Cargo.toml`
- Modify: `crates/apiserver/src/lib.rs`
- Create: `crates/apiserver/src/main.rs`

**Interfaces:**
- Consumes: `velos_store::{Store, SqliteStore, StoredObject, Selector, StoreError}`.
- Produces:
  - `pub fn app(store: std::sync::Arc<dyn Store>) -> axum::Router`
  - `pub enum ApiError` implementing `IntoResponse`
  - Internal helpers `kind_for`, `extract_name`, `extract_labels`, `extract_node_name`, `stamp_meta`, `parse_selector` (used by later tasks).
  - Routes: `POST /api/v1/{plural}` (create → 201), `GET /api/v1/{plural}/{name}` (read one → 200/404). (`GET` collection, `PUT`, `PUT /status`, `DELETE` are added in Tasks 6-7.)

- [ ] **Step 1: Write the failing test**

Append to `crates/apiserver/src/lib.rs`:
```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_app() -> axum::Router {
        let store = Arc::new(velos_store::SqliteStore::in_memory().unwrap());
        app(store)
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_assigns_uid_and_resource_version_then_get_returns_it() {
        let app = test_app();
        let body = serde_json::json!({
            "metadata": { "name": "c1" },
            "spec": { "image": "alpine:latest" }
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/containers")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp).await;
        assert_eq!(created["metadata"]["resourceVersion"], 1);
        assert!(created["metadata"]["uid"].is_string());

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/containers/c1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let got = body_json(resp).await;
        assert_eq!(got["metadata"]["name"], "c1");
        assert_eq!(got["spec"]["image"], "alpine:latest");
    }

    #[tokio::test]
    async fn create_without_name_is_bad_request() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/containers")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "spec": {} }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_unknown_kind_is_not_found() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/widgets/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p velos-apiserver`
Expected: FAIL — compile error, `app`/`ApiError` not defined.

- [ ] **Step 3: Add dependencies and implement create/get + router**

`crates/apiserver/Cargo.toml`:
```toml
[package]
name = "velos-apiserver"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[lib]
name = "velos_apiserver"
path = "src/lib.rs"

[[bin]]
name = "velos-apiserver"
path = "src/main.rs"

[dependencies]
velos-store = { path = "../store" }
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde_json = "1.0"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4" }
thiserror = "1.0"
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
```

Replace `crates/apiserver/src/lib.rs` (keep the Step 1 test module at the end):
```rust
//! Velos API server: a Kubernetes-shaped REST surface over `velos_store`.
//!
//! Objects are handled as opaque JSON; only the indexed envelope fields
//! (`metadata.name`, `metadata.labels`, `spec.nodeName`) are interpreted.
//! Typed admission against `velos-models` is a later phase.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use uuid::Uuid;

use velos_store::{Selector, Store, StoredObject};

#[derive(Clone)]
struct AppState {
    store: Arc<dyn Store>,
}

pub enum ApiError {
    NotFound,
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<velos_store::StoreError> for ApiError {
    fn from(e: velos_store::StoreError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

/// Maps a plural resource path segment to its stored kind.
fn kind_for(plural: &str) -> Option<&'static str> {
    match plural {
        "containers" => Some("Container"),
        "workers" => Some("Worker"),
        "leases" => Some("Lease"),
        _ => None,
    }
}

fn extract_name(doc: &Value) -> Option<String> {
    doc.get("metadata")?.get("name")?.as_str().map(str::to_string)
}

fn extract_labels(doc: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(map) = doc
        .get("metadata")
        .and_then(|m| m.get("labels"))
        .and_then(Value::as_object)
    {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    out
}

fn extract_node_name(doc: &Value) -> Option<String> {
    doc.get("spec")?.get("nodeName")?.as_str().map(str::to_string)
}

/// Stamps server-owned envelope fields into a (mutable) object document.
fn stamp_meta(doc: &mut Value, uid: &Uuid, rv: u64) {
    if !doc.get("metadata").map(Value::is_object).unwrap_or(false) {
        doc["metadata"] = serde_json::json!({});
    }
    if let Some(m) = doc.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("uid".to_string(), serde_json::json!(uid.to_string()));
        m.insert("resourceVersion".to_string(), serde_json::json!(rv));
        m.entry("creationTimestamp")
            .or_insert_with(|| serde_json::json!(chrono::Utc::now().to_rfc3339()));
        m.entry("labels").or_insert_with(|| serde_json::json!({}));
        m.entry("annotations").or_insert_with(|| serde_json::json!({}));
        m.entry("finalizers").or_insert_with(|| serde_json::json!([]));
    }
}

fn parse_selector(params: &HashMap<String, String>) -> Result<Selector, ApiError> {
    let mut sel = Selector::default();
    if let Some(ls) = params.get("labelSelector") {
        for pair in ls.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| ApiError::BadRequest(format!("bad labelSelector: {pair}")))?;
            sel.labels.push((k.to_string(), v.to_string()));
        }
    }
    if let Some(fs) = params.get("fieldSelector") {
        for pair in fs.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| ApiError::BadRequest(format!("bad fieldSelector: {pair}")))?;
            if k == "spec.nodeName" {
                sel.node_name = Some(v.to_string());
            } else {
                return Err(ApiError::BadRequest(format!("unsupported fieldSelector: {k}")));
            }
        }
    }
    Ok(sel)
}

async fn create(
    State(state): State<AppState>,
    Path(plural): Path<String>,
    Json(mut body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if !body.is_object() {
        return Err(ApiError::BadRequest("body must be a JSON object".into()));
    }
    let name =
        extract_name(&body).ok_or_else(|| ApiError::BadRequest("metadata.name required".into()))?;
    let uid = Uuid::new_v4();
    let rv = state.store.next_resource_version()?;
    stamp_meta(&mut body, &uid, rv);

    let obj = StoredObject {
        kind: kind.to_string(),
        name,
        uid,
        resource_version: rv,
        node_name: extract_node_name(&body),
        labels: extract_labels(&body),
        document: body.clone(),
    };
    state.store.put(&obj)?;
    Ok((StatusCode::CREATED, Json(body)))
}

async fn get_one(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let obj = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    Ok(Json(obj.document))
}

pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/v1/:plural", post(create))
        .route("/api/v1/:plural/:name", get(get_one))
        .with_state(state)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    // (test module from Step 1 goes here)
}
```

Create `crates/apiserver/src/main.rs`:
```rust
use std::sync::Arc;

use velos_store::SqliteStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let store = Arc::new(SqliteStore::open("velos.db")?);
    let app = velos_apiserver::app(store);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("velos-apiserver listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

Make sure the test module from Step 1 replaces the placeholder `mod tests` block.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p velos-apiserver`
Expected: PASS — 3 tests ok.

- [ ] **Step 5: Commit**

```bash
git add crates/apiserver/
git commit -m "feat(apiserver): create + get handlers, router, binary entrypoint"
```

---

## Task 6: `velos-apiserver` — list collection with selectors

**Files:**
- Modify: `crates/apiserver/src/lib.rs`

**Interfaces:**
- Consumes: `parse_selector`, `kind_for`, `AppState` from Task 5.
- Produces: `GET /api/v1/{plural}` returning `{ "items": [ ...documents ] }`, honoring `?labelSelector=` and `?fieldSelector=spec.nodeName=`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/apiserver/src/lib.rs`:
```rust
    async fn post(app: &axum::Router, plural: &str, body: serde_json::Value) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/{plural}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn list_returns_items_and_honors_selectors() {
        let app = test_app();
        post(&app, "containers", serde_json::json!({
            "metadata": { "name": "c1", "labels": { "team": "a" } },
            "spec": { "image": "img", "nodeName": "node-7" }
        })).await;
        post(&app, "containers", serde_json::json!({
            "metadata": { "name": "c2", "labels": { "team": "b" } },
            "spec": { "image": "img", "nodeName": "node-8" }
        })).await;

        // list all
        let resp = app.clone().oneshot(
            Request::builder().method("GET").uri("/api/v1/containers").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let all = body_json(resp).await;
        assert_eq!(all["items"].as_array().unwrap().len(), 2);

        // label selector
        let resp = app.clone().oneshot(
            Request::builder().method("GET").uri("/api/v1/containers?labelSelector=team=a").body(Body::empty()).unwrap()
        ).await.unwrap();
        let filtered = body_json(resp).await;
        assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["items"][0]["metadata"]["name"], "c1");

        // field selector
        let resp = app.oneshot(
            Request::builder().method("GET").uri("/api/v1/containers?fieldSelector=spec.nodeName=node-8").body(Body::empty()).unwrap()
        ).await.unwrap();
        let by_node = body_json(resp).await;
        assert_eq!(by_node["items"].as_array().unwrap().len(), 1);
        assert_eq!(by_node["items"][0]["metadata"]["name"], "c2");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p velos-apiserver list_returns_items_and_honors_selectors`
Expected: FAIL — `GET /api/v1/:plural` (collection) is not routed yet (returns 405/404).

- [ ] **Step 3: Implement the list handler and route it**

Add the handler to `crates/apiserver/src/lib.rs`:
```rust
async fn list(
    State(state): State<AppState>,
    Path(plural): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let selector = parse_selector(&params)?;
    let objs = state.store.list(kind, &selector)?;
    let items: Vec<Value> = objs.into_iter().map(|o| o.document).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}
```

Update the router so the collection path serves both POST and GET:
```rust
pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/v1/:plural", post(create).get(list))
        .route("/api/v1/:plural/:name", get(get_one))
        .with_state(state)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p velos-apiserver`
Expected: PASS — all apiserver tests ok.

- [ ] **Step 5: Commit**

```bash
git add crates/apiserver/
git commit -m "feat(apiserver): list collection with label/field selectors"
```

---

## Task 7: `velos-apiserver` — replace, status subresource, delete

**Files:**
- Modify: `crates/apiserver/src/lib.rs`

**Interfaces:**
- Consumes: handlers/state/helpers from Tasks 5-6.
- Produces:
  - `PUT /api/v1/{plural}/{name}` — replace spec/metadata, preserve `uid` + `creationTimestamp`, bump `resourceVersion` (200).
  - `PUT /api/v1/{plural}/{name}/status` — replace only `status`, bump `resourceVersion`, leave spec/metadata otherwise intact (200).
  - `DELETE /api/v1/{plural}/{name}` — remove (204) or 404.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block:
```rust
    #[tokio::test]
    async fn replace_status_and_delete_lifecycle() {
        let app = test_app();
        post(&app, "containers", serde_json::json!({
            "metadata": { "name": "c1" },
            "spec": { "image": "img" }
        })).await;

        // PUT status subresource
        let resp = app.clone().oneshot(
            Request::builder().method("PUT").uri("/api/v1/containers/c1/status")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "status": { "phase": "Running" } }).to_string()))
                .unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let after = body_json(resp).await;
        assert_eq!(after["status"]["phase"], "Running");
        assert_eq!(after["spec"]["image"], "img"); // spec preserved
        assert_eq!(after["metadata"]["resourceVersion"], 2); // bumped

        // PUT replace whole object
        let resp = app.clone().oneshot(
            Request::builder().method("PUT").uri("/api/v1/containers/c1")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({
                    "metadata": { "name": "c1" },
                    "spec": { "image": "img2" }
                }).to_string()))
                .unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let replaced = body_json(resp).await;
        assert_eq!(replaced["spec"]["image"], "img2");
        assert_eq!(replaced["metadata"]["resourceVersion"], 3);

        // DELETE
        let resp = app.clone().oneshot(
            Request::builder().method("DELETE").uri("/api/v1/containers/c1").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // DELETE again → 404
        let resp = app.oneshot(
            Request::builder().method("DELETE").uri("/api/v1/containers/c1").body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p velos-apiserver replace_status_and_delete_lifecycle`
Expected: FAIL — PUT/DELETE routes not present.

- [ ] **Step 3: Implement replace, status, delete and route them**

Add handlers to `crates/apiserver/src/lib.rs`:
```rust
async fn replace(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if !body.is_object() {
        return Err(ApiError::BadRequest("body must be a JSON object".into()));
    }
    let existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    let rv = state.store.next_resource_version()?;
    stamp_meta(&mut body, &existing.uid, rv);

    // Force name to match the path and preserve the original creationTimestamp.
    if let Some(m) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("name".to_string(), serde_json::json!(name));
        if let Some(ct) = existing
            .document
            .get("metadata")
            .and_then(|x| x.get("creationTimestamp"))
        {
            m.insert("creationTimestamp".to_string(), ct.clone());
        }
    }

    let obj = StoredObject {
        kind: kind.to_string(),
        name: name.clone(),
        uid: existing.uid,
        resource_version: rv,
        node_name: extract_node_name(&body),
        labels: extract_labels(&body),
        document: body.clone(),
    };
    state.store.put(&obj)?;
    Ok(Json(body))
}

async fn replace_status(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    let mut existing = state.store.get(kind, &name)?.ok_or(ApiError::NotFound)?;
    // Accept either `{ "status": {...} }` or a bare status object.
    let new_status = body.get("status").cloned().unwrap_or(body);
    let rv = state.store.next_resource_version()?;

    if let Some(m) = existing.document.as_object_mut() {
        m.insert("status".to_string(), new_status);
    }
    if let Some(m) = existing
        .document
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
    {
        m.insert("resourceVersion".to_string(), serde_json::json!(rv));
    }
    existing.resource_version = rv;
    state.store.put(&existing)?;
    Ok(Json(existing.document))
}

async fn delete(
    State(state): State<AppState>,
    Path((plural, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let kind = kind_for(&plural).ok_or(ApiError::NotFound)?;
    if state.store.delete(kind, &name)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
```

Update imports and the router. Change the routing import line to include `put`:
```rust
use axum::routing::{delete as delete_route, get, post, put};
```
And the router:
```rust
pub fn app(store: Arc<dyn Store>) -> Router {
    let state = AppState { store };
    Router::new()
        .route("/api/v1/:plural", post(create).get(list))
        .route(
            "/api/v1/:plural/:name",
            get(get_one).put(replace).delete(delete_route),
        )
        .route("/api/v1/:plural/:name/status", put(replace_status))
        .with_state(state)
}
```
(The local handler `delete` is imported into scope as the function; the routing combinator is aliased to `delete_route` to avoid the name clash.)

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS — models, store, and apiserver tests all green.

- [ ] **Step 5: Run the full pre-PR gate and commit**

Run: `make check`
Expected: fmt clean, clippy clean (`-D warnings`), all tests pass.

```bash
git add crates/apiserver/
git commit -m "feat(apiserver): replace, status subresource, and delete"
```

---

## Phase 1 Self-Review

**Spec coverage (against `docs/superpowers/specs/2026-06-27-velos-design.md`):**
- Single-node control plane + embedded SQLite → Tasks 3-5 (`SqliteStore`, `app`, `main`). ✓
- fluorite wire types, camelCase, schemars-free for now → Task 2. ✓ (schemars/OpenAPI deferred to a later phase — noted below.)
- Opaque-document + index-column store (protocol ≠ storage) → Task 3 `StoredObject`. ✓
- REST CRUD + status subresource + label/field selectors → Tasks 5-7. ✓
- Watch endpoint, scheduler/controllers, leases → **Phase 2** (out of scope here). ✓ deferred intentionally.
- `veloslet` + `ContainerRuntime` → **Phase 3**. Auth/registration → **Phase 4**. `velosctl` → **Phase 5**. ✓ deferred.

**Intentional deferrals (carried to later phases, not gaps):**
- `schemars::JsonSchema` derive + `GET /openapi.json` — deferred to keep Phase 1's dependency surface minimal; add the derive via `RustOptions::with_additional_derives` and the `schemars` features for `chrono`/`uuid` when building the OpenAPI endpoint.
- Optimistic concurrency (`resourceVersion` precondition → 409) — Phase 1 is last-write-wins; add `expected_version` to `Store::put` in Phase 2 alongside watch.
- Typed admission (deserialize into `velos-models` types and validate) — Phase 1 stores opaque JSON; wire `velos-models` into admission in a later phase.

**Placeholder scan:** none — every step contains complete, runnable code and exact commands.

**Type consistency:** `Store` method names (`next_resource_version`, `put`, `get`, `list`, `delete`), `StoredObject`/`Selector` field names, and the `app(store: Arc<dyn Store>)` signature are used identically across Tasks 3-7. Handler helper names (`kind_for`, `extract_name`, `extract_labels`, `extract_node_name`, `stamp_meta`, `parse_selector`) are defined in Task 5 and reused unchanged in Tasks 6-7.
