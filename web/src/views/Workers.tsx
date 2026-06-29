import { useState } from "react";
import { Server } from "lucide-react";
import { useContainers, useLeases, useWorkers } from "../api";
import { Bar, Card, EmptyState, Labels, Spinner, StatusDot } from "../ui";
import { Drawer, Field, Json } from "../components/Drawer";
import { ageFrom, fmtBytes, isWorkerReady, leaseFor, phaseOf, secondsSince } from "../format";
import type { Worker } from "../types";

export function Workers() {
  const { data: workers, isLoading } = useWorkers();
  const { data: leases = [] } = useLeases();
  const { data: containers = [] } = useContainers();
  const [selected, setSelected] = useState<Worker | null>(null);

  if (isLoading) return <Spinner />;
  if (!workers || workers.length === 0)
    return (
      <Card className="p-2">
        <EmptyState
          icon={<Server size={32} />}
          title="No workers registered"
          hint="Start a veloslet pointed at this server with a bootstrap token to register a node."
        />
      </Card>
    );

  return (
    <>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
        {workers.map((w) => {
          const ready = isWorkerReady(w);
          const lease = leaseFor(leases, w.metadata.name);
          const leaseAge = secondsSince(lease?.spec.renewTime);
          const onNode = containers.filter((c) => c.spec.nodeName === w.metadata.name);
          const active = onNode.filter((c) => ["Scheduled", "Running"].includes(phaseOf(c)));
          const cpuCap = w.status?.allocatable?.cpu ?? 0;
          const memCap = w.status?.allocatable?.memoryBytes ?? 0;
          const cpuUsed = active.reduce((a, c) => a + (c.spec.resources?.cpu ?? 1), 0);
          const memUsed = active.reduce((a, c) => a + (c.spec.resources?.memoryBytes ?? 512 * 1024 ** 2), 0);

          return (
            <Card
              key={w.metadata.uid ?? w.metadata.name}
              className="cursor-pointer p-5 transition hover:border-white/10 hover:bg-white/[0.04]"
            >
              <div onClick={() => setSelected(w)}>
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-2.5">
                    <div className="rounded-lg bg-white/5 p-2 text-zinc-400">
                      <Server size={16} />
                    </div>
                    <div className="min-w-0">
                      <div className="truncate font-medium text-zinc-100">{w.metadata.name}</div>
                      <div className="text-[11px] text-zinc-500">
                        {w.status?.containerRuntimeVersion ?? "unknown runtime"}
                      </div>
                    </div>
                  </div>
                  <StatusDot ok={ready} label={ready ? "Ready" : "NotReady"} />
                </div>

                <div className="mt-4 space-y-3">
                  <Usage label="CPU" used={`${cpuUsed}`} total={`${cpuCap}`} u={cpuUsed} t={cpuCap} color="bg-indigo-400" />
                  <Usage
                    label="Memory"
                    used={fmtBytes(memUsed)}
                    total={fmtBytes(memCap)}
                    u={memUsed}
                    t={memCap}
                    color="bg-violet-400"
                  />
                </div>

                <div className="mt-4 flex items-center justify-between border-t border-white/5 pt-3 text-xs text-zinc-500">
                  <span>
                    <span className="font-mono text-zinc-300">{active.length}</span> running
                  </span>
                  <span>
                    lease{" "}
                    <span className={leaseAge < (lease?.spec.leaseDurationSeconds ?? 40) ? "text-emerald-400" : "text-rose-400"}>
                      {lease ? `${ageFrom(lease.spec.renewTime)} ago` : "none"}
                    </span>
                  </span>
                </div>
              </div>
            </Card>
          );
        })}
      </div>

      <WorkerDrawer worker={selected} leases={leases} onClose={() => setSelected(null)} containers={containers} />
    </>
  );
}

function Usage({
  label,
  used,
  total,
  u,
  t,
  color,
}: {
  label: string;
  used: string;
  total: string;
  u: number;
  t: number;
  color: string;
}) {
  return (
    <div>
      <div className="mb-1 flex justify-between text-xs">
        <span className="text-zinc-500">{label}</span>
        <span className="font-mono text-zinc-400">
          {used} <span className="text-zinc-600">/ {total}</span>
        </span>
      </div>
      <Bar used={u} total={t} color={color} />
    </div>
  );
}

function WorkerDrawer({
  worker,
  leases,
  containers,
  onClose,
}: {
  worker: Worker | null;
  leases: import("../types").Lease[];
  containers: import("../types").Container[];
  onClose: () => void;
}) {
  if (!worker) return <Drawer open={false} onClose={onClose} title="" children={null} />;
  const ready = isWorkerReady(worker);
  const lease = leaseFor(leases, worker.metadata.name);
  const onNode = containers.filter((c) => c.spec.nodeName === worker.metadata.name);

  return (
    <Drawer
      open={!!worker}
      onClose={onClose}
      title={worker.metadata.name}
      subtitle={`Worker · uid ${worker.metadata.uid ?? "—"}`}
    >
      <div className="divide-y divide-white/5">
        <Field label="Status">
          <StatusDot ok={ready} label={ready ? "Ready" : "NotReady"} />
        </Field>
        <Field label="Runtime">{worker.status?.containerRuntimeVersion ?? "unknown"}</Field>
        <Field label="Schedulable">{worker.spec.unschedulable ? "No (cordoned)" : "Yes"}</Field>
        <Field label="Capacity">
          {worker.status?.capacity?.cpu ?? "—"} cores · {fmtBytes(worker.status?.capacity?.memoryBytes)}
        </Field>
        <Field label="Addresses">
          {worker.status?.addresses?.length ? worker.status.addresses.join(", ") : "—"}
        </Field>
        <Field label="Lease">
          {lease ? (
            <>
              renewed {ageFrom(lease.spec.renewTime)} ago · {lease.spec.leaseDurationSeconds}s duration
            </>
          ) : (
            "none"
          )}
        </Field>
        <Field label="Created">{ageFrom(worker.metadata.creationTimestamp)} ago</Field>
      </div>

      <div className="mt-6">
        <div className="mb-2 text-sm font-semibold text-zinc-300">
          Containers on this node ({onNode.length})
        </div>
        {onNode.length === 0 ? (
          <div className="text-sm text-zinc-600">None</div>
        ) : (
          <div className="space-y-1.5">
            {onNode.map((c) => (
              <div
                key={c.metadata.name}
                className="flex items-center justify-between rounded-lg bg-white/[0.03] px-3 py-2 text-sm"
              >
                <span className="font-mono text-zinc-300">{c.metadata.name}</span>
                <span className="text-xs text-zinc-500">{phaseOf(c)}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="mt-6">
        <div className="text-sm font-semibold text-zinc-300">Labels</div>
        <div className="mt-2">
          <Labels labels={worker.metadata.labels} />
        </div>
      </div>

      <div className="mt-6">
        <div className="text-sm font-semibold text-zinc-300">Raw object</div>
        <Json value={worker} />
      </div>
    </Drawer>
  );
}
