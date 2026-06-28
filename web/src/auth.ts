// Browser-side credential handling.
//
// Velos requires a worker credential on every /api/v1 call. Until real operator
// auth exists, the dashboard obtains one itself via the open bootstrap flow and
// caches it in localStorage:
//
//   1. POST /auth/v1/tokens          -> a short-lived bootstrap token
//   2. POST /auth/v1/register        -> a durable credential for `velos-dashboard`
//   3. DELETE /api/v1/workers/...    -> remove that identity's Worker object so it
//                                       never shows up in the workers list (the
//                                       credential is stored separately and lives on)
//
// This runs the same whether the bundle is served by the apiserver (same-origin)
// or by `npm run dev` (Vite proxies the paths). When a credential is rejected
// (e.g. the server's database was reset), it is discarded and re-minted.

const STORAGE_KEY = "velos.credential";
const DASHBOARD_IDENTITY = "velos-dashboard";

let inflight: Promise<string> | null = null;

function cached(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function store(cred: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, cred);
  } catch {
    /* private mode / disabled storage: fall back to in-memory only */
  }
}

export function forgetCredential(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
}

async function mint(): Promise<string> {
  const tok = await fetch("/auth/v1/tokens", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ ttlSeconds: 86400 }),
  }).then((r) => r.json());
  const bootstrap = `${tok.tokenId}.${tok.secret}`;

  const reg = await fetch("/auth/v1/register", {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${bootstrap}` },
    body: JSON.stringify({
      name: DASHBOARD_IDENTITY,
      capacity: {},
      addresses: [],
      containerRuntimeVersion: "dashboard",
    }),
  }).then((r) => r.json());
  const cred: string = reg.token;

  // Keep the dashboard identity out of the workers list. The credential is
  // stored independently of the Worker object, so it keeps working.
  await fetch(`/api/v1/workers/${DASHBOARD_IDENTITY}`, {
    method: "DELETE",
    headers: { authorization: `Bearer ${cred}` },
  }).catch(() => {});

  store(cred);
  return cred;
}

/** Return a usable credential, minting and caching one if needed. */
export function getCredential(): Promise<string> {
  const existing = cached();
  if (existing) return Promise.resolve(existing);
  if (!inflight) inflight = mint().finally(() => (inflight = null));
  return inflight;
}
