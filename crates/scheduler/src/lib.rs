//! The Velos scheduler: a pure decision function that binds an unscheduled
//! container to a worker.
//!
//! Principle #5 (pure core): `schedule` is a total function of its inputs with no
//! I/O. The controller that observes state and writes the binding lives elsewhere
//! (`velos-server`); this crate only decides.

/// A worker's name — a semantic type, not a bare `String` (Principle #1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkerName(pub String);

/// A resource ask (or usage). cpu is a whole-core count; memory in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRequest {
    pub cpu: u32,
    pub memory_bytes: u64,
}

/// The scheduler's view of one candidate worker: enough to decide fit, nothing
/// about wire or storage shapes.
#[derive(Debug, Clone)]
pub struct WorkerView {
    pub name: WorkerName,
    pub ready: bool,
    pub unschedulable: bool,
    /// Total schedulable resources on the node.
    pub allocatable: ResourceRequest,
    /// Resources already committed to containers bound here.
    pub allocated: ResourceRequest,
}

impl WorkerView {
    /// Free cpu cores after accounting for already-allocated containers.
    fn free_cpu(&self) -> u32 {
        self.allocatable.cpu.saturating_sub(self.allocated.cpu)
    }

    /// Free memory bytes after accounting for already-allocated containers.
    fn free_memory(&self) -> u64 {
        self.allocatable
            .memory_bytes
            .saturating_sub(self.allocated.memory_bytes)
    }

    /// Whether this worker can admit one more container needing `req`.
    fn admits(&self, req: &ResourceRequest) -> bool {
        self.ready
            && !self.unschedulable
            && self.free_cpu() >= req.cpu
            && self.free_memory() >= req.memory_bytes
    }
}

/// First-fit: return the first worker, in the given order, that can admit `req`.
/// `None` means the container stays `Pending`. This seam later absorbs affinity,
/// bin-packing, and taints without changing callers.
pub fn schedule(req: &ResourceRequest, workers: &[WorkerView]) -> Option<WorkerName> {
    workers
        .iter()
        .find(|w| w.admits(req))
        .map(|w| w.name.clone())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn worker(
        name: &str,
        ready: bool,
        unschedulable: bool,
        cpu: u32,
        mem: u64,
        used_cpu: u32,
        used_mem: u64,
    ) -> WorkerView {
        WorkerView {
            name: WorkerName(name.to_string()),
            ready,
            unschedulable,
            allocatable: ResourceRequest {
                cpu,
                memory_bytes: mem,
            },
            allocated: ResourceRequest {
                cpu: used_cpu,
                memory_bytes: used_mem,
            },
        }
    }

    #[test]
    fn picks_first_fitting_ready_worker() {
        let req = ResourceRequest {
            cpu: 2,
            memory_bytes: 2 * GB,
        };
        let workers = vec![
            worker("w1", true, false, 1, 8 * GB, 0, 0), // too few cores
            worker("w2", true, false, 4, 8 * GB, 0, 0), // fits
        ];
        assert_eq!(schedule(&req, &workers), Some(WorkerName("w2".to_string())));
    }

    #[test]
    fn skips_not_ready_and_unschedulable_workers() {
        let req = ResourceRequest {
            cpu: 1,
            memory_bytes: GB,
        };
        let workers = vec![
            worker("w1", false, false, 8, 16 * GB, 0, 0), // not ready
            worker("w2", true, true, 8, 16 * GB, 0, 0),   // cordoned
            worker("w3", true, false, 8, 16 * GB, 0, 0),  // ok
        ];
        assert_eq!(schedule(&req, &workers), Some(WorkerName("w3".to_string())));
    }

    #[test]
    fn respects_already_allocated_capacity() {
        let req = ResourceRequest {
            cpu: 2,
            memory_bytes: GB,
        };
        // 4 cores total, 3 used -> only 1 free, request needs 2.
        let workers = vec![worker("w1", true, false, 4, 8 * GB, 3, 0)];
        assert_eq!(schedule(&req, &workers), None);
    }

    #[test]
    fn returns_none_when_nothing_fits() {
        let req = ResourceRequest {
            cpu: 64,
            memory_bytes: 256 * GB,
        };
        let workers = vec![worker("w1", true, false, 8, 16 * GB, 0, 0)];
        assert_eq!(schedule(&req, &workers), None);
    }
}
