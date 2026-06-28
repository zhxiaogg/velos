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

## How it's served

`npm run build` emits the bundle into `../crates/apiserver/ui`, which the
**apiserver embeds and serves itself** (via `rust-embed`). In production there is
no separate web process — `velos-server` serves the dashboard same-origin
alongside the API, and unknown paths fall back to `index.html` for client-side
routing. A `cargo install velos-server` therefore ships a working UI.

## Auth (interim)

Velos requires a worker credential on every `/api/v1/*` call, and real operator
auth is not built yet. Until then the browser obtains a credential itself
(`src/auth.ts`): it mints a bootstrap token (`POST /auth/v1/tokens`), exchanges
it for a durable credential under the identity `velos-dashboard`
(`POST /auth/v1/register`), then deletes that identity's `Worker` object so it
never appears in the workers list (the credential is stored separately and lives
on). The credential is cached in `localStorage` and re-minted if rejected. This
runs identically whether the bundle is served by the apiserver or by `npm run
dev`.

> This open bootstrap flow is a placeholder. A future change adds real
> authentication between the dashboard and the server.

## Develop

The apiserver serves the built bundle, so for production you only need
`make build` (or `make web`) and then run `velos-server`. For a fast
edit/reload loop, run the Vite dev server, which proxies the API/auth paths to
the apiserver (purely to avoid CORS):

```bash
npm install
npm run dev      # http://localhost:5173, proxies to the apiserver on :8080
```

Point the proxy at a different apiserver with `VELOS_SERVER=http://host:port npm run dev`.
