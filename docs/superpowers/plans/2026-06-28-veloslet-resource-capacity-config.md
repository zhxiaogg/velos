# Veloslet Resource Capacity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the worker's CPU/memory capacity required configuration validated against the real machine, and remove `maxContainers` entirely.

**Architecture:** Add a `Memory` semantic type and a host-detection/validation pair to `veloslet`; wire `cpu`/`memory` through `WorkerConfig`, the `run`/`install` CLI, and the registration request. Separately strip `maxContainers` (and the now-dead `running_containers` count cap) from the fluorite model, scheduler, server controllers, UI, and all test fixtures.

**Tech Stack:** Rust (edition 2024), clap, serde, thiserror, anyhow; macOS `sysctl`; fluorite IDL; React/TypeScript web UI.

## Global Constraints

- No `unwrap`/`expect`/`panic` in production code (workspace clippy `deny`). Test code may `#[allow(...)]`.
- No wildcard enum match arms.
- Semantic types over bare types: memory is a `Memory(u64)` newtype, never a bare integer.
- Fail closed: missing required config and over-host capacity are hard errors, never defaults/warnings.
- Memory is base-1024 (`1G == 1024^3`); suffixes `K`/`KB`/`M`/`MB`/`G`/`GB`/`B` (case-insensitive).
- Pre-PR gate: `make check` = `cargo fmt --all --check` + `cargo clippy --all-targets --all-features -- -D warnings` + `cargo test --workspace`.

---

### Task 1: `Memory` semantic type

**Files:**
- Create: `crates/veloslet/src/memory.rs`
- Modify: `crates/veloslet/src/lib.rs` (add `pub mod memory;`)

**Interfaces:**
- Produces: `veloslet::memory::Memory` with `Memory::from_bytes(u64) -> Memory`, `Memory::bytes(self) -> u64`, `impl FromStr` (base-1024, K/M/G/B suffixes), `impl Display`, serde as a string; and `veloslet::memory::MemoryParseError`.

- [ ] **Step 1: Write the failing tests**

Create `crates/veloslet/src/memory.rs`:

```rust
//! A human-readable memory size (`8G`, `512M`, `1048576`) wrapping a byte count.
//!
//! A semantic type (Principle #1): memory is never a bare integer. Parsed from
//! and rendered as a compact human string; base-1024.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A memory quantity in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory(u64);

impl Memory {
    /// Construct from a raw byte count.
    pub fn from_bytes(bytes: u64) -> Self {
        Memory(bytes)
    }

    /// The value in bytes.
    pub fn bytes(self) -> u64 {
        self.0
    }
}

/// Why a memory string could not be parsed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemoryParseError {
    #[error("memory value is empty")]
    Empty,
    #[error("invalid memory value {0:?}: expected a number optionally suffixed with K/M/G")]
    Invalid(String),
}

impl FromStr for Memory {
    type Err = MemoryParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(MemoryParseError::Empty);
        }
        let lower = trimmed.to_ascii_lowercase();
        let (digits, mult) = if let Some(n) = strip_unit(&lower, "gb", "g") {
            (n, 1024u64 * 1024 * 1024)
        } else if let Some(n) = strip_unit(&lower, "mb", "m") {
            (n, 1024u64 * 1024)
        } else if let Some(n) = strip_unit(&lower, "kb", "k") {
            (n, 1024u64)
        } else if let Some(n) = lower.strip_suffix('b') {
            (n, 1u64)
        } else {
            (lower.as_str(), 1u64)
        };
        let value: u64 = digits
            .trim()
            .parse()
            .map_err(|_| MemoryParseError::Invalid(s.to_string()))?;
        value
            .checked_mul(mult)
            .map(Memory)
            .ok_or_else(|| MemoryParseError::Invalid(s.to_string()))
    }
}

/// Strip a two- then one-char unit suffix, returning the remaining digits.
fn strip_unit<'a>(s: &'a str, long: &str, short: &str) -> Option<&'a str> {
    s.strip_suffix(long).or_else(|| s.strip_suffix(short))
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const G: u64 = 1024 * 1024 * 1024;
        const M: u64 = 1024 * 1024;
        const K: u64 = 1024;
        let b = self.0;
        if b != 0 && b % G == 0 {
            write!(f, "{}G", b / G)
        } else if b != 0 && b % M == 0 {
            write!(f, "{}M", b / M)
        } else if b != 0 && b % K == 0 {
            write!(f, "{}K", b / K)
        } else {
            write!(f, "{b}")
        }
    }
}

impl Serialize for Memory {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Memory {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[cfg_attr(test, allow(clippy::unwrap_used))]
mod tests {
    use super::*;

    #[test]
    fn parses_suffixes_base_1024() {
        assert_eq!("8G".parse::<Memory>().unwrap().bytes(), 8 * 1024 * 1024 * 1024);
        assert_eq!("512M".parse::<Memory>().unwrap().bytes(), 512 * 1024 * 1024);
        assert_eq!("2kb".parse::<Memory>().unwrap().bytes(), 2048);
        assert_eq!("1024".parse::<Memory>().unwrap().bytes(), 1024);
        assert_eq!("4096b".parse::<Memory>().unwrap().bytes(), 4096);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("".parse::<Memory>(), Err(MemoryParseError::Empty));
        assert!("8x".parse::<Memory>().is_err());
        assert!("abc".parse::<Memory>().is_err());
    }

    #[test]
    fn display_round_trips() {
        let m = "8G".parse::<Memory>().unwrap();
        assert_eq!(m.to_string(), "8G");
        assert_eq!(m.to_string().parse::<Memory>().unwrap(), m);
    }

    #[test]
    fn serde_is_a_string() {
        let m = Memory::from_bytes(8 * 1024 * 1024 * 1024);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "\"8G\"");
        assert_eq!(serde_json::from_str::<Memory>(&json).unwrap(), m);
    }
}
```

