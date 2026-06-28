# Velos Web UX

A PoC dashboard for the Velos control plane — TypeScript + React + Vite + Tailwind v4.

## What it shows

- **Overview** — workers ready, container counts, running now, cluster CPU/memory
  allocation gauges, and a containers-by-phase breakdown.
- **Workers** — one card per registered node: Ready/NotReady status, runtime
  version, live CPU/memory allocation, slot usage, and lease freshness. Click a
  card for a detail drawer (capacity, addresses, lease, containers on the node,
  raw object).
- **Containers** — filterable table (by phase) with phase badges, image, node,
  resources, labels, age, a row detail drawer, **Launch container** (create), and
  per-row delete.

Data auto-refreshes every 2s.

## How auth works (no Rust changes, token-free browser)

The Velos apiserver requires a worker credential on every `/api/v1/*` call. The
Vite dev server handles this entirely:

1. A small plugin (`velosAuth` in `vite.config.ts`) mints a bootstrap token via
   the open `POST /auth/v1/tokens`, exchanges it at `POST /auth/v1/register` for a
   durable credential under the identity `velos-dashboard`, then deletes that
   identity's `Worker` object (the credential survives) so it never pollutes the
   workers list.
2. All browser traffic goes to a same-origin `/velos/*` prefix, which the dev
   server proxies to `http://127.0.0.1:8080`, injecting `Authorization: Bearer
   <credential>`. The browser never sees a token and there are no CORS concerns.

Point at a different apiserver with `VELOS_SERVER=http://host:port npm run dev`.

## Run

Prerequisites: the apiserver running on `127.0.0.1:8080` with at least one
registered worker (a `veloslet`).

```bash
npm install
npm run dev      # http://localhost:5173
```
