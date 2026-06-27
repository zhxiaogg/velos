//! `veloslet` — the Velos worker daemon (the kubelet analog).
//!
//! It registers with the apiserver, watches the containers assigned to it,
//! reconciles desired vs. observed via the pure [`reconcile`] core, and actuates
//! through the [`velos_runtime::ContainerRuntime`] seam. It renews a `Lease` as a
//! liveness heartbeat. The worker is authoritative for container `status`.

pub mod client;
pub mod reconcile;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use velos_runtime::ContainerRuntime;

pub use client::{ApiClient, ClientError};
pub use reconcile::{Action, DesiredContainer, ObservedInstance, RestartPolicy, reconcile};

/// The finalizer this worker owns; its presence means "veloslet must clean up
/// the micro-VM before the apiserver may remove the object".
pub const FINALIZER: &str = "veloslet";

#[derive(Debug, thiserror::Error)]
pub enum VelosletError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("runtime error: {0}")]
    Runtime(#[from] velos_runtime::RuntimeError),
}

// ---------------------------------------------------------------------------
// Observation: turn apiserver container documents into the pure-core inputs.
// ---------------------------------------------------------------------------

fn str_at<'a>(doc: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = doc;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_str()
}

fn desired_from_doc(doc: &Value) -> Option<DesiredContainer> {
    let name = str_at(doc, &["metadata", "name"])?.to_string();
    let uid = str_at(doc, &["metadata", "uid"])?.to_string();
    let image = str_at(doc, &["spec", "image"]).unwrap_or("").to_string();
    let command = doc
        .pointer("/spec/command")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let env = doc
        .pointer("/spec/env")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let restart_policy =
        RestartPolicy::parse(str_at(doc, &["spec", "restartPolicy"]).unwrap_or("Never"));
    let phase = str_at(doc, &["status", "phase"])
        .unwrap_or("Pending")
        .to_string();
    let marked_for_deletion = doc
        .pointer("/metadata/deletionTimestamp")
        .map(|v| !v.is_null())
        .unwrap_or(false);
    let has_finalizer = doc
        .pointer("/metadata/finalizers")
        .and_then(Value::as_array)
        .map(|a| a.iter().any(|v| v.as_str() == Some(FINALIZER)))
        .unwrap_or(false);

    Some(DesiredContainer {
        name,
        uid,
        image,
        command,
        env,
        restart_policy,
        phase,
        marked_for_deletion,
        has_finalizer,
    })
}

// ---------------------------------------------------------------------------
// Actuation
// ---------------------------------------------------------------------------

fn running_status(node: &str, instance_id: &str) -> Value {
    serde_json::json!({
        "phase": "Running",
        "workerName": node,
        "containerID": instance_id,
        "startedAt": chrono::Utc::now().to_rfc3339(),
    })
}

fn terminal_status(node: &str, phase: &str, exit_code: i32) -> Value {
    serde_json::json!({
        "phase": phase,
        "workerName": node,
        "exitCode": exit_code,
        "finishedAt": chrono::Utc::now().to_rfc3339(),
    })
}

async fn clear_finalizer(client: &ApiClient, name: &str) -> Result<(), VelosletError> {
    let Some(mut doc) = client.get_container(name).await? else {
        return Ok(());
    };
    if let Some(meta) = doc.get_mut("metadata").and_then(Value::as_object_mut) {
        let remaining: Vec<Value> = meta
            .get("finalizers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|v| v.as_str() != Some(FINALIZER))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        meta.insert("finalizers".to_string(), Value::Array(remaining));
    }
    client.replace_container(name, &doc).await?;
    Ok(())
}

/// Apply one decided action against the runtime and apiserver.
pub async fn apply_action(
    client: &ApiClient,
    runtime: &dyn ContainerRuntime,
    node: &str,
    action: Action,
) -> Result<(), VelosletError> {
    match action {
        Action::Start { name, spec } => {
            let id = runtime.run(&spec).await?;
            client
                .put_status(&name, running_status(node, &id.0))
                .await?;
        }
        Action::Restart { name, uid, spec } => {
            runtime.remove(&uid).await?;
            let id = runtime.run(&spec).await?;
            client
                .put_status(&name, running_status(node, &id.0))
                .await?;
        }
        Action::ReportRunning { name } => {
            client.put_status(&name, running_status(node, "")).await?;
        }
        Action::ReportTerminal {
            name,
            phase,
            exit_code,
        } => {
            client
                .put_status(&name, terminal_status(node, &phase, exit_code))
                .await?;
        }
        Action::Cleanup {
            name,
            uid,
            clear_finalizer: clear,
        } => {
            runtime.stop(&uid).await?;
            runtime.remove(&uid).await?;
            if clear {
                clear_finalizer(client, &name).await?;
            }
        }
        Action::ClearFinalizer { name } => {
            clear_finalizer(client, &name).await?;
        }
        Action::Reap { uid } => {
            runtime.stop(&uid).await?;
            runtime.remove(&uid).await?;
        }
    }
    Ok(())
}

/// One reconcile pass: observe assigned containers + runtime, decide, actuate.
/// Returns the number of actions applied.
pub async fn run_once(
    client: &ApiClient,
    runtime: &dyn ContainerRuntime,
    node: &str,
) -> Result<usize, VelosletError> {
    let assigned = client.list_assigned(node).await?;
    let desired: Vec<DesiredContainer> = assigned.iter().filter_map(desired_from_doc).collect();

    let observed: Vec<ObservedInstance> = runtime
        .list()
        .await?
        .into_iter()
        .map(|i| ObservedInstance {
            uid: i.uid,
            state: i.state,
        })
        .collect();

    let actions = reconcile(&desired, &observed);
    let n = actions.len();
    for action in actions {
        apply_action(client, runtime, node, action).await?;
    }
    Ok(n)
}

/// Run the worker forever: heartbeat + reconcile on intervals.
pub async fn run_loop(
    client: ApiClient,
    runtime: Arc<dyn ContainerRuntime>,
    node: String,
    reconcile_interval: Duration,
    heartbeat_interval: Duration,
    lease_duration_secs: u32,
) {
    let mut reconcile_tick = tokio::time::interval(reconcile_interval);
    let mut heartbeat_tick = tokio::time::interval(heartbeat_interval);
    loop {
        tokio::select! {
            _ = reconcile_tick.tick() => {
                if let Err(e) = run_once(&client, runtime.as_ref(), &node).await {
                    tracing::warn!("reconcile failed: {e}");
                }
            }
            _ = heartbeat_tick.tick() => {
                if let Err(e) = client.renew_lease(&node, lease_duration_secs).await {
                    tracing::warn!("lease renew failed: {e}");
                }
            }
        }
    }
}
