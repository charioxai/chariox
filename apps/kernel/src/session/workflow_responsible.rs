//! Who answers for a workflow, and what a run that needs someone looks like.
//!
//! A workflow has one responsible: the person, or an agent that receives each run's needs as
//! an ordinary prompt. Runs never raise cards; they end or hold with a typed outcome that the
//! responsible reads.
use serde::{Deserialize, Serialize};

use super::workflow_diagnostics::WorkflowFailureKind;
use super::workflow_run_records::WorkflowNodeRun;
use super::workflow_runs::WorkflowRun;
use super::workflow_turns::{WorkflowNodeRunStatus, WorkflowNodeWaitingReason, WorkflowRunStatus};

/// Longest reason a failed run's outcome carries.
pub const OUTCOME_DETAIL_CHARS: usize = 200;

/// Who answers for a workflow. `label` is the agent's name when it took the workflow, so a
/// gone agent still has a name to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowResponsible {
    #[default]
    Person,
    Agent {
        kernel_id: String,
        room_id: String,
        agent_id: String,
        label: String,
    },
}

impl WorkflowResponsible {
    pub fn is_person(&self) -> bool {
        matches!(self, Self::Person)
    }

    pub fn agent(&self) -> Option<ResponsibleAgent<'_>> {
        match self {
            Self::Person => None,
            Self::Agent {
                kernel_id,
                room_id,
                agent_id,
                label,
            } => Some(ResponsibleAgent {
                kernel_id,
                room_id,
                agent_id,
                label,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsibleAgent<'a> {
    pub kernel_id: &'a str,
    pub room_id: &'a str,
    pub agent_id: &'a str,
    pub label: &'a str,
}

impl ResponsibleAgent<'_> {
    /// Whether both name the same agent; the label is only how it was called when it was chosen.
    pub fn is_same_agent(&self, other: &ResponsibleAgent<'_>) -> bool {
        (self.kernel_id, self.room_id, self.agent_id)
            == (other.kernel_id, other.room_id, other.agent_id)
    }
}

/// The workflow an agent's responsible trigger listens for. It identifies the workflow on its
/// own kernel, so the trigger can be found again to replace or end it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsibleForWorkflow {
    pub kernel_id: String,
    pub room_id: String,
    pub workflow_id: String,
}

/// Why the responsible went back to the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsibleNoticeKind {
    AgentGone,
    MachineGone,
}

/// Shown once to the person after a fallback, until they set the responsible again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsibleNotice {
    pub kind: ResponsibleNoticeKind,
    pub label: String,
}

/// What a run waits on while it holds. A fixed class, never provider text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowWaitingOn {
    ProviderProfileUnavailable,
    ProviderProfileUnverified,
    ProviderProfileUnauthenticated,
    ProviderProfileUsageExhausted,
    BlockedOnWorkspaceClaim,
}

impl From<WorkflowNodeWaitingReason> for WorkflowWaitingOn {
    fn from(reason: WorkflowNodeWaitingReason) -> Self {
        match reason {
            WorkflowNodeWaitingReason::ProviderProfileUnavailable => {
                Self::ProviderProfileUnavailable
            }
            WorkflowNodeWaitingReason::ProviderProfileUnverified => Self::ProviderProfileUnverified,
            WorkflowNodeWaitingReason::ProviderProfileUnauthenticated => {
                Self::ProviderProfileUnauthenticated
            }
            WorkflowNodeWaitingReason::ProviderProfileUsageExhausted => {
                Self::ProviderProfileUsageExhausted
            }
        }
    }
}

/// How a run that needs someone stands: failed with its first failure, or holding a node on a
/// named thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowRunOutcome {
    Failed {
        reason: WorkflowFailureKind,
        detail: String,
    },
    Waiting {
        on: WorkflowWaitingOn,
        node_id: String,
    },
}

impl WorkflowRunOutcome {
    /// One value per distinct need of a run: a cleared need stays cleared, a different one
    /// opens attention again.
    pub fn key(&self) -> String {
        match self {
            Self::Failed { reason, .. } => format!("failed:{reason:?}"),
            Self::Waiting { on, node_id } => format!("waiting:{node_id}:{on:?}"),
        }
    }
}

fn node_waiting_on(node_run: &WorkflowNodeRun) -> Option<WorkflowWaitingOn> {
    if node_run.status() == WorkflowNodeRunStatus::BlockedOnWorkspaceClaim {
        return Some(WorkflowWaitingOn::BlockedOnWorkspaceClaim);
    }
    node_run.waiting_reason().map(WorkflowWaitingOn::from)
}

impl WorkflowRun {
    /// Present for a failed run and for a run holding a node on a named thing.
    pub fn outcome(&self) -> Option<WorkflowRunOutcome> {
        match self.status() {
            WorkflowRunStatus::Failed => {
                let failure = self.failure_events().first()?;
                if failure.kind() == WorkflowFailureKind::RunStopped {
                    return None;
                }
                let detail = crate::secret_redaction::redact_secrets(failure.message());
                Some(WorkflowRunOutcome::Failed {
                    reason: failure.kind(),
                    detail: detail.chars().take(OUTCOME_DETAIL_CHARS).collect(),
                })
            }
            status if status.is_terminal() => None,
            _ => self.node_runs().iter().find_map(|node_run| {
                node_waiting_on(node_run).map(|on| WorkflowRunOutcome::Waiting {
                    on,
                    node_id: node_run.node_id().to_owned(),
                })
            }),
        }
    }

    /// True while the run needs someone and nobody has cleared that need.
    pub fn attention(&self) -> bool {
        self.outcome()
            .is_some_and(|outcome| self.attention_cleared() != Some(outcome.key().as_str()))
    }

    /// The person opened the run, or a newer run of the workflow started.
    pub fn clear_attention(&mut self) {
        let cleared = self.outcome().map(|outcome| outcome.key());
        self.set_attention_cleared(cleared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsible_shapes_are_tagged_snake_case() {
        assert_eq!(
            serde_json::to_value(WorkflowResponsible::Person).unwrap(),
            serde_json::json!({"kind": "person"})
        );
        let agent = WorkflowResponsible::Agent {
            kernel_id: "k".into(),
            room_id: "r".into(),
            agent_id: "a".into(),
            label: "orch12".into(),
        };
        assert_eq!(
            serde_json::to_value(&agent).unwrap(),
            serde_json::json!({"kind": "agent", "kernel_id": "k", "room_id": "r", "agent_id": "a", "label": "orch12"})
        );
        assert_eq!(
            serde_json::to_value(ResponsibleNotice {
                kind: ResponsibleNoticeKind::MachineGone,
                label: "orch12".into(),
            })
            .unwrap(),
            serde_json::json!({"kind": "machine_gone", "label": "orch12"})
        );
    }

    #[test]
    fn outcome_shapes_match_the_binding_shapes() {
        assert_eq!(
            serde_json::to_value(WorkflowRunOutcome::Failed {
                reason: WorkflowFailureKind::StepNeedsHelp,
                detail: "resend the diff".into(),
            })
            .unwrap(),
            serde_json::json!({"kind": "failed", "reason": "step_needs_help", "detail": "resend the diff"})
        );
        assert_eq!(
            serde_json::to_value(WorkflowRunOutcome::Waiting {
                on: WorkflowWaitingOn::ProviderProfileUsageExhausted,
                node_id: "n1".into(),
            })
            .unwrap(),
            serde_json::json!({"kind": "waiting", "on": "provider_profile_usage_exhausted", "node_id": "n1"})
        );
    }
}
