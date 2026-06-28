import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { Container, Lease, List, RestartPolicy, Worker } from "./types";

// Everything is proxied through the Vite dev server, which injects the Bearer
// credential. The browser just talks to a same-origin `/velos` prefix.
const BASE = "/velos/api/v1";

async function http<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
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