Add to `crates/veloslet/src/lib.rs` after `pub mod daemon;`:

```rust
pub mod memory;
```

- [ ] **Step 2: Run tests to verify they pass (after implementation lives in same file)**

Run: `cargo test -p veloslet memory::`
Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/veloslet/src/memory.rs crates/veloslet/src/lib.rs
git commit -m "feat(veloslet): add Memory semantic type"
```

---

### Task 2: Host detection + capacity validation

**Files:**
- Create: `crates/veloslet/src/host.rs`
- Modify: `crates/veloslet/src/lib.rs` (add `pub mod host;`)

**Interfaces:**
- Consumes: `crate::memory::Memory`.
- Produces: `veloslet::host::HostResources { cpu: u32, memory_bytes: u64 }`, `detect_host() -> anyhow::Result<HostResources>`, `validate_capacity(cpu: u32, memory: Memory, host: HostResources) -> anyhow::Result<()>`.

- [ ] **Step 1: Write the failing tests + implementation**

Create `crates/veloslet/src/host.rs`:

```rust
//! Host resource detection (macOS `sysctl`) and capacity validation.
//!
//! `validate_capacity` is a pure function over `HostResources` so it is unit
//! tested without touching the machine; `detect_host` is the side-effecting edge.

use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::memory::Memory;

/// The physical resources of the machine the worker runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostResources {
    pub cpu: u32,
    pub memory_bytes: u64,
}

/// Read host capacity from macOS `sysctl` (`hw.logicalcpu`, `hw.memsize`).
pub fn detect_host() -> Result<HostResources> {
    let cpu = sysctl_u64("hw.logicalcpu")?;
    let memory_bytes = sysctl_u64("hw.memsize")?;
    Ok(HostResources {
        cpu: u32::try_from(cpu).unwrap_or(u32::MAX),
        memory_bytes,
    })
}

fn sysctl_u64(key: &str) -> Result<u64> {
    let out = Command::new("sysctl")
        .args(["-n", key])
        .output()
        .with_context(|| format!("running sysctl -n {key}"))?;
    if !out.status.success() {
        bail!("sysctl -n {key} failed");
    }
    let text =
        String::from_utf8(out.stdout).with_context(|| format!("sysctl {key} output not UTF-8"))?;
    text.trim()
        .parse::<u64>()
        .with_context(|| format!("parsing sysctl {key} output {:?}", text.trim()))
}

