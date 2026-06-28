import { useEffect, useState } from "react";
import { getStatus, login, onAuthChange, sessionToken, setup } from "../auth";

/// Gates the dashboard behind admin auth: first-run setup, then login. Renders
/// `children` only once a session token is present.
export function AuthGate({ children }: { children: React.ReactNode }) {
  const [token, setTok] = useState<string | null>(sessionToken());
  const [initialized, setInitialized] = useState<boolean | null>(null);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => onAuthChange(() => setTok(sessionToken())), []);
  useEffect(() => {
    getStatus()
      .then((s) => setInitialized(s.initialized))
      .catch(() => setInitialized(true));
  }, []);

  if (token) return <>{children}</>;
  if (initialized === null) {
    return <div className="grid h-full place-items-center text-sm text-zinc-500">Loading…</div>;
  }

  const isSetup = !initialized;
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (isSetup) {
        await setup(username, password);
        setInitialized(true);
      }
      await login(username, password);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid h-full place-items-center px-4">
      <form
        onSubmit={submit}
        className="w-full max-w-sm space-y-5 rounded-2xl border border-white/5 bg-white/[0.02] p-8 shadow-2xl shadow-black/40"
      >
        <div className="flex items-center gap-2.5">
          <div className="grid h-9 w-9 place-items-center rounded-xl bg-gradient-to-br from-indigo-500 to-violet-600 font-bold text-white shadow-lg shadow-indigo-500/30">
            V
          </div>
          <div>
            <div className="text-sm font-semibold tracking-tight text-zinc-100">Velos</div>
            <div className="text-[11px] text-zinc-500">
              {isSetup ? "set up the admin account" : "sign in"}
            </div>
          </div>
        </div>

        <div className="space-y-3">
          <input
            className="w-full rounded-lg border border-white/5 bg-black/30 px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-500/50"
            placeholder="username"
            value={username}
            autoComplete="username"
            onChange={(e) => setUsername(e.target.value)}
          />
          <input
            className="w-full rounded-lg border border-white/5 bg-black/30 px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-500/50"
            type="password"
            placeholder="password"
            value={password}
            autoComplete={isSetup ? "new-password" : "current-password"}
            onChange={(e) => setPassword(e.target.value)}
          />
        </div>

        {error && <p className="text-sm text-rose-400">{error}</p>}

        <button
          type="submit"
          disabled={busy || !password}
          className="w-full rounded-lg bg-indigo-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-indigo-500 disabled:opacity-50"
        >
          {busy ? "…" : isSetup ? "Create admin & sign in" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
