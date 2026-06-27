# Fleet — Design

**Date:** 2026-06-27
**Status:** Approved design, pre-implementation

## Summary

Fleet is a Kubernetes-style control plane that manages the lifecycle of **containers**
on a fleet of registered remote **macOS workers**, exposed over a RESTful API. A
container is an OCI image run in its own lightweight VM via **Apple Containerization**
(the open-source `container` tool); the worker daemon drives it by wrapping the
`container` CLI.

The system follows the k8s architecture: declarative resources with a `spec`
(desired) / `status` (observed) split, worker-initiated watch + reconciliation, and
lease-based heartbeats. This gives an **eventual-convergence** guarantee — a workload
is only ever reported running when the owning worker says so, and dead workers are
detected and their work reconciled.

### Decisions locked during design

| Topic | Decision |
|---|---|
| Use case | General-purpose VM fleet → generic, workload-agnostic lifecycle API |
| VM backend | Apple Containerization (`container` CLI), wrapped by the Rust worker daemon |
| Workload unit | A **Container** (OCI image + run spec), each backed by a micro-VM (the Pod analog) |
| Control plane | Single-node process, embedded **SQLite** datastore |
| Worker comms | **Worker-initiated pull/watch** + status reporting + lease heartbeats |
| Worker auth | **Bootstrap token → issued per-worker credential** (the `kubeadm` pattern) |
| Architecture style | Mirror k8s: declarative resources + reconciliation (not an imperative job queue) |
| Schema/codegen | **fluorite** `.fl` schemas → shared Rust wire types (TS/Swift available later) |
| Storage | Opaque serialized document + hand-written index columns (protocol ≠ storage) |
| Web UI | Deferred — separate future design, will consume fluorite-generated TS types |

Design principles (semantic types, illegal-states-unrepresentable, deep modules,
compile-time enforcement, pure core / impure edge, fail closed, protocol ≠ storage)
are adopted from `zhxiaogg/hackamore` and persisted in this repo's `CLAUDE.md`.

Naming: the system is **Fleet**; the control plane is **`fleet-apiserver`** plus
in-process controllers; the worker daemon is **`fleetlet`** (the kubelet analog); the
CLI is **`fleetctl`**.

## Architecture

```
                          ┌──────────────────────────────────────────┐
   operator / CLI  ───────▶            CONTROL PLANE (single node)     │
   (fleetctl)      REST   │                                            │
                          │  ┌─────────────┐   ┌──────────────────┐    │
                          │  │  apiserver  │◀──│ SQLite datastore │    │
                          │  │ (REST+watch)│   │ (objects + watch │    │
                          │  └──────┬──────┘   │   event log)     │    │
                          │         │          └──────────────────┘    │
                          │  ┌──────▼──────────────────────────┐       │
                          │  │ controllers (in-process tasks):  │       │
                          │  │  • scheduler  • node-lifecycle   │       │
                          │  │  • container-gc / reconcilers    │       │
                          │  └──────────────────────────────────┘       │
                          └───────────────▲──────────────────────────────┘
                                          │  outbound HTTPS only
                  ┌───────────────────────┼───────────────────────┐
          ┌───────▼────────┐      ┌───────▼────────┐      ┌───────▼────────┐
          │  WORKER (mac)  │      │  WORKER (mac)  │      │  WORKER (mac)  │
          │   fleetlet     │      │   fleetlet     │      │   fleetlet     │
          │  wraps `container` CLI (Apple Containerization) │                │
          │  ┌─────────┐ ┌─────────┐               │      │                │
          │  │microVM/ │ │microVM/ │   ...         │      │                │
          │  │container│ │container│               │      │                │
          │  └─────────┘ └─────────┘               │      │                │
          └────────────────┘      └────────────────┘      └────────────────┘
```

### Components

**Control plane** (one Rust process, `fleet-apiserver`):

- **API server** — REST + a streaming `watch` endpoint; the only component that
  touches the datastore.
- **Datastore** — embedded SQLite. Stores all objects plus an append-only
  **event/revision log** that powers `watch`. Each write bumps a global monotonic
  `resourceVersion` (a SQLite sequence).
- **Controllers** (in-process async tasks, each a reconcile loop):
  - **scheduler** — assigns unscheduled Containers to a Worker.
  - **node-lifecycle** — watches Worker leases, marks dead workers `NotReady`,
    triggers rescheduling.
  - **container-gc / reconciler** — cleans up terminated containers, enforces
    desired vs observed.

**Worker** (Rust daemon, `fleetlet`, one per macOS host):

- Registers with a bootstrap token, receives a per-worker credential.
- Opens an **outbound** watch for Containers assigned to it
  (`fieldSelector=spec.nodeName=<me>`).
