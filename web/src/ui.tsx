import type { ReactNode } from "react";
import { PHASE_STYLES } from "./format";
import type { ContainerPhase } from "./types";

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`rounded-xl border border-white/5 bg-white/[0.02] shadow-lg shadow-black/20 ${className}`}
    >
      {children}
    </div>
  );
}

export function PhaseBadge({ phase }: { phase: ContainerPhase }) {
  const s = PHASE_STYLES[phase];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ring-1 ring-inset ${s.bg} ${s.text} ${s.ring}`}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full ${s.dot} ${phase === "Running" ? "live-dot" : ""}`}
      />
      {phase}
    </span>
  );
}

export function StatusDot({ ok, label }: { ok: boolean; label: string }) {
  return (
    <span className="inline-flex items-center gap-2">
      <span
        className={`h-2 w-2 rounded-full ${ok ? "bg-emerald-400 live-dot" : "bg-rose-500"}`}
      />
      <span className={ok ? "text-emerald-300" : "text-rose-300"}>{label}</span>
    </span>
  );
}

// Horizontal usage bar: `used` of `total`.
export function Bar({ used, total, color = "bg-indigo-400" }: { used: number; total: number; color?: string }) {
  const pct = total > 0 ? Math.min(100, (used / total) * 100) : 0;
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/[0.06]">
      <div className={`h-full rounded-full ${color} transition-all duration-500`} style={{ width: `${pct}%` }} />
    </div>
  );
}

export function Labels({ labels }: { labels?: Record<string, string> }) {
  const entries = Object.entries(labels ?? {});
  if (entries.length === 0) return <span className="text-zinc-600">—</span>;
  return (
    <div className="flex flex-wrap gap-1">
      {entries.map(([k, v]) => (
        <span
          key={k}
          className="rounded bg-white/[0.04] px-1.5 py-0.5 font-mono text-[11px] text-zinc-400 ring-1 ring-inset ring-white/5"
        >
          {k}={v}
        </span>
      ))}
    </div>
  );
}

export function Spinner() {
  return (
    <div className="flex items-center justify-center py-16 text-zinc-500">
      <svg className="h-5 w-5 animate-spin" viewBox="0 0 24 24" fill="none">
        <circle className="opacity-20" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
        <path className="opacity-80" d="M22 12a10 10 0 0 1-10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
      </svg>
    </div>
  );
}

export function EmptyState({ icon, title, hint }: { icon: ReactNode; title: string; hint?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
      <div className="text-zinc-600">{icon}</div>
      <div className="text-sm font-medium text-zinc-400">{title}</div>
      {hint && <div className="max-w-sm text-xs text-zinc-600">{hint}</div>}
    </div>
  );
}