/// Reject capacity that exceeds the physical host or is degenerate. Fail closed.
pub fn validate_capacity(cpu: u32, memory: Memory, host: HostResources) -> Result<()> {
    if cpu == 0 {
        bail!("cpu must be at least 1");
    }
    if cpu > host.cpu {
        bail!("requested {cpu} cores but machine has {}", host.cpu);
    }
    let want = memory.bytes();
    if want == 0 {
        bail!("memory must be greater than 0");
    }
    if want > host.memory_bytes {
        bail!(
            "requested {} memory but machine has {}",
            memory,
            Memory::from_bytes(host.memory_bytes)
        );
    }
    Ok(())
}

#[cfg(test)]
#[cfg_attr(test, allow(clippy::unwrap_used))]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn host() -> HostResources {
        HostResources {
            cpu: 8,
            memory_bytes: 16 * GB,
        }
    }

    #[test]
    fn accepts_capacity_within_host() {
        assert!(validate_capacity(8, Memory::from_bytes(16 * GB), host()).is_ok());
        assert!(validate_capacity(1, Memory::from_bytes(GB), host()).is_ok());
    }

    #[test]
    fn rejects_too_many_cores() {
        assert!(validate_capacity(9, Memory::from_bytes(GB), host()).is_err());
    }

    #[test]
    fn rejects_too_much_memory() {
        assert!(validate_capacity(1, Memory::from_bytes(32 * GB), host()).is_err());
    }

    #[test]
    fn rejects_zero() {
        assert!(validate_capacity(0, Memory::from_bytes(GB), host()).is_err());
        assert!(validate_capacity(1, Memory::from_bytes(0), host()).is_err());
    }
}
```

Add to `crates/veloslet/src/lib.rs` after `pub mod memory;`:

```rust
pub mod host;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p veloslet host::`
Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/veloslet/src/host.rs crates/veloslet/src/lib.rs
git commit -m "feat(veloslet): detect host resources and validate capacity"
```

---

### Task 3: Wire cpu/memory through config, CLI, and registration

**Files:**
- Modify: `crates/veloslet/src/daemon.rs` (`WorkerConfig` + tests)
- Modify: `crates/veloslet/src/main.rs` (`RunArgs`, `InstallArgs`, `resolve_run_config`, `run`, `install`)

**Interfaces:**
- Consumes: `Memory`, `detect_host`, `validate_capacity` from Tasks 1–2.
- Produces: `WorkerConfig` with required `cpu: u32` and `memory: Memory` fields.

- [ ] **Step 1: Update `WorkerConfig` and its tests in `daemon.rs`**

In `crates/veloslet/src/daemon.rs`, add the import near the top (after `use serde::...`):

```rust
use crate::memory::Memory;
```

Replace the `WorkerConfig` struct (currently fields server/node/token + 3 interval fields) so the two new fields sit after `token` with **no** serde default:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Server base URL, e.g. `http://192.168.68.60:8088`.
    pub server: String,
    /// This worker's name.
    pub node: String,
    /// Bootstrap token (`id.secret`) used to register on each start.
    pub token: String,
    /// Advertised CPU cores. Required; validated against the host at startup.
    pub cpu: u32,
    /// Advertised memory (e.g. `"8G"`). Required; validated against the host.
    pub memory: Memory,
    #[serde(default = "default_reconcile_secs")]
    pub reconcile_secs: u64,
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u64,
    #[serde(default = "default_lease_secs")]
    pub lease_secs: u32,
}
```

Update the two affected tests in the same file. Replace `config_roundtrips_through_json`'s struct literal to include the new fields:

```rust
    #[test]
    fn config_roundtrips_through_json() {
        let cfg = WorkerConfig {
            server: "http://192.168.68.60:8088".to_string(),
            node: "node-a".to_string(),
            token: "id.secret".to_string(),
            cpu: 4,
            memory: crate::memory::Memory::from_bytes(8 * 1024 * 1024 * 1024),
            reconcile_secs: 5,
            heartbeat_secs: 10,
            lease_secs: 40,
        };
        let text = serde_json::to_string(&cfg).unwrap();
        let back: WorkerConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }
```

Replace `config_applies_interval_defaults_when_omitted` so the JSON includes the now-required `cpu`/`memory`:

```rust
    #[test]
    fn config_applies_interval_defaults_when_omitted() {
        let cfg: WorkerConfig = serde_json::from_str(
            r#"{"server":"http://h:1","node":"n","token":"t","cpu":4,"memory":"8G"}"#,
        )
        .unwrap();
        assert_eq!(cfg.cpu, 4);
        assert_eq!(cfg.memory.bytes(), 8 * 1024 * 1024 * 1024);
        assert_eq!(cfg.reconcile_secs, 5);
        assert_eq!(cfg.heartbeat_secs, 10);
        assert_eq!(cfg.lease_secs, 40);
    }