- Drives Apple Containerization by wrapping the `container` CLI
  (`run`/`stop`/`rm`/`ls`/`inspect`), maps results into `status`.
- Renews a **Lease** (heartbeat) every ~10s; reports node capacity/health.

**`fleetctl`** — thin Rust CLI over the REST API (the kubectl analog).

**Web UI** — *deferred*. A separate future design; it will consume the
fluorite-generated TypeScript types from `fleet-models`.

## Schema / codegen layer (fluorite)

`fleet-models` compiles `fluorite/*.fl` schemas in `build.rs` with serde + schemars
derives. `apiserver`, `fleetlet`, and `fleetctl` all depend on it — one source of
truth for wire types. fluorite is a pure data-type IDL (no REST endpoint description),
so resource *types* live in `.fl` and routing is hand-written.

Each resource is a full object composed of a shared `ObjectMeta` (fluorite has no
generics on user-defined types, so there is no `Resource<Spec, Status>`):

```rust
package fleet;                      // single namespace → names unique project-wide

struct ObjectMeta {
    name: String, uid: Uuid,
    labels: Map<String, String>, annotations: Map<String, String>,
    resourceVersion: u64, creationTimestamp: DateTimeUtc,
    deletionTimestamp: Option<DateTimeUtc>, finalizers: Vec<String>,
}

struct Container { metadata: ObjectMeta, spec: ContainerSpec, status: ContainerStatus }
struct ContainerSpec {
    image: String, command: Vec<String>, env: Map<String, String>,
    resources: ResourceReqs, restartPolicy: RestartPolicy,
    nodeName: Option<String>,          // set by the scheduler
}
struct ContainerStatus {
    phase: ContainerPhase, workerName: Option<String>, containerID: Option<String>,
    startedAt: Option<DateTimeUtc>, finishedAt: Option<DateTimeUtc>,
    exitCode: Option<i32>, message: Option<String>,
}
enum ContainerPhase { Pending, Scheduled, Running, Succeeded, Failed, Unknown }
enum RestartPolicy { Never, OnFailure, Always }

struct Worker { metadata: ObjectMeta, spec: WorkerSpec, status: WorkerStatus }
struct WorkerSpec { unschedulable: bool }
struct WorkerStatus {
    capacity: NodeResources, allocatable: NodeResources,
    conditions: Vec<NodeCondition>, addresses: Vec<String>,
    containerRuntimeVersion: String,
}

struct Lease { metadata: ObjectMeta, spec: LeaseSpec }
struct LeaseSpec { holderIdentity: String, renewTime: DateTimeUtc, leaseDurationSeconds: u32 }

// watch frames — payload is Any (serde_json::Value)
#[type_tag = "type"] #[content_tag = "object"]
union WatchEvent { Added(Any), Modified(Any), Deleted(Any) }
```

Hand-written convenience constructors live in `fleet-models/src/lib.rs`, not in the
schema.

## REST API surface

Hand-written axum routes over the fluorite types, k8s-shaped:

| Method + path | Purpose |
|---|---|
| `GET /api/v1/{containers,workers,leases}` | list; `?labelSelector=`, `?fieldSelector=spec.nodeName=node-7` |
| `GET …/{name}` | read one |
| `POST …` / `PUT …/{name}` / `PATCH …/{name}` / `DELETE …/{name}` | create / replace / merge / delete (DELETE sets `deletionTimestamp` + runs finalizers) |
| `PUT …/{name}/status` | **status subresource** — the only way to write `status` |
| `GET /api/v1/{resource}?watch=true&resourceVersion=N` | **watch stream** — chunked NDJSON of `WatchEvent` frames since version N |
| `POST /api/v1/workers:register` | bootstrap-token join → returns `WorkerCredential` |
| `GET /openapi.json` | served from schemars-generated schemas |

The **status subresource** enforces the spec/status split: clients/scheduler write
`spec`; only the owning `fleetlet` writes `status`. The **watch** endpoint is what
`fleetlet` opens outbound.

## Reconciliation & lifecycle

Everything is a control loop: pure decision functions produce intended actions;
actuators apply them. No fire-and-forget.

Container happy path:

```
1. CREATE     client POSTs Container {spec.image, ...} → apiserver admits, persists
              phase=Pending, resourceVersion=N, emits WatchEvent::Added
2. SCHEDULE   scheduler (watching Pending, nodeName=None) runs pure
              schedule(container, workers) -> Some("node-7")
              PUT spec.nodeName=node-7   (spec write — scheduler owns assignment)
              phase=Scheduled
3. DISPATCH   fleetlet on node-7 (watch fieldSelector=spec.nodeName=node-7)
              receives WatchEvent::Modified → sees the new assignment
4. ACTUATE    fleetlet reconciles desired vs local reality:
              `container run --detach <image> ...` tagged with the container uid
              PUT .../status {phase=Running, containerID, startedAt}
5. OBSERVE    fleetlet polls `container ls/inspect`; on exit:
              PUT .../status {phase=Succeeded|Failed, exitCode, finishedAt}
6. DELETE     client DELETE → deletionTimestamp set, finalizer present
              fleetlet `container stop/rm`, clears its finalizer
              → apiserver hard-deletes the row
```

Key properties:

- **The worker is authoritative for `status`.** Only the owning `fleetlet` writes a
  Container's `status` (via the `/status` subresource). "Is it really running?" is
  answered solely by what the worker reported.
- **Desired state is durable; reconciliation is continuous.** A crashed/reconnected
  `fleetlet` re-lists assigned containers at its last `resourceVersion` and
  re-converges. A missed watch event is harmless — a periodic full LIST (~30–60s) is
  the safety net. Delivery is not a packet that can be lost.
- **Pure core / impure edge.** `schedule(...)` and the fleetlet's
  `reconcile(desired, observed) -> Vec<Action>` are pure and unit-testable; only the
  actuators touch SQLite or the `container` CLI.
- **`restartPolicy`** is enforced in the fleetlet's reconcile: `Always`/`OnFailure`
  → re-run on exit and report new `status`; `Never` → terminal phase.
- **Idempotency.** Every actuator action is keyed by container `uid`; reconcile after
  a crash checks `container ls` for an existing instance tagged with the uid before
  launching, so it never double-creates.

Watch/resync mechanics: `watch?resourceVersion=N` replays the event log from version
N then streams live frames; `fleetlet` keeps its last-seen version to resume cheaply;
a periodic full LIST guards against any gap.

## Failure handling, heartbeats & rescheduling

Heartbeats: each `fleetlet` renews its `Lease` every ~10s. The node-lifecycle
controller watches leases; if `now - renewTime > leaseDuration` (default 40s) it sets
`Worker` condition `Ready=False` (`NotReady`). The lease is the single source of
liveness.

| Failure | Detection | Response |
|---|---|---|
| Worker process dies / host offline | Lease expires (40s) | Worker → `NotReady`. After grace (`evictionTimeout`, default ~5m), its `Running` containers → `status.phase=Unknown`; reschedulable ones have `spec.nodeName` cleared for re-binding. |
| Worker reconnects after blip | Lease renews | `NotReady`→`Ready`; fleetlet re-lists at last `resourceVersion` and reconciles. No reschedule if recovered before grace. |
| `container` CLI failure (pull/OOM/runtime) | fleetlet observes non-zero/missing instance | fleetlet writes `status.phase=Failed` + `message`; restartPolicy decides retry. Fail closed: ambiguous → `Failed`. |
| Container exits | fleetlet `inspect` | `Succeeded`/`Failed` + `exitCode`; restartPolicy applies. |
| apiserver restart | — | State durable in SQLite incl. event log + last `resourceVersion`; controllers and fleetlets reconnect watches and resume. |
| Split-brain (two fleetlets, same name) | Lease `holderIdentity` mismatch | Registration binds a worker name to one credential; a second holder is rejected. |
| Orphaned micro-VM | fleetlet periodic sweep: `container ls` vs assigned set | fleetlet reaps any instance tagged with a uid it isn't assigned. |

Rescheduling policy (v1): only containers explicitly marked reschedulable (or, later,
owned by a higher-level controller) are re-bound on node loss. A bare one-off
container is **not** auto-rescheduled — it goes `Unknown`/`Failed` and is surfaced
(matching k8s "bare pods aren't rescheduled"). This is the seam where a future
`Deployment`/`ReplicaSet`-style controller adds self-healing.

Timeouts are control-plane config: `heartbeatInterval` (10s), `leaseDuration` (40s),
`evictionTimeout` (5m).

## Auth & worker registration

Bootstrap-token join (the `kubeadm` pattern), fail-closed throughout:

```
1. MINT     `fleetctl token create` → BootstrapToken {id, secret, ttl, allowedLabels?}
            Secret shown once; stored hashed.
2. JOIN     fleetlet --server URL --token <id.secret>
            POST /api/v1/workers:register  (Authorization: Bearer <id.secret>)
            body: proposed worker name + node info (arch, macosVersion, capacity)
3. ISSUE    apiserver verifies token (hash match, unexpired, unrevoked) →
            creates Worker + issues a per-worker WorkerCredential (long-lived bearer,
            stored hashed). Returned ONCE.
4. PERSIST  fleetlet writes credential to a local file (0600); uses it as Bearer on
            all subsequent calls (watch, status, lease renew).
5. AUTHZ    every request → authenticate + authorize: a worker may only read/write
            objects scoped to itself (its Worker, its Leases, Containers where
            spec.nodeName == itself).
```

