// Browser-side admin session handling.
//
// The dashboard authenticates as the Velos admin: on first run it sets the
// admin username/password (POST /auth/v1/setup), thereafter it logs in
// (POST /auth/v1/login) and stores the returned short-lived session token in
// localStorage, sending it as a Bearer on every API call. A 401 drops the token
// and the app shell falls back to the login screen.

const STORAGE_KEY = "velos.session";
const listeners = new Set<() => void>();

export function sessionToken(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function setToken(tok: string | null): void {
  try {
    if (tok) localStorage.setItem(STORAGE_KEY, tok);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* private mode / disabled storage: ignore */
  }
  listeners.forEach((l) => l());
}

/** Subscribe to session changes (login/logout). Returns an unsubscribe fn. */
export function onAuthChange(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

export async function getStatus(): Promise<{ initialized: boolean }> {
  const r = await fetch("/auth/v1/status");
  if (!r.ok) throw new Error(`status ${r.status}`);
  return r.json();
}

async function postJson(path: string, body: unknown): Promise<Response> {
  return fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export async function setup(username: string, password: string): Promise<void> {
  const r = await postJson("/auth/v1/setup", { username, password });
  if (!r.ok) {
    const detail = (await r.json().catch(() => ({})))?.error ?? `setup failed (${r.status})`;
    throw new Error(detail);
  }
}

export async function login(username: string, password: string): Promise<void> {
  const r = await postJson("/auth/v1/login", { username, password });
  if (!r.ok) throw new Error("invalid username or password");
  const { token } = await r.json();
  setToken(token);
}

export function logout(): void {
  setToken(null);
}