```

- [ ] **Step 2: Update CLI args and wiring in `main.rs`**

In `crates/veloslet/src/main.rs`, extend the imports:

```rust
use veloslet::daemon::{self, BUNDLE_EXECUTABLE, BUNDLE_ID, WorkerConfig};
use veloslet::host::{detect_host, validate_capacity};
use veloslet::memory::Memory;
use veloslet::{ApiClient, run_loop};
```

Add to `RunArgs` (after the `token` field):

```rust
    /// Advertised CPU cores (overrides config).
    #[arg(long)]
    cpu: Option<u32>,
    /// Advertised memory, e.g. `8G` (overrides config).
    #[arg(long)]
    memory: Option<Memory>,
```

Add to `InstallArgs` (after the `token` field), both required:

```rust
    /// Advertised CPU cores.
    #[arg(long)]
    cpu: u32,
    /// Advertised memory, e.g. `8G`.
    #[arg(long)]
    memory: Memory,
```

In `resolve_run_config`, extend the `None =>` base literal to require the new fields (insert after the `token:` line, before `reconcile_secs:`):

```rust
            cpu: args
                .cpu
                .context("--cpu is required when --config is not given")?,
            memory: args
                .memory
                .context("--memory is required when --config is not given")?,
```

And add overrides after the existing `if let Some(v) = args.token { cfg.token = v; }` block:

```rust
    if let Some(v) = args.cpu {
        cfg.cpu = v;
    }
    if let Some(v) = args.memory {
        cfg.memory = v;
    }
