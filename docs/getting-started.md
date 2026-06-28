# Getting started with Velos

This guide walks through running Velos end-to-end: install it, start the control
plane, register a worker, launch containers from the CLI or the dashboard, and
understand what happens under the hood.

- [1. Prerequisites](#1-prerequisites)
- [2. Install](#2-install)
- [3. Start the control plane](#3-start-the-control-plane)
- [4. Register a worker](#4-register-a-worker)
- [5. Use the CLI (velosctl)](#5-use-the-cli-velosctl)
- [6. Use the dashboard](#6-use-the-dashboard)
- [7. The container lifecycle](#7-the-container-lifecycle)
- [8. Authentication](#8-authentication)
- [9. Troubleshooting](#9-troubleshooting)
- [10. Tearing down](#10-tearing-down)

---

## 1. Prerequisites

| Requirement | Needed for |
|---|---|
| **Rust** (stable) | Building from source. Pinned by `rust-toolchain.toml`. |
| **Node.js 18+** + npm | Building the dashboard from source (not needed if you `cargo install`). |
| **Apple `container` CLI** | Running a **worker** — this is the current container runtime backend. The control plane, CLI, and dashboard don't need it. |
| **jq** | Used by the token-minting snippet below. |

The control plane and clients are runtime-agnostic; only the worker executes
containers, and today it does so through Apple Containerization. Check the
runtime on a machine that will host workloads:

```bash
container --version
```

## 2. Install

### Via cargo

```bash
cargo install velos-apiserver   # control plane (the web dashboard is built in)
cargo install velosctl          # CLI
cargo install veloslet          # worker agent
```

### From source

```bash
git clone https://github.com/zhxiaogg/velos
cd velos
make build      # builds the web UI, then all binaries into target/debug/
```

The rest of this guide uses bare command names (`velos-apiserver`, `velosctl`,
`veloslet`); if you built from source, run them from `./target/debug/` or add
that directory to your `PATH`.

## 3. Start the control plane

```bash
velos-apiserver
```

- Listens on **`127.0.0.1:8080`** and serves both the API and the **web
  dashboard** (open `http://127.0.0.1:8080`).
- Creates a SQLite database **`velos.db`** in the working directory.
- Runs the scheduler (every ~2s) and the worker-health controller (every ~5s).

Control log verbosity with `RUST_LOG`, e.g. `RUST_LOG=info velos-apiserver`.

Leave it running and open a new terminal for the next steps.

## 4. Register a worker

Registration is a two-step, fail-closed flow: mint a short-lived *bootstrap
token*, then hand it to `veloslet`, which exchanges it for a durable worker
credential on first start.

```bash
# Mint a bootstrap token and assemble it as `tokenId.secret`.
TOKEN=$(velosctl token create | jq -r '"\(.tokenId).\(.secret)"')

# Start the worker agent. It registers on first start, then renews its lease.
veloslet --node "$(hostname -s)" --token "$TOKEN"
```

`veloslet` flags:

| Flag | Default | Meaning |
|---|---|---|
| `--server` | `http://127.0.0.1:8080` | control-plane base URL |
| `--node` | *(required)* | this worker's unique name |
| `--token` | — | bootstrap token, needed only for first registration |
| `--reconcile-secs` | `5` | how often it reconciles its containers |
| `--heartbeat-secs` | `10` | how often it renews its lease |
| `--lease-secs` | `40` | lease duration; not renewed in time → worker goes `NotReady` |

Within a few seconds the worker reports **Ready** (its lease is fresh).

## 5. Use the CLI (velosctl)

`velosctl` talks to the API and needs a credential for `/api/v1/*` calls — pass
one with `--token`. (The easiest credential to grab is one the dashboard mints,
or register a throwaway identity; see [§8](#8-authentication).)

```bash
# List / get
velosctl --token "$CRED" get workers
velosctl --token "$CRED" get containers
velosctl --token "$CRED" get container my-job
velosctl --token "$CRED" get containers --selector app=demo

# Create from a JSON file (status.phase MUST be "Pending" to be scheduled)
cat > job.json <<'JSON'
{
  "metadata": { "name": "my-job", "labels": { "app": "demo" } },
  "spec": {
    "image": "docker.io/library/alpine:latest",
    "command": ["sleep", "600"],
    "resources": { "cpu": 1, "memoryBytes": 268435456 },
    "restartPolicy": "Never"
  },
  "status": { "phase": "Pending" }
}
JSON
velosctl --token "$CRED" apply container --file job.json

# Delete
velosctl --token "$CRED" delete container my-job
```

> **Why `status.phase: "Pending"`:** the scheduler only places containers whose
> phase is `Pending`. The dashboard sets this for you.

## 6. Use the dashboard

The dashboard is served by the apiserver — just open **`http://127.0.0.1:8080`**.
It gives you:

- **Overview** — workers ready, container counts, cluster CPU/memory allocation,
  and a containers-by-phase breakdown.
- **Workers** — per-node cards (Ready status, runtime version, live allocation,
  slot usage, lease freshness) with a detail drawer.
- **Containers** — a phase-filterable table with a **Launch container** form,
  per-row delete, and a detail drawer.

Data refreshes every 2 seconds.

To iterate on the UI itself, run the Vite dev server (it proxies the API to the
apiserver for hot-reload):

```bash
cd web && npm install && npm run dev      # http://localhost:5173
```

## 7. The container lifecycle

1. **`Pending`** — created via the API with `status.phase: Pending`.
2. **`Scheduled`** — the scheduler binds it to a Ready worker with capacity and
   sets `spec.nodeName`.
3. **`Running`** — that worker's `veloslet` starts the container via the runtime
   and reports its ID.
4. **`Succeeded` / `Failed`** — when the process exits (0 vs non-zero); the
   `restartPolicy` (`Never` / `OnFailure` / `Always`) decides whether it restarts.

If a worker's lease goes stale, the health controller marks it `NotReady`; after
a grace period its containers are evicted (rescheduled if labeled
`velos.io/reschedulable=true`, otherwise marked `Unknown`).

## 8. Authentication

Velos is fail-closed: every `/api/v1/*` request needs a valid worker credential.

- `POST /auth/v1/tokens` — mints a bootstrap token.
- `POST /auth/v1/register` — exchanges a bootstrap token for a durable credential
  (`workerName.secret`) and creates the `Worker` object.
- A worker credential can list all workers/containers/leases and manage
  containers; it may only address its *own* Worker/Lease object by name.

The **dashboard** currently obtains a credential itself: it mints a bootstrap
token, registers an identity named `velos-dashboard`, then deletes that identity's
Worker object (the credential is stored separately and keeps working), and caches
the credential in the browser.

> This open bootstrap flow is an interim placeholder. Real authentication between
> the dashboard and the control plane — and authenticated worker-token vending —
> is a planned change.

## 9. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `{"error":"unauthorized"}` from the API | You used a **bootstrap token** on `/api/v1/*`. Those need a **worker credential** (`workerName.secret`) from `/auth/v1/register`. |
| Container stuck in `Pending` | Created without `status.phase: "Pending"`, or no worker is `Ready` / has capacity. |
| Container goes straight to `Failed` | The runtime couldn't run it (image pull failed, or the `container` CLI is missing on the worker). Check the `veloslet` logs. |
| Worker shows `NotReady` | `veloslet` isn't renewing its lease — confirm it's running and can reach the apiserver. |
| Dashboard says "apiserver unreachable" | The apiserver isn't running, or you opened the dev server while the apiserver is down. |
| `address already in use` on start | Something already holds `:8080` — `lsof -nP -iTCP:8080 -sTCP:LISTEN`. |

## 10. Tearing down

```bash
# Stop the processes (Ctrl-C in their terminals, or:)
pkill -f velos-apiserver
pkill -f veloslet

# Reset all control-plane state
rm -f velos.db
```
