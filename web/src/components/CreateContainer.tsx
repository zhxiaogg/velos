import { useState } from "react";
import { Rocket, X } from "lucide-react";
import { useCreateContainer, type NewContainer } from "../api";
import type { RestartPolicy } from "../types";

const POLICIES: RestartPolicy[] = ["Never", "OnFailure", "Always"];

export function CreateContainer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const create = useCreateContainer();
  const [name, setName] = useState("");
  const [image, setImage] = useState("docker.io/library/alpine:latest");
  const [command, setCommand] = useState("sleep 3600");
  const [cpu, setCpu] = useState(1);
  const [mem, setMem] = useState(256);
  const [memUnit, setMemUnit] = useState<"MiB" | "GiB">("MiB");
  const [policy, setPolicy] = useState<RestartPolicy>("Never");
  const [labels, setLabels] = useState("app=demo");

  if (!open) return null;

  function parseKV(s: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const part of s.split(/[,\s]+/).filter(Boolean)) {
      const i = part.indexOf("=");
      if (i > 0) out[part.slice(0, i)] = part.slice(i + 1);
    }
    return out;
  }

  async function submit() {
    const body: NewContainer = {
      name: name.trim(),
      image: image.trim(),
      command: command.trim() ? command.trim().split(/\s+/) : [],
      cpu,
      memoryBytes: mem * (memUnit === "GiB" ? 1024 ** 3 : 1024 ** 2),
      restartPolicy: policy,
      env: {},
      labels: parseKV(labels),
    };
    try {
      await create.mutateAsync(body);
      onClose();
      setName("");
    } catch {
      /* error surfaced below */
    }
  }

  const valid = name.trim().length > 0 && image.trim().length > 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} />
      <div className="relative w-full max-w-lg rounded-2xl border border-white/10 bg-[#0d0d14] shadow-2xl">
        <div className="flex items-center justify-between border-b border-white/5 px-6 py-4">
          <div className="flex items-center gap-2 text-lg font-semibold text-zinc-100">
            <Rocket size={18} className="text-indigo-400" />
            Launch container
          </div>
          <button onClick={onClose} className="rounded-lg p-1.5 text-zinc-400 hover:bg-white/5">
            <X size={18} />
          </button>
        </div>

        <div className="space-y-4 px-6 py-5">
          <Row label="Name">
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value.replace(/[^a-z0-9-]/g, ""))}
              placeholder="my-job"
              className={input}
            />
          </Row>
          <Row label="Image">
            <input value={image} onChange={(e) => setImage(e.target.value)} className={input} />
          </Row>
          <Row label="Command">
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder="(image default)"
              className={`${input} font-mono`}
            />
          </Row>
          <div className="grid grid-cols-2 gap-4">
            <Row label="CPU (cores)">
              <input
                type="number"
                min={1}
                value={cpu}
                onChange={(e) => setCpu(Math.max(1, +e.target.value))}
                className={input}
              />
            </Row>
            <Row label="Memory">
              <div className="flex gap-2">
                <input
                  type="number"
                  min={1}
                  value={mem}
                  onChange={(e) => setMem(Math.max(1, +e.target.value))}
                  className={input}
                />
                <select
                  value={memUnit}
                  onChange={(e) => setMemUnit(e.target.value as "MiB" | "GiB")}
                  className={`${input} w-24`}
                >
                  <option>MiB</option>
                  <option>GiB</option>
                </select>
              </div>
            </Row>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <Row label="Restart policy">
              <select value={policy} onChange={(e) => setPolicy(e.target.value as RestartPolicy)} className={input}>
                {POLICIES.map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </Row>
            <Row label="Labels">
              <input value={labels} onChange={(e) => setLabels(e.target.value)} className={`${input} font-mono`} />
            </Row>
          </div>

          {create.isError && (
            <div className="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-sm text-rose-300">
              {(create.error as Error).message}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-3 border-t border-white/5 px-6 py-4">
          <button onClick={onClose} className="rounded-lg px-4 py-2 text-sm text-zinc-400 hover:bg-white/5">
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={!valid || create.isPending}
            className="inline-flex items-center gap-2 rounded-lg bg-indigo-500 px-4 py-2 text-sm font-medium text-white shadow-lg shadow-indigo-500/20 hover:bg-indigo-400 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {create.isPending ? "Launching…" : "Launch"}
          </button>
        </div>
      </div>
    </div>
  );
}

const input =
  "w-full rounded-lg border border-white/10 bg-black/30 px-3 py-2 text-sm text-zinc-100 outline-none focus:border-indigo-400/60 focus:ring-2 focus:ring-indigo-400/20";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <div className="mb-1.5 text-xs font-medium uppercase tracking-wide text-zinc-500">{label}</div>
      {children}
    </label>
  );
}