- **Semantic types:** `BootstrapToken`, `WorkerCredential`, `WorkerName` are distinct;
  secrets are a `Secret` newtype that never `Display`s/logs.
- **Fail closed:** unknown/expired/revoked token → 401, no worker created; missing
  credential → 401; cross-worker access → 403. No anonymous path.
- **Protocol ≠ storage:** the wire `WorkerCredential` (fluorite type) is distinct from
  the hashed credential record persisted in SQLite (hand-written). Only the hash is
  stored.
- **Revocation:** `fleetctl worker delete <name>` tombstones the hashed credential;
  the worker's next call fails closed and it must re-join.

Transport: TLS on the apiserver from day one (operator-supplied cert, or self-signed
dev cert). The `Authenticator` is a trait; mTLS with a CA is the documented upgrade
path behind that seam (not built in v1).

## Tech stack

| Concern | Choice |
|---|---|
| Async runtime | tokio |
| HTTP server | axum (streaming watch bodies, tower middleware for auth) |
| HTTP client (fleetlet) | reqwest |
| Datastore | rusqlite (or `sqlx` sqlite) — opaque-document + index-column store |
| Schema/codegen | fluorite (`fluorite_codegen` in build.rs) |
| Serialization | serde + serde_json (NDJSON watch frames) |
| OpenAPI | schemars derive on fluorite types |
| CLI | clap |
| Errors | thiserror (libs) / anyhow (binaries) |
| Tracing | tracing + tracing-subscriber |
| `container` interface | wrap the `container` CLI via tokio::process; parse JSON output |

## Crate layout

```
fleet/                          # Rust crate workspace
├── Cargo.toml                  # workspace + clippy lints (deny unwrap/expect/panic/wildcard)
├── Makefile                    # make check = fmt + clippy -D warnings + test
├── CLAUDE.md                   # persisted design principles
├── crates/
│   ├── fleet-models/           # fluorite .fl schemas + build.rs → wire types (shared)
│   │   └── fluorite/*.fl
│   ├── fleet-store/            # Store trait + SQLite impl (storage types, NOT fluorite); watch event log
│   ├── fleet-scheduler/        # pure: schedule(unbound, workers) -> Option<WorkerName>
│   ├── fleet-apiserver/        # axum REST + watch + admission + auth; hosts controllers
│   │   └── controllers/        #   node-lifecycle, gc, reconcilers
│   ├── fleet-auth/             # Authenticator trait, bootstrap tokens, credentials, Secret newtype
│   ├── fleetlet/               # worker daemon: register, watch, reconcile, container-CLI actuator
│   │   └── runtime/            #   ContainerRuntime trait + AppleContainer impl (CLI wrapper)
│   ├── fleetctl/               # CLI client over REST
│   └── fleet-tests/            # full-stack e2e (apiserver + fake ContainerRuntime)
└── docs/superpowers/specs/     # this design doc
```

Two trait seams keep the core swappable without API changes: **`Store`** (SQLite
today, Postgres later) and **`ContainerRuntime`** (Apple `container` today, Tart /
full VMs / Linux later). Both are deep modules.

A **Web UI** (`web/`) is a planned future component, designed and implemented
separately; it consumes the TypeScript types regenerated from `fleet-models`.

## Future extensibility (additive, no API breaks)

- **Higher-level controllers**: `Deployment`/`ReplicaSet`-style replica management &
  self-healing → new resource types + controllers reusing the watch/reconcile
  machinery.
- **`ContainerRuntime` backends**: Tart (full macOS VMs), UTM, Linux — re-introducing
  a generic `Instance` abstraction at this seam.
- **Scheduling**: first-fit today; the pure `schedule()` seam absorbs affinity,
  resource-aware bin-packing, taints/tolerations.
- **HA**: swap `Store` to Postgres/raft + leader election on controllers; REST API
  unchanged.
- **Networking/Services, namespaces (multi-tenant), metrics endpoint, audit log**:
  additive resources/middleware.
- **TS/Swift clients**: regenerate from the same `.fl` for the web UI or Swift
  tooling.

## Out of scope for v1 (YAGNI)

Deployment/ReplicaSet controllers, Services/networking abstractions, namespaces,
autoscaling, HA control plane, the Web UI, and full PKI/mTLS. The resource envelope +
labels + trait seams make all of these additive later without breaking the API.
