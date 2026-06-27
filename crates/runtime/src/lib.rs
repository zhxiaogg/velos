//! The container runtime seam (Principle #3, deep module).
//!
//! `veloslet` drives micro-VMs only through the [`ContainerRuntime`] trait, so the
//! Apple Containerization `container` CLI can be swapped for Tart, Linux, or a
//! fake without touching the worker's reconcile logic. Every instance is keyed by
//! its Velos container **uid**, which makes actuation idempotent: reconcile after a
//! crash matches existing instances by uid before launching.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime command failed: {0}")]
    Command(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("lock poisoned")]
    Lock,
}

/// The runtime-local identifier of a launched instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceId(pub String);

/// What `veloslet` asks the runtime to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub uid: String,
    pub image: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Observed liveness of an instance. There is no "assumed running": an instance
/// the runtime cannot account for simply isn't in `list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceState {
    Running,
    Exited { exit_code: i32 },
}

/// One instance the runtime is tracking, tagged with its Velos uid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub uid: String,
    pub id: InstanceId,
    pub state: InstanceState,
}

#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    /// Launch an instance tagged with `spec.uid`. Idempotent callers check
    /// [`list`](ContainerRuntime::list) first.
    async fn run(&self, spec: &RunSpec) -> Result<InstanceId, RuntimeError>;
    /// Stop the instance tagged with `uid` (no-op if already gone).
    async fn stop(&self, uid: &str) -> Result<(), RuntimeError>;
    /// Remove the instance tagged with `uid` (no-op if already gone).
    async fn remove(&self, uid: &str) -> Result<(), RuntimeError>;
    /// All instances the runtime knows about, by uid.
    async fn list(&self) -> Result<Vec<Instance>, RuntimeError>;
    /// Reported runtime version string (for `WorkerStatus`).
    async fn version(&self) -> Result<String, RuntimeError>;
}

// ---------------------------------------------------------------------------
// FakeRuntime — in-memory, for tests and the e2e harness.
// ---------------------------------------------------------------------------

/// An in-memory runtime used by tests and `velos-tests`. Exit can be simulated
/// with [`FakeRuntime::set_exited`].
#[derive(Default)]
pub struct FakeRuntime {
    instances: Mutex<HashMap<String, Instance>>,
}

impl FakeRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate the instance for `uid` exiting with `exit_code`.
    pub fn set_exited(&self, uid: &str, exit_code: i32) -> Result<(), RuntimeError> {
        let mut g = self.instances.lock().map_err(|_| RuntimeError::Lock)?;
        if let Some(inst) = g.get_mut(uid) {
            inst.state = InstanceState::Exited { exit_code };
        }
        Ok(())
    }
}

#[async_trait]
impl ContainerRuntime for FakeRuntime {
    async fn run(&self, spec: &RunSpec) -> Result<InstanceId, RuntimeError> {
        let id = InstanceId(format!("fake-{}", spec.uid));
        let mut g = self.instances.lock().map_err(|_| RuntimeError::Lock)?;
        g.insert(
            spec.uid.clone(),
            Instance {
                uid: spec.uid.clone(),
                id: id.clone(),
                state: InstanceState::Running,
            },
        );
        Ok(id)
    }

    async fn stop(&self, uid: &str) -> Result<(), RuntimeError> {
        let mut g = self.instances.lock().map_err(|_| RuntimeError::Lock)?;
        if let Some(inst) = g.get_mut(uid) {
            inst.state = InstanceState::Exited { exit_code: 0 };
        }
        Ok(())
    }

    async fn remove(&self, uid: &str) -> Result<(), RuntimeError> {
        let mut g = self.instances.lock().map_err(|_| RuntimeError::Lock)?;
        g.remove(uid);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<Instance>, RuntimeError> {
        let g = self.instances.lock().map_err(|_| RuntimeError::Lock)?;
        Ok(g.values().cloned().collect())
    }

    async fn version(&self) -> Result<String, RuntimeError> {
        Ok("fake-runtime/1.0".to_string())
    }
}

// ---------------------------------------------------------------------------
// AppleContainer — wraps the `container` CLI (Apple Containerization).
// ---------------------------------------------------------------------------

/// Label key used to tag every instance with its Velos uid.
const UID_LABEL: &str = "velos.uid";

/// Real backend: shells out to the `container` CLI via `tokio::process`.
pub struct AppleContainer {
    bin: String,
}

impl Default for AppleContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl AppleContainer {
    pub fn new() -> Self {
        Self {
            bin: "container".to_string(),
        }
    }

