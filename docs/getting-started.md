# Getting started with Velos

This guide walks through running Velos end-to-end: install it, start the control
plane, set up the admin account, connect `velosctl`, register a worker, launch
containers, and understand what happens under the hood.

- [1. Prerequisites](#1-prerequisites)
- [2. Install](#2-install)
- [3. Start the control plane](#3-start-the-control-plane)
- [4. First-run setup & connecting velosctl](#4-first-run-setup--connecting-velosctl)
- [5. Register a worker](#5-register-a-worker)
- [6. Use the CLI (velosctl)](#6-use-the-cli-velosctl)
- [7. Use the dashboard](#7-use-the-dashboard)
- [8. The container lifecycle](#8-the-container-lifecycle)
- [9. Authentication](#9-authentication)
- [10. Troubleshooting](#10-troubleshooting)
- [11. Tearing down](#11-tearing-down)

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

A freshly started server is **uninitialized** and fails closed: every route
except the first-run setup is rejected until you create the admin account
(§4). Leave the apiserver running and open a new terminal for the next steps.

## 4. First-run setup & connecting velosctl

Velos has one **admin** account, created once on first run, plus per-worker
identities (§9). The admin is set up through the dashboard, which then mints the
**CLI token** that `velosctl` carries.

1. Open **`http://127.0.0.1:8080`**. On first run it shows a **Setup** screen —
   choose an admin username and password. (The password is hashed with argon2 and
   never leaves the server; setup works only while the server is uninitialized.)
2. You're signed in. Go to the **Tokens** tab → **Create CLI token**, give it a
   label (e.g. `laptop`), and **copy the token — it is shown only once.**
3. Hand that token to `velosctl`:

```bash
velosctl login --token <PASTE_TOKEN> --server http://127.0.0.1:8080
```

`login` validates the token against the server, then saves the **server and
token** to `~/.velos/config` (mode `0600`). After this, plain commands need no
flags:

```bash
velosctl get workers     # uses the saved server + token
velosctl logout          # forget the saved credential
```

Resolution precedence, highest first:

| Value | Order |
|---|---|
| token | `--token` flag → `VELOS_TOKEN` env → `~/.velos/config` |
| server | `--server` flag → `VELOS_SERVER` env → `~/.velos/config` → `http://127.0.0.1:8080` |

> Prefer the CLI without a browser? You can drive setup over HTTP directly:
> `curl -X POST :8080/auth/v1/setup -d '{"username":"admin","password":"…"}'`,
> then `curl -X POST :8080/auth/v1/login …` for a session token and
> `POST /auth/v1/admin/tokens {"label":"laptop"}` for a CLI token. See §9.

## 5. Register a worker

Worker registration is a fail-closed, two-step flow: an **admin** mints a
short-lived *bootstrap token*, then `veloslet` exchanges it for a durable,
node-scoped worker credential on first start.

```bash
# As the logged-in admin, mint a bootstrap token and assemble it as `tokenId.secret`.
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

Within a few seconds the worker reports **Ready** (its lease is fresh):

```bash
velosctl get workers
```

## 6. Use the CLI (velosctl)

Once you've run `velosctl login` (§4), commands carry your admin credential
automatically — no `--token` needed.

```bash
# List / get
velosctl get workers
velosctl get containers
velosctl get container my-job
velosctl get containers --selector app=demo

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
velosctl apply container --file job.json

# Delete
velosctl delete container my-job
```

> **Why `status.phase: "Pending"`:** the scheduler only places containers whose
> phase is `Pending`. The dashboard sets this for you.

For a one-off against a different server or with a different identity, override
per-command: `velosctl --server http://other:8080 --token <tok> get workers`.

## 7. Use the dashboard

The dashboard is served by the apiserver — just open **`http://127.0.0.1:8080`**.
After signing in (§4) it gives you:

- **Overview** — workers ready, container counts, cluster CPU/memory allocation,
  and a containers-by-phase breakdown.
- **Workers** — per-node cards (Ready status, runtime version, live allocation,
  slot usage, lease freshness) with a detail drawer.
- **Containers** — a phase-filterable table with a **Launch container** form,
  per-row delete, and a detail drawer.
- **Tokens** — create, list, and revoke the CLI tokens that `velosctl` uses.

Data refreshes every 2 seconds. **Sign out** from the header clears the browser
session. To iterate on the UI itself, run the Vite dev server (it proxies the API
to the apiserver for hot-reload):

```bash
cd web && npm install && npm run dev      # http://localhost:5173
```

## 8. The container lifecycle

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

## 9. Authentication

Velos is fail-closed and recognizes two kinds of identity:

- **Admin** — full access to all resources and to the privileged auth endpoints.
  There is one admin account (username + argon2-hashed password), created once
  via first-run setup.
- **Worker** — a registered machine. A worker credential can read all
  workers/containers/leases and manage containers, but may only address its *own*
  Worker/Lease object by name.

**Initialization gate.** Until the admin account exists, the server is
*uninitialized*: only `GET /auth/v1/status` and `POST /auth/v1/setup` are
reachable; everything else returns `401`. `setup` is single-shot — once an admin
exists it returns `409`.

**Admin tokens.** Both the dashboard session and `velosctl`'s credential are the
same primitive: a random opaque token, persisted only as a hash and looked up on
each request. Logging in returns a short-lived **session token** (held by the
browser); the **Tokens** page mints long-lived **CLI tokens** (the GitHub
personal-access-token model). Revoking a token in the dashboard takes effect
immediately. `velosctl login` stores its token + server in `~/.velos/config`
(`0600`).

**Worker credentials.** An admin mints a bootstrap token
(`POST /auth/v1/tokens`); `veloslet` exchanges it (`POST /auth/v1/register`) for a
durable `workerName.secret` credential and the server creates the `Worker` object.

Auth endpoints at a glance:

| Endpoint | Who | Purpose |
|---|---|---|
| `GET /auth/v1/status` | open | `{ "initialized": bool }` |
| `POST /auth/v1/setup` | open *(uninitialized only)* | create the admin account |
| `POST /auth/v1/login` | open | username+password → session token |
| `GET /auth/v1/me` | any valid token | echo the caller's identity |
| `GET/POST /auth/v1/admin/tokens`, `DELETE …/{id}` | **admin** | list / create / revoke CLI tokens |
| `POST /auth/v1/tokens` | **admin** | mint a worker bootstrap token |
| `POST /auth/v1/register` | bootstrap token | join → worker credential |

> Identity is resolved behind a `TokenVerifier` seam, so an external OIDC provider
> can be integrated later (validate a JWT against the provider) without changing
> any endpoint. Single-admin and the two-tier model are the current scope.

## 10. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `{"error":"unauthorized"}` from `/api/v1/*` | Not logged in — run `velosctl login` (§4) — or the token was revoked/expired, or the server isn't set up yet (`GET /auth/v1/status`). |
| `401` on everything, even `/auth/v1/login` | Server is **uninitialized**; complete first-run setup in the dashboard (§4). |
| `409` from `/auth/v1/setup` | The admin already exists; use **login**, not setup. |
| `velosctl token create` → `403`/`401` | Bootstrap minting is **admin-only**; log in first (§4). |
| Container stuck in `Pending` | Created without `status.phase: "Pending"`, or no worker is `Ready` / has capacity. |
| Container goes straight to `Failed` | The runtime couldn't run it (image pull failed, or the `container` CLI is missing on the worker). Check the `veloslet` logs. |
| Worker shows `NotReady` | `veloslet` isn't renewing its lease — confirm it's running and can reach the apiserver. |
| Dashboard says "apiserver unreachable" | The apiserver isn't running, or you opened the dev server while the apiserver is down. |
| `address already in use` on start | Something already holds `:8080` — `lsof -nP -iTCP:8080 -sTCP:LISTEN`. |

## 11. Tearing down

```bash
# Stop the processes (Ctrl-C in their terminals, or:)
pkill -f velos-apiserver
pkill -f veloslet

# Forget the saved CLI credential
velosctl logout

# Reset all control-plane state (including the admin account)
rm -f velos.db
```
