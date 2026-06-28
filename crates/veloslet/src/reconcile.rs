//! Pure worker reconciliation (Principle #5).
//!
//! `reconcile(desired, observed) -> Vec<Action>` is a total function: it compares
//! the containers the server assigned to this worker against what the runtime
//! actually reports, and returns the actions that converge the two. All matching
//! is keyed by container **uid**, which makes the result idempotent across crashes.

use velos_runtime::{InstanceState, RunSpec};

/// Restart behavior for a container (mirrors `velos::RestartPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

impl RestartPolicy {
    /// Parse the wire string; unknown values fail closed to `Never`.
    pub fn parse(s: &str) -> Self {
        match s {
            "Always" => RestartPolicy::Always,
            "OnFailure" => RestartPolicy::OnFailure,
            _ => RestartPolicy::Never,
        }
    }

    fn should_restart(self, exit_code: i32) -> bool {
        match self {
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure => exit_code != 0,
            RestartPolicy::Never => false,
        }
    }
}

/// A container the server has assigned to this worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredContainer {
    pub name: String,
    pub uid: String,
    pub image: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    pub restart_policy: RestartPolicy,
    pub phase: String,
    pub marked_for_deletion: bool,
    pub has_finalizer: bool,
}

impl DesiredContainer {
    fn run_spec(&self) -> RunSpec {
        RunSpec {
            uid: self.uid.clone(),
            image: self.image.clone(),
            command: self.command.clone(),
            env: self.env.clone(),
        }
    }
}

/// What the runtime reports for one instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedInstance {
    pub uid: String,
    pub state: InstanceState,
}

/// An intended action; the actuator turns these into runtime + server calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Launch the instance, then report `Running`.
    Start { name: String, spec: RunSpec },
    /// Remove the exited instance and launch a fresh one (restart policy).
    Restart {
        name: String,
        uid: String,
        spec: RunSpec,
    },
    /// Instance is running but status is stale → report `Running`.
    ReportRunning { name: String },
    /// Instance exited and won't restart → report `Succeeded`/`Failed`.
    ReportTerminal {
        name: String,
        phase: String,
        exit_code: i32,
    },
    /// Container is being deleted → stop+remove instance, optionally clear finalizer.
    Cleanup {
        name: String,
        uid: String,
        clear_finalizer: bool,
    },
    /// Container is being deleted, has no instance, but still holds our finalizer.
    ClearFinalizer { name: String },
    /// An instance with no matching assignment → reap the orphan.
    Reap { uid: String },
}

fn is_terminal(phase: &str) -> bool {
    matches!(phase, "Succeeded" | "Failed")
}

fn terminal_phase(exit_code: i32) -> &'static str {
    if exit_code == 0 {
        "Succeeded"
    } else {
        "Failed"
    }
}

