import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { forgetCredential, getCredential } from "./auth";
import type { Container, Lease, List, RestartPolicy, Worker } from "./types";

// The API is same-origin: the apiserver serves this bundle in production, and
// the Vite dev server proxies these paths to it. The browser supplies its own
// Bearer credential (see ./auth).
const BASE = "/api/v1";

async function send<T>(path: string, init: RequestInit, retry = true): Promise<T> {
  const cred = await getCredential();
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${cred}`,
      ...(init.headers ?? {}),
    },
  });

  // A stale credential (e.g. the server's store was reset) — drop it and retry
  // once with a freshly minted one.
  if (res.status === 401 && retry) {
    forgetCredential();
    return send<T>(path, init, false);
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