```

In `run`, validate before registering. Replace the start of `run` up to and including the register `request` literal:

```rust
async fn run(cfg: WorkerConfig) -> Result<()> {
    // Fail closed: never advertise more than the machine physically has.
    let host = detect_host()?;
    validate_capacity(cfg.cpu, cfg.memory, host)?;

    let runtime = AppleContainer::new();
    let runtime_version = runtime
        .version()
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    // Register with the bootstrap token to obtain a worker credential.
    let boot = ApiClient::new(&cfg.server, Some(cfg.token.clone()));
    let request = serde_json::json!({
        "name": cfg.node,
        "capacity": { "cpu": cfg.cpu, "memoryBytes": cfg.memory.bytes() },
        "addresses": [],
        "containerRuntimeVersion": runtime_version,
    });
```

In `install`, validate before any side effects. Insert right after `let version = env!("CARGO_PKG_VERSION");`:

```rust
    // Reject impossible capacity before touching the filesystem or launchd.
    let host = detect_host()?;
    validate_capacity(args.cpu, args.memory, host)?;
```

And extend the persisted `WorkerConfig` literal (the `let cfg = WorkerConfig { ... }` in `install`) to carry the new fields after `token: args.token,`:

```rust
        cpu: args.cpu,
        memory: args.memory,
```

- [ ] **Step 3: Build and test**

Run: `cargo test -p veloslet`
Expected: PASS (daemon + memory + host tests).

Run: `cargo build -p veloslet`
Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add crates/veloslet/src/daemon.rs crates/veloslet/src/main.rs
git commit -m "feat(veloslet): require & validate cpu/memory from config or flags"
```

---

### Task 4: Remove `maxContainers` from model, scheduler, server, and fixtures

This must be one atomic change so the workspace keeps compiling.

**Files:**
- Modify: `crates/models/fluorite/velos.fl:59-63`
- Modify: `crates/scheduler/src/lib.rs`
- Modify: `crates/server/src/controllers.rs`
- Modify: `crates/server/src/lib.rs:1532`
- Modify: `crates/tests/tests/e2e.rs:61`
- Modify: `crates/tests/tests/e2e_apple.rs:163`

**Interfaces:**
- Produces: `scheduler::WorkerView` without `running_containers`/`max_containers`; `admits` bounded by ready/unschedulable/cpu/memory only.

- [ ] **Step 1: Fluorite model** — in `crates/models/fluorite/velos.fl`, change `NodeResources` to drop the cap:

```
struct NodeResources {
    cpu: u32,
    memory_bytes: u64,
}
```

- [ ] **Step 2: Scheduler** — in `crates/scheduler/src/lib.rs`:

Remove the two fields from `WorkerView` (delete the `pub running_containers: u32,` and `pub max_containers: u32,` lines). The struct ends at `allocated: ResourceRequest,`.

Replace `admits`:

```rust
    /// Whether this worker can admit one more container needing `req`.
    fn admits(&self, req: &ResourceRequest) -> bool {
        self.ready
            && !self.unschedulable
            && self.free_cpu() >= req.cpu
            && self.free_memory() >= req.memory_bytes
    }
```

Replace the test `worker` helper (drop the `running`/`max` params and fields):

```rust
    fn worker(
        name: &str,
        ready: bool,
        unschedulable: bool,
        cpu: u32,
        mem: u64,
        used_cpu: u32,
        used_mem: u64,
    ) -> WorkerView {
        WorkerView {
            name: WorkerName(name.to_string()),
            ready,
            unschedulable,
            allocatable: ResourceRequest {
                cpu,
                memory_bytes: mem,
            },
            allocated: ResourceRequest {
                cpu: used_cpu,
                memory_bytes: used_mem,
            },
        }
    }
```

Update every `worker(...)` call to drop the trailing two args:
- `picks_first_fitting_ready_worker`: `worker("w1", true, false, 1, 8 * GB, 0, 0)` and `worker("w2", true, false, 4, 8 * GB, 0, 0)`
- `skips_not_ready_and_unschedulable_workers`: `worker("w1", false, false, 8, 16 * GB, 0, 0)`, `worker("w2", true, true, 8, 16 * GB, 0, 0)`, `worker("w3", true, false, 8, 16 * GB, 0, 0)`
- `respects_already_allocated_capacity`: `worker("w1", true, false, 4, 8 * GB, 3, 0)`
- `returns_none_when_nothing_fits`: `worker("w1", true, false, 8, 16 * GB, 0, 0)`

Delete the entire `respects_max_container_count` test.

- [ ] **Step 3: Server controllers** — in `crates/server/src/controllers.rs`:

Replace `usage_for` to return just the resource sum (drop the count):

```rust
fn usage_for(containers: &[StoredObject], worker: &str) -> ResourceRequest {
    let mut cpu = 0;
    let mut mem = 0;
    for c in containers {
        if c.node_name.as_deref() == Some(worker)
            && matches!(phase(&c.document), Some("Scheduled") | Some("Running"))
        {
            let r = container_request(&c.document);
            cpu += r.cpu;
            mem += r.memory_bytes;
        }
    }
    ResourceRequest {
        cpu,
        memory_bytes: mem,
    }
}
```

In `build_worker_views`, replace the `usage_for` call line and drop the two fields:

```rust
            let allocated = usage_for(containers, &w.name);
            WorkerView {
                name: WorkerName(w.name.clone()),
                ready: worker_ready(&w.document),
                unschedulable: w
                    .document
                    .get("spec")
                    .and_then(|s| s.get("unschedulable"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                allocatable: ResourceRequest {
                    cpu: u64_at(&w.document, &["status", "allocatable", "cpu"])
                        .map(|c| c as u32)
                        .unwrap_or(0),
                    memory_bytes: u64_at(&w.document, &["status", "allocatable", "memoryBytes"])
                        .unwrap_or(0),
                },
                allocated,
            }
```

In `plan_bindings`, delete the line `w.running_containers += 1;`.

In the test `view` helper (line ~432), drop the `max` param and the two fields:

```rust
    fn view(name: &str, cpu: u32, mem: u64) -> WorkerView {
        WorkerView {
            name: WorkerName(name.to_string()),
            ready: true,
            unschedulable: false,
            allocatable: ResourceRequest {
                cpu,
                memory_bytes: mem,
            },
            allocated: ResourceRequest {
                cpu: 0,
                memory_bytes: 0,
            },
        }
    }
```

Update its call site (line ~474): `let workers = vec![view("w1", 4, 8 * GB)];`.

In `ready_worker_doc` (line ~527), drop `maxContainers` from the allocatable JSON:

```rust
                "allocatable": { "cpu": 8, "memoryBytes": 16u64 * GB },
```

- [ ] **Step 4: Remaining test fixtures** — drop `, "maxContainers": N` from the capacity/allocatable JSON in:
- `crates/server/src/lib.rs:1532` → `"capacity": { "cpu": 4, "memoryBytes": 8589934592u64 },`
- `crates/tests/tests/e2e.rs:61` → `"allocatable": { "cpu": 4, "memoryBytes": 8589934592u64 },`
- `crates/tests/tests/e2e_apple.rs:163` → `"capacity": { "cpu": 4, "memoryBytes": 8589934592u64 },`

- [ ] **Step 5: Build and test the workspace**

Run: `cargo test -p velos-scheduler -p velos-server`
Expected: PASS (no `respects_max_container_count`; CPU/memory admission tests still green).

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/models/fluorite/velos.fl crates/scheduler/src/lib.rs \
        crates/server/src/controllers.rs crates/server/src/lib.rs \
        crates/tests/tests/e2e.rs crates/tests/tests/e2e_apple.rs
git commit -m "refactor: remove maxContainers capacity field and container count cap"
```

---

### Task 5: Remove `maxContainers` from the web UI

**Files:**
- Modify: `web/src/types.ts:56-60`
- Modify: `web/src/views/Workers.tsx:74-78` and `:155-157`

- [ ] **Step 1: Drop the type field** — in `web/src/types.ts`, change `Capacity`:

```ts
export interface Capacity {
  cpu?: number;
  memoryBytes?: number;
}
```

- [ ] **Step 2: Replace the card "slots" gauge** — in `web/src/views/Workers.tsx`, replace the left `<span>` (the one rendering `{active.length} / … slots`) with a plain running count, keeping the lease span untouched:

```tsx
                  <span>
                    <span className="font-mono text-zinc-300">{active.length}</span> running
                  </span>
```

- [ ] **Step 3: Drop the drawer "max"** — in the same file, the `<Field label="Capacity">` block becomes:

```tsx
        <Field label="Capacity">
          {worker.status?.capacity?.cpu ?? "—"} cores · {fmtBytes(worker.status?.capacity?.memoryBytes)}
        </Field>
```

- [ ] **Step 4: Verify the build**

Run: `cd web && npm run build`
Expected: type-checks and builds with no reference to `maxContainers`.

- [ ] **Step 5: Commit**

```bash
git add web/src/types.ts web/src/views/Workers.tsx
git commit -m "refactor(web): drop maxContainers slots display"
```

---

### Task 6: Docs + full gate

**Files:**
- Modify: `docs/getting-started.md` (install example)

- [ ] **Step 1: Update the install example** — find the `veloslet install` example in `docs/getting-started.md` and add the now-required flags, e.g.:

```
veloslet install \
  --server http://192.168.68.60:8088 \
  --node node-a \
  --token <id.secret> \
  --cpu 8 \
  --memory 16G
```

Add a one-line note: the worker refuses to start if `--cpu`/`--memory` exceed the machine's physical resources, and existing installs must be re-run (or their `~/.velos/veloslet.json` updated with `cpu`/`memory`) after upgrading.

- [ ] **Step 2: Run the full pre-PR gate**

Run: `make check`
Expected: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --workspace` all pass.

- [ ] **Step 3: Commit**

```bash
git add docs/getting-started.md
git commit -m "docs: document required veloslet --cpu/--memory flags"
```

---

## Self-Review

**Spec coverage:**
- Read cpu/memory from config or CLI → Task 3 (config fields + `RunArgs`/`InstallArgs` flags + override merge).
- Required, no defaults → Task 3 (no `serde(default)`; `context("… is required …")` for the flag path).
- Validate against machine → Task 2 (`detect_host`/`validate_capacity`) applied in Task 3 (`run` + `install`).
- Human-readable memory → Task 1 (`Memory` type, serde-as-string).
- Remove `maxContainers` → Tasks 4 (model/scheduler/server/fixtures) + 5 (UI).
- Rollout note → Task 6 (docs).

**Placeholder scan:** none — every code step shows full content.

**Type consistency:** `Memory::from_bytes`/`Memory::bytes` used identically across Tasks 1–3; `validate_capacity(cpu, memory, host)` signature matches its call sites; `usage_for` returns `ResourceRequest` and its sole caller in `build_worker_views` is updated; scheduler `worker(...)`/controllers `view(...)` helper arities match all updated call sites.
