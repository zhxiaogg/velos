import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { logout, sessionToken } from "./auth";
import type { Container, Lease, List, RestartPolicy, Worker } from "./types";

// The API is same-origin: the apiserver serves this bundle in production, and
// the Vite dev server proxies these paths to it. The browser sends its admin
// session token (see ./auth) as a Bearer on every call.
const BASE = "/api/v1";

/// Send to a fully-qualified path with the session bearer; 401 -> logout.
async function sendAuth<T>(path: string, init: RequestInit): Promise<T> {
  const tok = sessionToken();
  const res = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(tok ? { authorization: `Bearer ${tok}` } : {}),
      ...(init.headers ?? {}),
    },
  });

  // Session expired/invalid -> drop it; the app shell shows the login screen.
  if (res.status === 401) {
    logout();
    throw new Error("unauthorized");
  }
  if (!res.ok) {
    let detail = "";
    try {
      detail = (await res.json())?.error ?? "";
    } catch {
      /* ignore */
    }
    throw new Error(`${res.status} ${res.statusText}${detail ? ` — ${detail}` : ""}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

const send = <T>(path: string, init: RequestInit): Promise<T> => sendAuth<T>(`${BASE}${path}`, init);

const http = <T>(path: string, init: RequestInit = {}): Promise<T> => send<T>(path, init);

const REFRESH_MS = 2000;

export function useWorkers() {
  return useQuery({
    queryKey: ["workers"],
    queryFn: () => http<List<Worker>>("/workers"),
    refetchInterval: REFRESH_MS,
    select: (d) => d.items ?? [],
  });
}

export function useContainers() {
  return useQuery({
    queryKey: ["containers"],
    queryFn: () => http<List<Container>>("/containers"),
    refetchInterval: REFRESH_MS,
    select: (d) => d.items ?? [],
  });
}

export function useLeases() {
  return useQuery({
    queryKey: ["leases"],
    queryFn: () => http<List<Lease>>("/leases"),
    refetchInterval: REFRESH_MS,
    select: (d) => d.items ?? [],
  });
}

export interface NewContainer {
  name: string;
  image: string;
  command: string[];
  cpu: number;
  memoryBytes: number;
  restartPolicy: RestartPolicy;
  env: Record<string, string>;
  labels: Record<string, string>;
}

export function useCreateContainer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (c: NewContainer) =>
      http<Container>("/containers", {
        method: "POST",
        body: JSON.stringify({
          metadata: { name: c.name, labels: c.labels },
          spec: {
            image: c.image,
            command: c.command,
            env: c.env,
            resources: { cpu: c.cpu, memoryBytes: c.memoryBytes },
            restartPolicy: c.restartPolicy,
          },
          // The scheduler only places containers whose phase is Pending.
          status: { phase: "Pending" },
        }),
      }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["containers"] }),
  });
}

export function useDeleteContainer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => http<void>(`/containers/${name}`, { method: "DELETE" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["containers"] }),
  });
}

// ── Admin CLI tokens ──────────────────────────────────────────────────────
// These hit /auth/v1/admin/* (not under /api/v1), so they call sendAuth directly.

export interface AdminToken {
  id: string;
  label: string;
  kind: string;
  createdAt: string;
  expiresAt: string;
}

export function useTokens() {
  return useQuery({
    queryKey: ["admin-tokens"],
    queryFn: () => sendAuth<{ items: AdminToken[] }>("/auth/v1/admin/tokens", {}),
    select: (d) => d.items ?? [],
  });
}

export function useCreateToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (label: string) =>
      sendAuth<{ id: string; token: string }>("/auth/v1/admin/tokens", {
        method: "POST",
        body: JSON.stringify({ label }),
      }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["admin-tokens"] }),
  });
}

export function useRevokeToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      sendAuth<void>(`/auth/v1/admin/tokens/${id}`, { method: "DELETE" }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["admin-tokens"] }),
  });
}
