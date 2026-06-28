import type { ReactNode } from "react";
import { X } from "lucide-react";

export function Drawer({
  open,
  onClose,
  title,
  subtitle,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div
      className={`fixed inset-0 z-40 transition ${open ? "pointer-events-auto" : "pointer-events-none"}`}
      aria-hidden={!open}
    >
      <div
        className={`absolute inset-0 bg-black/50 transition-opacity ${open ? "opacity-100" : "opacity-0"}`}
        onClick={onClose}
      />
      <div
        className={`absolute right-0 top-0 flex h-full w-full max-w-xl flex-col border-l border-white/10 bg-[#0d0d14] shadow-2xl transition-transform duration-300 ${
          open ? "translate-x-0" : "translate-x-full"
        }`}
      >
        <div className="flex items-start justify-between border-b border-white/5 px-6 py-4">
          <div className="min-w-0">
            <div className="truncate text-lg font-semibold text-zinc-100">{title}</div>
            {subtitle && <div className="mt-0.5 text-xs text-zinc-500">{subtitle}</div>}
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-zinc-400 hover:bg-white/5 hover:text-zinc-200"
          >
            <X size={18} />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto px-6 py-5">{children}</div>
      </div>
    </div>
  );
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[140px_1fr] gap-3 py-2 text-sm">
      <div className="text-zinc-500">{label}</div>
      <div className="min-w-0 break-words text-zinc-200">{children}</div>
    </div>
  );
}

export function Json({ value }: { value: unknown }) {
  return (
    <pre className="mt-2 overflow-x-auto rounded-lg border border-white/5 bg-black/40 p-3 font-mono text-[11px] leading-relaxed text-zinc-400">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}
