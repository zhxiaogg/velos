import type { ContainerPhase, Lease, Worker } from "./types";

export function fmtBytes(n?: number): string {
  if (!n || n <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v % 1 === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

export function ageFrom(iso?: string, now = Date.now()): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  let s = Math.max(0, Math.floor((now - then) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  s = s % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}

export function secondsSince(iso?: string, now = Date.now()): number {
  if (!iso) return Infinity;
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return Infinity;
  return (now - then) / 1000;
}

// Phase → tailwind class fragments for badges / dots.
export const PHASE_STYLES: Record<ContainerPhase, { text: string; bg: string; dot: string; ring: string }> = {
  Pending: { text: "text-amber-300", bg: "bg-amber-400/10", dot: "bg-amber-400", ring: "ring-amber-400/30" },
  Scheduled: { text: "text-sky-300", bg: "bg-sky-400/10", dot: "bg-sky-400", ring: "ring-sky-400/30" },
  Running: { text: "text-emerald-300", bg: "bg-emerald-400/10", dot: "bg-emerald-400", ring: "ring-emerald-400/30" },
  Succeeded: { text: "text-teal-300", bg: "bg-teal-400/10", dot: "bg-teal-400", ring: "ring-teal-400/30" },
  Failed: { text: "text-rose-300", bg: "bg-rose-400/10", dot: "bg-rose-400", ring: "ring-rose-400/30" },
  Unknown: { text: "text-zinc-400", bg: "bg-zinc-400/10", dot: "bg-zinc-400", ring: "ring-zinc-400/30" },
};

export function phaseOf(c: { status?: { phase?: ContainerPhase } }): ContainerPhase {
  return c.status?.phase ?? "Unknown";
}

// A worker is Ready when it carries a true `Ready` condition.
export function isWorkerReady(w: Worker): boolean {
  return !!w.status?.conditions?.some((c) => c.conditionType === "Ready" && c.status);
}

export function leaseFor(leases: Lease[], workerName: string): Lease | undefined {
  return leases.find((l) => l.metadata.name === workerName || l.spec.holderIdentity === workerName);
}
