import { useState } from "react";
import { Copy, KeyRound, Trash2 } from "lucide-react";
import { useCreateToken, useRevokeToken, useTokens } from "../api";
import { Card } from "../ui";

/// Admin CLI-token management: create a named token (shown once), list, revoke.
export function Tokens() {
  const { data: allTokens = [] } = useTokens();
  // Show only CLI tokens; UI session tokens are an internal detail.
  const tokens = allTokens.filter((t) => t.kind === "cli");
  const create = useCreateToken();
  const revoke = useRevokeToken();
  const [label, setLabel] = useState("");
  const [secret, setSecret] = useState<string | null>(null);

  const onCreate = async () => {
    const l = label.trim();
    if (!l) return;
    const r = await create.mutateAsync(l);
    setSecret(r.token);
    setLabel("");
  };

  return (
    <div className="space-y-6">
      <Card className="p-5">
        <div className="flex items-center gap-3">
          <input
            className="flex-1 rounded-lg border border-white/5 bg-black/30 px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-500/50"
            placeholder="Token label, e.g. laptop or ci-runner"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && onCreate()}
          />
          <button
            onClick={onCreate}
            disabled={create.isPending || !label.trim()}
            className="inline-flex items-center gap-2 rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-indigo-500 disabled:opacity-50"
          >
            <KeyRound size={16} />
            Create CLI token
          </button>
        </div>

        {secret && (
          <div className="mt-4 rounded-lg border border-amber-500/30 bg-amber-500/[0.06] p-4 text-sm">
            <p className="mb-2 font-medium text-amber-300">
              Copy this token now — it will not be shown again.
            </p>
            <div className="flex items-center gap-2">
              <code className="flex-1 break-all rounded bg-black/40 px-3 py-2 text-xs text-zinc-200">
                {secret}
              </code>
              <button
                onClick={() => navigator.clipboard?.writeText(secret)}
                className="rounded-lg border border-white/10 p-2 text-zinc-300 hover:bg-white/5"
                title="Copy"
              >
                <Copy size={16} />
              </button>
            </div>
            <p className="mt-2 text-zinc-400">
              Use it with:{" "}
              <code className="text-zinc-300">velosctl login --token &lt;token&gt; --server {window.location.origin}</code>
            </p>
          </div>
        )}
      </Card>

      <Card>
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-white/5 text-left text-xs uppercase tracking-wide text-zinc-500">
              <th className="px-5 py-3 font-medium">Label</th>
              <th className="px-5 py-3 font-medium">Kind</th>
              <th className="px-5 py-3 font-medium">Expires</th>
              <th className="px-5 py-3" />
            </tr>
          </thead>
          <tbody>
            {tokens.length === 0 && (
              <tr>
                <td colSpan={4} className="px-5 py-8 text-center text-zinc-600">
                  No tokens yet.
                </td>
              </tr>
            )}
            {tokens.map((t) => (
              <tr key={t.id} className="border-b border-white/5 last:border-0">
                <td className="px-5 py-3 text-zinc-200">{t.label}</td>
                <td className="px-5 py-3 text-zinc-400">{t.kind}</td>
                <td className="px-5 py-3 text-zinc-500">{t.expiresAt}</td>
                <td className="px-5 py-3 text-right">
                  <button
                    onClick={() => revoke.mutate(t.id)}
                    className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs text-rose-400 hover:bg-rose-500/10"
                  >
                    <Trash2 size={14} />
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}
