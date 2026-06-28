import { useEffect, useState } from "react";
import { Box, KeyRound, LayoutDashboard, LogOut, Server } from "lucide-react";
import { useContainers, useWorkers } from "./api";
import { logout } from "./auth";
import { isWorkerReady } from "./format";
import { Overview } from "./views/Overview";
import { Workers } from "./views/Workers";
import { Containers } from "./views/Containers";
import { Tokens } from "./views/Tokens";

type Tab = "overview" | "workers" | "containers" | "tokens";

const NAV: { id: Tab; label: string; icon: React.ReactNode }[] = [
  { id: "overview", label: "Overview", icon: <LayoutDashboard size={18} /> },
  { id: "workers", label: "Workers", icon: <Server size={18} /> },
  { id: "containers", label: "Containers", icon: <Box size={18} /> },
  { id: "tokens", label: "Tokens", icon: <KeyRound size={18} /> },
];

const TABS: Tab[] = ["overview", "workers", "containers", "tokens"];

function tabFromHash(): Tab {
  const h = window.location.hash.replace("#", "") as Tab;
  return TABS.includes(h) ? h : "overview";
}

export default function App() {
  const [tab, setTabState] = useState<Tab>(tabFromHash);
  const setTab = (t: Tab) => {
    window.location.hash = t;
    setTabState(t);
  };

  useEffect(() => {
    const onHash = () => setTabState(tabFromHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  const { data: workers = [], isError } = useWorkers();
  const { data: containers = [] } = useContainers();

  // A heartbeat dot that ticks every refetch so "live" feels alive.
  const [beat, setBeat] = useState(false);
  useEffect(() => {
    setBeat(true);
    const t = setTimeout(() => setBeat(false), 400);
    return () => clearTimeout(t);
  }, [workers, containers]);

  const ready = workers.filter(isWorkerReady).length;

  return (
    <div className="flex h-full">
      <aside className="flex w-60 shrink-0 flex-col border-r border-white/5 bg-black/30 px-4 py-5">
        <div className="flex items-center gap-2.5 px-2">
          <div className="grid h-9 w-9 place-items-center rounded-xl bg-gradient-to-br from-indigo-500 to-violet-600 font-bold text-white shadow-lg shadow-indigo-500/30">
            V
          </div>
          <div>
            <div className="text-sm font-semibold tracking-tight text-zinc-100">Velos</div>
            <div className="text-[11px] text-zinc-500">control plane</div>
          </div>
        </div>

        <nav className="mt-8 space-y-1">
          {NAV.map((n) => (
            <button
              key={n.id}
              onClick={() => setTab(n.id)}
              className={`flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm transition ${
                tab === n.id
                  ? "bg-white/[0.06] text-zinc-100"
                  : "text-zinc-500 hover:bg-white/[0.03] hover:text-zinc-300"
              }`}
            >
              {n.icon}
              {n.label}
              {n.id === "workers" && workers.length > 0 && (
                <span className="ml-auto rounded-full bg-white/5 px-2 py-0.5 text-[11px] text-zinc-400">
                  {workers.length}
                </span>
              )}
              {n.id === "containers" && containers.length > 0 && (
                <span className="ml-auto rounded-full bg-white/5 px-2 py-0.5 text-[11px] text-zinc-400">
                  {containers.length}
                </span>
              )}
            </button>
          ))}
        </nav>

        <div className="mt-auto rounded-lg border border-white/5 bg-white/[0.02] p-3 text-xs">
          <div className="flex items-center gap-2 text-zinc-400">
            <span className={`h-2 w-2 rounded-full ${isError ? "bg-rose-500" : "bg-emerald-400 live-dot"}`} />
            {isError ? "apiserver unreachable" : `${ready} worker${ready === 1 ? "" : "s"} ready`}
          </div>
          <div className="mt-1 text-zinc-600">{window.location.host}</div>
        </div>
      </aside>

      <main className="flex-1 overflow-y-auto">
        <header className="sticky top-0 z-10 flex items-center justify-between border-b border-white/5 bg-[#0a0a0f]/80 px-8 py-5 backdrop-blur">
          <div>
            <h1 className="text-xl font-semibold tracking-tight text-zinc-100">
              {NAV.find((n) => n.id === tab)?.label}
            </h1>
            <p className="mt-0.5 text-sm text-zinc-500">
              {tab === "overview" && "Cluster health and capacity at a glance"}
              {tab === "workers" && "Registered nodes and their leases"}
              {tab === "containers" && "Workloads across the cluster"}
              {tab === "tokens" && "CLI access tokens for velosctl"}
            </p>
          </div>
          <div className="flex items-center gap-4 text-xs text-zinc-500">
            <span className="flex items-center gap-2">
              <span className={`h-1.5 w-1.5 rounded-full transition-colors ${beat ? "bg-emerald-400" : "bg-zinc-700"}`} />
              live · 2s
            </span>
            <button
              onClick={logout}
              className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-zinc-400 transition hover:bg-white/5 hover:text-zinc-200"
              title="Sign out"
            >
              <LogOut size={15} />
              Sign out
            </button>
          </div>
        </header>

        <div className="px-8 py-6">
          {tab === "overview" && <Overview />}
          {tab === "workers" && <Workers />}
          {tab === "containers" && <Containers />}
          {tab === "tokens" && <Tokens />}
        </div>
      </main>
    </div>
  );
}
