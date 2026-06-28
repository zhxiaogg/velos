import { Box, Cpu, MemoryStick, Server } from "lucide-react";
import { useContainers, useWorkers } from "../api";
import { Bar, Card } from "../ui";
import { fmtBytes, isWorkerReady, phaseOf, PHASE_STYLES } from "../format";
import type { ContainerPhase } from "../types";

const ACTIVE: ContainerPhase[] = ["Scheduled", "Running"];
const ORDER: ContainerPhase[] = ["Running", "Pending", "Scheduled", "Succeeded", "Failed", "Unknown"];

export function Overview() {
  const { data: workers = [] } = useWorkers();
  const { data: containers = [] } = useContainers();

  const ready = workers.filter(isWorkerReady).length;
  const running = containers.filter((c) => phaseOf(c) === "Running").length;

  // Cluster capacity vs. what scheduled/running containers have committed.
  const capCpu = workers.reduce((a, w) => a + (w.status?.allocatable?.cpu ?? 0), 0);
  const capMem = workers.reduce((a, w) => a + (w.status?.allocatable?.memoryBytes ?? 0), 0);
  const usedCpu = containers
    .filter((c) => ACTIVE.includes(phaseOf(c)))
    .reduce((a, c) => a + (c.spec.resources?.cpu ?? 1), 0);
  const usedMem = containers
    .filter((c) => ACTIVE.includes(phaseOf(c)))
    .reduce((a, c) => a + (c.spec.resources?.memoryBytes ?? 512 * 1024 ** 2), 0);

  const byPhase = ORDER.map((p) => ({ phase: p, n: containers.filter((c) => phaseOf(c) === p).length })).filter(
    (x) => x.n > 0,
  );
  const total = containers.length;

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Stat icon={<Server size={18} />} label="Workers ready" value={`${ready}/${workers.length}`} accent="text-emerald-400" />
        <Stat icon={<Box size={18} />} label="Containers" value={String(total)} accent="text-indigo-400" />
        <Stat icon={<Cpu size={18} />} label="Running now" value={String(running)} accent="text-sky-400" />
        <Stat
          icon={<MemoryStick size={18} />}
          label="Cluster memory"
          value={fmtBytes(capMem)}
          accent="text-violet-400"
        />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Card className="p-5 lg:col-span-2">
          <div className="mb-4 text-sm font-semibold text-zinc-300">Cluster allocation</div>
          <Gauge
            icon={<Cpu size={15} />}
            label="CPU"
            used={usedCpu}
            total={capCpu}
            fmt={(n) => `${n} cores`}
            color="bg-indigo-400"
          />
          <div className="h-4" />
          <Gauge
            icon={<MemoryStick size={15} />}
            label="Memory"
            used={usedMem}
            total={capMem}
            fmt={fmtBytes}
            color="bg-violet-400"
          />
        </Card>

        <Card className="p-5">
          <div className="mb-4 text-sm font-semibold text-zinc-300">Containers by phase</div>
          {total === 0 ? (
            <div className="py-6 text-center text-sm text-zinc-600">No containers yet</div>
          ) : (
            <>
              <div className="mb-4 flex h-2.5 w-full overflow-hidden rounded-full bg-white/[0.06]">
                {byPhase.map((x) => (
                  <div
                    key={x.phase}
                    className={PHASE_STYLES[x.phase].dot}
                    style={{ width: `${(x.n / total) * 100}%` }}
                    title={`${x.phase}: ${x.n}`}
                  />
                ))}
              </div>
              <div className="space-y-2">
                {byPhase.map((x) => (
                  <div key={x.phase} className="flex items-center justify-between text-sm">
                    <span className="flex items-center gap-2 text-zinc-400">
                      <span className={`h-2 w-2 rounded-full ${PHASE_STYLES[x.phase].dot}`} />
                      {x.phase}
                    </span>
                    <span className="font-mono text-zinc-200">{x.n}</span>
                  </div>
                ))}
              </div>
            </>
          )}
        </Card>
      </div>
    </div>
  );
}

function Stat({
  icon,
  label,
  value,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  accent: string;
}) {
  return (
    <Card className="p-5">
      <div className={`mb-3 ${accent}`}>{icon}</div>
      <div className="text-2xl font-semibold tracking-tight text-zinc-100">{value}</div>
      <div className="mt-1 text-xs text-zinc-500">{label}</div>
    </Card>
  );
}

function Gauge({
  icon,
  label,
  used,
  total,
  fmt,
  color,
}: {
  icon: React.ReactNode;
  label: string;
  used: number;
  total: number;
  fmt: (n: number) => string;
  color: string;
}) {
  return (
    <div>
      <div className="mb-1.5 flex items-center justify-between text-sm">
        <span className="flex items-center gap-2 text-zinc-400">
          {icon}
          {label}
        </span>
        <span className="font-mono text-zinc-300">
          {fmt(used)} <span className="text-zinc-600">/ {fmt(total)}</span>
        </span>
      </div>
      <Bar used={used} total={total} color={color} />
    </div>
  );
}
