# Veloslet resource capacity: configurable, machine-validated, and dropping `maxContainers`

**Date:** 2026-06-28
**Status:** Approved (pending spec review)

## Problem

The worker daemon (`veloslet`) registers with a **hardcoded** capacity —
`{ "cpu": 4, "memoryBytes": 8GB, "maxContainers": 16 }` — in
`crates/veloslet/src/main.rs:190`, regardless of the actual machine. Every worker
therefore advertises the same fictional capacity, the scheduler makes placement
decisions against numbers that have no relation to the host, and the UI shows a
meaningless "0 / 16 slots" gauge.

We want the worker's CPU and memory capacity to come from configuration (config
file or CLI flag), be **required with no defaults**, and be **validated against the
real machine** so a worker cannot advertise more than the host physically has.

Separately, `maxContainers` is being removed entirely (see Decisions).

## Goals

1. CPU cores and memory are read from the JSON config file **or** CLI flags.
2. Both are **required** — a missing value is a hard error, never a default.
3. At startup (and at install time) the requested values are **validated against
   the host**; requesting more than the machine has aborts with a clear error.
4. `maxContainers` is removed from the model, scheduler, server, UI, and tests.

## Non-goals

- Auto-detecting/auto-filling capacity (the user must state intent explicitly).
- CPU/memory *overcommit* policy beyond the physical ceiling.
- Any change to how per-container resource requests are specified.

## Decisions

- **Memory is human-readable** everywhere: config stores `"memory": "8G"`, the CLI
  takes `--memory 8G`. A new semantic `Memory` type parses/round-trips it.
- **Validation fails closed** (design principle #6): requesting more cores or bytes
  than the machine has aborts startup with a typed error — not a warning.
- **`maxContainers` is removed entirely.** It is the one capacity field with no
  physical machine analog (nothing to validate against), and CPU + memory already
  bound scheduling. Its only real use was a redundant count cap in the scheduler.

## Design

### 1. The `Memory` semantic type

A new type wrapping `u64` bytes — never a bare integer (design principle #1):

- `FromStr`: accepts `K`/`KB`, `M`/`MB`, `G`/`GB` suffixes (case-insensitive) and
  plain integers (bytes). **Base-1024** (`1G == 1024^3` bytes). Rejects empty,
  non-numeric, zero, and unknown-suffix input with a typed parse error.
- `Display` / `Serialize`: round-trips to a compact human string (e.g. `8G`). It
  serializes as a string, so config carries `"memory": "8G"`.
- `bytes(&self) -> u64`: the value used to build the wire request and to compare
  against the host.

Lives in `veloslet` (`daemon.rs` alongside `WorkerConfig`, or a small `memory.rs`).
Unit-tested for parse round-trips and rejection cases.

### 2. Config & CLI surface

`WorkerConfig` (in `crates/veloslet/src/daemon.rs`) gains two **required**,
no-`serde(default)` fields:

```rust
pub cpu: u32,          // cores
pub memory: Memory,    // serialized as "8G"
```

Because there is no `#[serde(default …)]`, a config file missing either field is a
hard `serde` parse error — fail-closed, no silent default. `maxContainers` was never
in the config, so nothing is removed there.

CLI flags (`crates/veloslet/src/main.rs`):

- `RunArgs`: add `--cpu <N>` (`Option<u32>`) and `--memory <SIZE>`
  (`Option<Memory>`). Optional at the clap layer; after merging with `--config`,
  if neither config nor flag supplied them, error — mirroring the existing
  `--server`/`--node`/`--token` "required when --config is not given" pattern in
  `resolve_run_config`.
- `InstallArgs`: add `--cpu <N>` (required) and `--memory <SIZE>` (required), like
  the existing required `--server`/`--node`/`--token`. They are written into the
  persisted `WorkerConfig`.

### 3. Machine validation (fail-closed)

A small detection function reads host capacity on macOS by shelling out to
`sysctl` (no new dependency; consistent with the daemon already shelling out to
`container` and `launchctl`):

- cores: `sysctl -n hw.logicalcpu`
- bytes: `sysctl -n hw.memsize`

Returned as a typed `HostResources { cpu: u32, memory_bytes: u64 }`. A typed error
covers "sysctl missing / unparseable output".

Validation policy (one function, the only new policy logic):

- `cfg.cpu == 0` → error (must be ≥ 1).
- `cfg.cpu > host.cpu` → error: `requested N cores but machine has M`.
- `cfg.memory.bytes() == 0` → error (must be > 0).
- `cfg.memory.bytes() > host.memory_bytes` → error: `requested 32G but machine has 16G`.

Applied:

- In `run()` **before** registering with the server.
- In `install()` **before** writing the config and loading the LaunchAgent, so a
  bad value is rejected at install time rather than failing silently later inside
  the agent.

### 4. Wire request

`crates/veloslet/src/main.rs` register request becomes:

```rust
"capacity": { "cpu": cfg.cpu, "memoryBytes": cfg.memory.bytes() },
```

`maxContainers` is gone from the payload.

### 5. Removing `maxContainers` (blast radius)

- **`crates/models/fluorite/velos.fl:62`** — drop `max_containers` from
  `NodeResources`.
- **`crates/scheduler/src/lib.rs`** — remove the `max_containers` field (`:31`), the
  `self.running_containers < self.max_containers` clause in `admits()` (`:51`), the
  now-unused `running_containers` field (it existed only for that comparison), the
  `respects_max_container_count` test (`:141`), and the corresponding test-fixture
  arguments. `admits()` becomes
  `ready && !unschedulable && free_cpu >= req.cpu && free_memory >= req.memory_bytes`.
- **`crates/server/src/controllers.rs`** — remove the `max_containers` extraction
  (`:172`), the `WorkerView` plumbing, and the `:446`/`:527` test references.
- **Test fixtures** — remove `"maxContainers"` from capacity JSON in
  `crates/server/src/lib.rs:1532`, `crates/server/src/controllers.rs:527`,
  `crates/tests/tests/e2e.rs:61`, `crates/tests/tests/e2e_apple.rs:163`.
- **UI** — `web/src/types.ts:59` drop `maxContainers` from `Capacity`;
  `web/src/views/Workers.tsx:77` remove the "X / Y slots" gauge; `:156` remove the
  "Y max" line in the worker drawer.

After removal, scheduling is bounded purely by CPU + memory, both now real
machine-validated values.

## Error handling

- Memory parse failure → typed parse error surfaced via `anyhow` context in the
  binary, naming the offending input.
- Missing required config/flags → existing `context("… is required …")` pattern.
- Capacity exceeds host → typed validation error with both requested and actual
  values; aborts `run`/`install`.
- `sysctl` unavailable/unparseable → error (fail-closed: a worker that cannot
  verify its own capacity does not register).

## Testing

- `Memory` parse/round-trip + rejection unit tests (in `veloslet`).
- Validation function unit tests: equal-to-host passes; over-host (cpu, memory)
  rejected; zero rejected. Host values injected (pure function over
  `HostResources`) so no real `sysctl` call in unit tests.
- Scheduler tests updated to drop `maxContainers`; existing CPU/memory admission
  tests continue to pass.
- Server/e2e fixtures updated; `make check` (fmt + clippy + workspace tests) green.

## Rollout / compatibility

Existing installs have a `~/.velos/veloslet.json` without `cpu`/`memory`; after this
change the worker will refuse to start until the config is updated (or it is
re-installed with `--cpu`/`--memory`). This is intentional (fail-closed) and noted
in `docs/getting-started.md`, which gains the two new required flags in the install
example.
