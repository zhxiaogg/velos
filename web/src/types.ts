// Wire types as served by the Velos server (camelCase JSON).
// These mirror the fluorite-generated protocol types — the dashboard only reads
// the fields it renders, treating the document as otherwise opaque.

export interface ObjectMeta {
  name: string;
  uid?: string;
  labels?: Record<string, string>;
  annotations?: Record<string, string>;
  resourceVersion?: number;
  creationTimestamp?: string;
  deletionTimestamp?: string;
  finalizers?: string[];
}

export type ContainerPhase =
  | "Pending"
  | "Scheduled"
  | "Running"
  | "Succeeded"
  | "Failed"
  | "Unknown";

export type RestartPolicy = "Never" | "OnFailure" | "Always";

export interface ResourceSpec {
  cpu?: number;
  memoryBytes?: number;
}

export interface ContainerSpec {
  image: string;
  command?: string[];
  env?: Record<string, string>;
  resources?: ResourceSpec;
  restartPolicy?: RestartPolicy;
  nodeName?: string;
}

export interface ContainerStatus {
  phase?: ContainerPhase;
  workerName?: string;
  containerID?: string;
  startedAt?: string;
  finishedAt?: string;
  exitCode?: number;
  message?: string;
}

export interface Container {
  metadata: ObjectMeta;
  spec: ContainerSpec;
  status?: ContainerStatus;
}

export interface Capacity {
  cpu?: number;
  memoryBytes?: number;
  maxContainers?: number;
}

export interface WorkerCondition {
  conditionType: "Ready";
  status: boolean;
  lastTransitionTime?: string;
  reason?: string;
}

export interface WorkerStatus {
  capacity?: Capacity;
  allocatable?: Capacity;
  conditions?: WorkerCondition[];
  addresses?: string[];
  containerRuntimeVersion?: string;
}

export interface Worker {
  metadata: ObjectMeta;
  spec: { unschedulable?: boolean };
  status?: WorkerStatus;
}

export interface LeaseSpec {
  holderIdentity?: string;
  renewTime?: string;
  leaseDurationSeconds?: number;
}

export interface Lease {
  metadata: ObjectMeta;
  spec: LeaseSpec;
}

export interface List<T> {
  items: T[];
}
