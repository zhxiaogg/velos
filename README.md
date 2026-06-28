# Velos

**Velos** is a control plane for running containers across a pool of registered
worker machines, exposed over a RESTful API. You declare the containers you want;
Velos schedules them onto healthy workers, runs them through a container runtime,
and continuously reconciles their actual state back toward what you asked for.

The architecture is runtime- and OS-agnostic: workers talk to the control plane
over HTTP and execute containers through a pluggable runtime interface. The
current runtime backend is [Apple Containerization](https://github.com/apple/containerization)
(lightweight Linux micro-VMs); additional runtimes and platforms are a planned
direction.

```
   velosctl ─┐                  ┌──────────────────────────────┐
   (CLI)     │                  │         velos-apiserver        │
   dashboard ├───  REST  ──────▶│  REST API · scheduler ·        │
   (browser) │   (Bearer)       │  reconciliation · web UI       │
             │                  │  SQLite-backed object store    │
             ▼                  └───────────────▲────────────────┘
                                                │ register · lease · status
                                      ┌─────────┴──────────┐
                                      │      veloslet       │  one per worker
                                      │   reconcile loop    │
                                      │  ContainerRuntime ──┼──▶ container runtime
                                      └─────────────────────┘
```

## Components

- **`velos-apiserver`** — the control plane. Serves the REST API, persists objects
  in SQLite, runs the scheduler and reconciliation loops, and serves the web
  dashboard (embedded in the binary).
- **`veloslet`** — the per-worker agent. Registers its machine, renews a lease to
  prove liveness, and reconciles its assigned containers against the runtime.
- **`velosctl`** — a command-line client for the API. `velosctl login` saves an
  admin token (and server URL) to `~/.velos/config` for subsequent calls.
- **Web dashboard** — a React UI for first-run admin setup, watching workers and
  containers, launching workloads, and managing CLI tokens, served directly by the
  apiserver.

## Resource model

Velos manages three object types, each with `metadata` / `spec` / `status`, served
under `/api/v1/{plural}`:

- **Container** — a workload. Its phase moves `Pending → Scheduled → Running →
  Succeeded | Failed`, or `Unknown` when its node's state is lost.
- **Worker** — a registered machine, with its capacity and a `Ready` condition.
- **Lease** — a worker's periodic heartbeat; a stale lease marks its worker
  `NotReady`.

## Getting started

Install with cargo:

```bash
cargo install velos-apiserver velosctl veloslet
```

…or build from source with `make build` (which also builds the embedded dashboard).

Then follow **[docs/getting-started.md](docs/getting-started.md)** for the full
walkthrough: start the control plane, set up the admin account and connect
`velosctl`, register a worker, launch containers, and open the dashboard at
`http://127.0.0.1:8080`. On first run the dashboard prompts you to create the
admin account; from there you mint a CLI token for `velosctl`. (Running a worker
currently requires the Apple `container` CLI; the control plane, CLI, and
dashboard do not.)

## Development

```bash
make build     # build the web UI + workspace
make web       # rebuild just the web UI (embedded by the apiserver)
make test      # cargo test --workspace
make check     # fmt --check + clippy -D warnings + test  (pre-PR gate)
make run       # run the apiserver
```

Engineering conventions and the design philosophy live in [`CLAUDE.md`](CLAUDE.md).

## License

MIT — see [LICENSE](LICENSE).