/// Decide the actions that converge `observed` toward `desired`.
pub fn reconcile(desired: &[DesiredContainer], observed: &[ObservedInstance]) -> Vec<Action> {
    let mut actions = Vec::new();

    for d in desired {
        let obs = observed.iter().find(|o| o.uid == d.uid);

        if d.marked_for_deletion {
            match obs {
                Some(_) => actions.push(Action::Cleanup {
                    name: d.name.clone(),
                    uid: d.uid.clone(),
                    clear_finalizer: d.has_finalizer,
                }),
                None => {
                    if d.has_finalizer {
                        actions.push(Action::ClearFinalizer {
                            name: d.name.clone(),
                        });
                    }
                }
            }
            continue;
        }

        match obs.map(|o| &o.state) {
            None => {
                if !is_terminal(&d.phase) {
                    actions.push(Action::Start {
                        name: d.name.clone(),
                        spec: d.run_spec(),
                    });
                }
            }
            Some(InstanceState::Running) => {
                if d.phase != "Running" {
                    actions.push(Action::ReportRunning {
                        name: d.name.clone(),
                    });
                }
            }
            Some(InstanceState::Exited { exit_code }) => {
                if d.restart_policy.should_restart(*exit_code) {
                    actions.push(Action::Restart {
                        name: d.name.clone(),
                        uid: d.uid.clone(),
                        spec: d.run_spec(),
                    });
                } else {
                    let phase = terminal_phase(*exit_code);
                    if d.phase != phase {
                        actions.push(Action::ReportTerminal {
                            name: d.name.clone(),
                            phase: phase.to_string(),
                            exit_code: *exit_code,
                        });
                    }
                }
            }
        }
    }

    for o in observed {
        if !desired.iter().any(|d| d.uid == o.uid) {
            actions.push(Action::Reap { uid: o.uid.clone() });
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desired(name: &str, phase: &str, policy: RestartPolicy) -> DesiredContainer {
        DesiredContainer {
            name: name.to_string(),
            uid: format!("uid-{name}"),
            image: "img".to_string(),
            command: vec![],
            env: vec![],
            restart_policy: policy,
            phase: phase.to_string(),
            marked_for_deletion: false,
            has_finalizer: true,
        }
    }

    fn observed(name: &str, state: InstanceState) -> ObservedInstance {
        ObservedInstance {
            uid: format!("uid-{name}"),
            state,
        }
    }

    #[test]
    fn starts_pending_container_with_no_instance() {
        let d = vec![desired("c1", "Scheduled", RestartPolicy::Never)];
        let actions = reconcile(&d, &[]);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], Action::Start { name, .. } if name == "c1"));
    }

    #[test]
    fn reports_running_when_status_is_stale() {
        let d = vec![desired("c1", "Scheduled", RestartPolicy::Never)];
        let o = vec![observed("c1", InstanceState::Running)];
        assert_eq!(
            reconcile(&d, &o),
            vec![Action::ReportRunning {
                name: "c1".to_string()
            }]
        );
    }

    #[test]
    fn no_action_when_running_and_reported() {
        let d = vec![desired("c1", "Running", RestartPolicy::Never)];
        let o = vec![observed("c1", InstanceState::Running)];
        assert!(reconcile(&d, &o).is_empty());
    }

    #[test]
    fn reports_succeeded_on_clean_exit_with_never_policy() {
        let d = vec![desired("c1", "Running", RestartPolicy::Never)];
        let o = vec![observed("c1", InstanceState::Exited { exit_code: 0 })];
        assert_eq!(
            reconcile(&d, &o),
            vec![Action::ReportTerminal {
                name: "c1".to_string(),
                phase: "Succeeded".to_string(),
                exit_code: 0,
            }]
        );
    }

    #[test]
    fn restarts_on_failure_policy_when_exit_nonzero() {
        let d = vec![desired("c1", "Running", RestartPolicy::OnFailure)];
        let o = vec![observed("c1", InstanceState::Exited { exit_code: 1 })];
        assert!(matches!(&reconcile(&d, &o)[0], Action::Restart { name, .. } if name == "c1"));
    }

    #[test]
    fn always_policy_restarts_even_on_clean_exit() {
        let d = vec![desired("c1", "Running", RestartPolicy::Always)];
        let o = vec![observed("c1", InstanceState::Exited { exit_code: 0 })];
        assert!(matches!(&reconcile(&d, &o)[0], Action::Restart { .. }));
    }

    #[test]
    fn terminal_container_without_instance_is_left_alone() {
        let d = vec![desired("c1", "Succeeded", RestartPolicy::Never)];
        assert!(reconcile(&d, &[]).is_empty());
    }

    #[test]
    fn deletion_with_instance_cleans_up_and_clears_finalizer() {
        let mut d = desired("c1", "Running", RestartPolicy::Never);
        d.marked_for_deletion = true;
        let o = vec![observed("c1", InstanceState::Running)];
        assert_eq!(
            reconcile(&[d], &o),
            vec![Action::Cleanup {
                name: "c1".to_string(),
                uid: "uid-c1".to_string(),
                clear_finalizer: true,
            }]
        );
    }

    #[test]
    fn deletion_without_instance_just_clears_finalizer() {
        let mut d = desired("c1", "Running", RestartPolicy::Never);
        d.marked_for_deletion = true;
        assert_eq!(
            reconcile(&[d], &[]),
            vec![Action::ClearFinalizer {
                name: "c1".to_string()
            }]
        );
    }

    #[test]
    fn orphan_instance_is_reaped() {
        let o = vec![observed("ghost", InstanceState::Running)];
        assert_eq!(
            reconcile(&[], &o),
            vec![Action::Reap {
                uid: "uid-ghost".to_string()
            }]
        );
    }
}