    /// Override the CLI binary path (e.g. for an alternate install location).
    pub fn with_binary(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    async fn output(&self, args: &[String]) -> Result<String, RuntimeError> {
        let out = tokio::process::Command::new(&self.bin)
            .args(args)
            .output()
            .await
            .map_err(|e| RuntimeError::Io(e.to_string()))?;
        if !out.status.success() {
            return Err(RuntimeError::Command(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[async_trait]
impl ContainerRuntime for AppleContainer {
    async fn run(&self, spec: &RunSpec) -> Result<InstanceId, RuntimeError> {
        let mut args = vec![
            "run".to_string(),
            "--detach".to_string(),
            "--label".to_string(),
            format!("{UID_LABEL}={}", spec.uid),
        ];
        for (k, v) in &spec.env {
            args.push("--env".to_string());
            args.push(format!("{k}={v}"));
        }
        args.push(spec.image.clone());
        args.extend(spec.command.iter().cloned());
        let id = self.output(&args).await?;
        Ok(InstanceId(id))
    }

    async fn stop(&self, uid: &str) -> Result<(), RuntimeError> {
        // Best-effort: ignore "no such container" by mapping a failure to Ok only
        // when nothing matched. We resolve the runtime id from the uid label.
        if let Some(inst) = self.find(uid).await? {
            self.output(&["stop".to_string(), inst.id.0]).await?;
        }
        Ok(())
    }

    async fn remove(&self, uid: &str) -> Result<(), RuntimeError> {
        if let Some(inst) = self.find(uid).await? {
            self.output(&["rm".to_string(), inst.id.0]).await?;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<Instance>, RuntimeError> {
        let raw = self
            .output(&[
                "ls".to_string(),
                "--all".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ])
            .await?;
        parse_ls(&raw)
    }

    async fn version(&self) -> Result<String, RuntimeError> {
        self.output(&["--version".to_string()]).await
    }
}

impl AppleContainer {
    async fn find(&self, uid: &str) -> Result<Option<Instance>, RuntimeError> {
        Ok(self.list().await?.into_iter().find(|i| i.uid == uid))
    }
}

/// Parse `container ls --format json` into uid-tagged instances. Entries without
/// the velos uid label are ignored (not ours).
fn parse_ls(raw: &str) -> Result<Vec<Instance>, RuntimeError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| RuntimeError::Command(e.to_string()))?;
    let arr = value.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for entry in arr {
        let uid = entry
            .pointer("/labels")
            .and_then(|l| l.get(UID_LABEL))
            .and_then(|v| v.as_str());
        let Some(uid) = uid else { continue };
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(uid)
            .to_string();
        let status = entry
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let state = match status {
            "running" => InstanceState::Running,
            _ => InstanceState::Exited {
                exit_code: entry.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            },
        };
        out.push(Instance {
            uid: uid.to_string(),
            id: InstanceId(id),
            state,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn spec(uid: &str) -> RunSpec {
        RunSpec {
            uid: uid.to_string(),
            image: "alpine".to_string(),
            command: vec![],
            env: vec![],
        }
    }

    #[tokio::test]
    async fn fake_runtime_run_list_exit_remove() {
        let rt = FakeRuntime::new();
        rt.run(&spec("u1")).await.unwrap();
        let list = rt.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, InstanceState::Running);

        rt.set_exited("u1", 3).unwrap();
        let list = rt.list().await.unwrap();
        assert_eq!(list[0].state, InstanceState::Exited { exit_code: 3 });

        rt.remove("u1").await.unwrap();
        assert!(rt.list().await.unwrap().is_empty());
    }

    #[test]
    fn parse_ls_filters_to_velos_instances() {
        let raw = r#"[
            {"id":"abc","status":"running","labels":{"velos.uid":"u1"}},
            {"id":"def","status":"stopped","labels":{"velos.uid":"u2"},"exitCode":2},
            {"id":"ghi","status":"running","labels":{"other":"x"}}
        ]"#;
        let mut got = parse_ls(raw).unwrap();
        got.sort_by(|a, b| a.uid.cmp(&b.uid));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].uid, "u1");
        assert_eq!(got[0].state, InstanceState::Running);
        assert_eq!(got[1].state, InstanceState::Exited { exit_code: 2 });
    }
}
