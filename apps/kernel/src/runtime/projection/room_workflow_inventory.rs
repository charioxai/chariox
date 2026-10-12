//! Safe room-local workflow read model; never owns workflow runtime state.
use crate::session::{
    RuntimeSession, WorkflowNodeRunStatus, WorkflowQueuedPromptStatus, WorkflowRunStatus,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomWorkflowInventory {
    pub session_id: String,
    pub home_kernel_id: String,
    pub revision: String,
    pub workflow_count: usize,
    pub workflows: Vec<RoomWorkflowSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomWorkflowState {
    Idle,
    Running,
    Paused,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomWorkflowSummary {
    pub workflow_id: String,
    pub workflow_revision: u64,
    pub label: String,
    pub state: RoomWorkflowState,
    pub running_count: usize,
    pub paused_count: usize,
    pub queued_count: usize,
    pub runs: Vec<RoomWorkflowRunSummary>,
    pub endpoints: Vec<RoomWorkflowEndpointSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomWorkflowRunSummary {
    pub run_id: String,
    pub endpoint_id: String,
    pub status: WorkflowRunStatus,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomWorkflowEndpointSummary {
    pub endpoint_id: String,
    pub label: String,
    pub entry_node_id: String,
    pub can_start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_disabled_reason: Option<String>,
}

impl RoomWorkflowInventory {
    pub fn project(session: &RuntimeSession, caller_user_id: &str) -> Self {
        let mut definitions = session.workflows().iter().collect::<Vec<_>>();
        definitions.sort_by(|a, b| (a.created_at_ms(), a.id()).cmp(&(b.created_at_ms(), b.id())));
        let workflows = definitions
            .into_iter()
            .map(|workflow| {
                let runs = session
                    .workflow_runs()
                    .iter()
                    .filter(|run| run.workflow_id() == workflow.id() && !run.status().is_terminal())
                    .map(|run| RoomWorkflowRunSummary {
                        run_id: run.id().to_string(),
                        endpoint_id: run.endpoint_id().to_string(),
                        status: run.status(),
                        can_pause: run.status() != WorkflowRunStatus::Paused,
                        can_resume: run.status() == WorkflowRunStatus::Paused
                            && run.node_runs().iter().any(|node| {
                                node.status() == WorkflowNodeRunStatus::Stopped
                                    && node.completion().is_none()
                                    && node
                                        .turn_envelope()
                                        .and_then(|envelope| envelope.rendered_prompt())
                                        .is_some()
                            }),
                        can_stop: true,
                    })
                    .collect::<Vec<_>>();
                let paused_count = runs
                    .iter()
                    .filter(|run| run.status == WorkflowRunStatus::Paused)
                    .count();
                let running_count = runs.len() - paused_count;
                let state = match (running_count > 0, paused_count > 0) {
                    (true, true) => RoomWorkflowState::Mixed,
                    (true, false) => RoomWorkflowState::Running,
                    (false, true) => RoomWorkflowState::Paused,
                    (false, false) => RoomWorkflowState::Idle,
                };
                RoomWorkflowSummary {
                    workflow_id: workflow.id().into(),
                    workflow_revision: workflow.revision(),
                    label: workflow.alias().unwrap_or(workflow.id()).into(),
                    state,
                    running_count,
                    paused_count,
                    queued_count: session
                        .workflow_queued_prompts()
                        .iter()
                        .filter(|prompt| {
                            prompt.workflow_id() == workflow.id()
                                && prompt.status() == WorkflowQueuedPromptStatus::Queued
                        })
                        .count(),
                    runs,
                    endpoints: workflow
                        .endpoints()
                        .iter()
                        .map(|endpoint| {
                            let can_start = endpoint.owner_user_id() == caller_user_id;
                            RoomWorkflowEndpointSummary {
                                endpoint_id: endpoint.id().into(),
                                label: endpoint.alias().unwrap_or(endpoint.id()).into(),
                                entry_node_id: endpoint.entry_node_id().into(),
                                can_start,
                                start_disabled_reason: (!can_start)
                                    .then(|| "Only the endpoint owner can start it".into()),
                            }
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        let mut inventory = Self {
            session_id: session.id().into(),
            home_kernel_id: session.host_daemon_id().into(),
            revision: String::new(),
            workflow_count: workflows.len(),
            workflows,
        };
        // A content revision changes only for this read model, never transcript tokens.
        inventory.revision = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&inventory).expect("inventory serializes"))
        );
        inventory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        RuntimeSession, WorkflowDefinition, WorkflowEndpointDefinition, WorkflowRun,
        WorkflowRunStatus,
    };

    fn room() -> RuntimeSession {
        RuntimeSession::new("room", None, "workspace", "worktree", "machine", "home")
    }

    #[test]
    fn room_inventory_counts_blank_definitions_and_every_endpoint() {
        let mut room = room();
        assert_eq!(
            RoomWorkflowInventory::project(&room, "local").workflow_count,
            0
        );
        room.create_workflow(WorkflowDefinition::new("blank", None));
        let inventory = RoomWorkflowInventory::project(&room, "local");
        assert_eq!(inventory.workflow_count, 1);
        assert!(inventory.workflows[0].endpoints.is_empty());
        let workflow = room.workflow_mut("blank").unwrap();
        workflow.add_endpoint(WorkflowEndpointDefinition::new(
            "one",
            Some("One".into()),
            "node",
        ));
        workflow.add_endpoint(WorkflowEndpointDefinition::new(
            "two",
            Some("Two".into()),
            "node",
        ));
        let updated = RoomWorkflowInventory::project(&room, "local");
        assert_eq!(updated.workflows[0].endpoints.len(), 2);
        assert_ne!(inventory.revision, updated.revision);
        room.remove_workflow("blank");
        assert_eq!(
            RoomWorkflowInventory::project(&room, "local").workflow_count,
            0
        );
    }

    #[test]
    fn room_inventory_includes_completing_and_paused_but_excludes_terminal_runs() {
        let mut room = room();
        room.create_workflow(WorkflowDefinition::new("workflow", None));
        for (id, status) in [
            ("completing", WorkflowRunStatus::Completing),
            ("paused", WorkflowRunStatus::Paused),
            ("done", WorkflowRunStatus::Completed),
        ] {
            let mut run = WorkflowRun::new(
                id,
                "workflow",
                "endpoint",
                "node",
                Some("private prompt".into()),
                None,
                vec![],
                vec![],
            );
            run.set_status(status);
            room.create_workflow_run(run);
        }
        let inventory = RoomWorkflowInventory::project(&room, "local");
        let workflow = &inventory.workflows[0];
        assert_eq!(workflow.state, RoomWorkflowState::Mixed);
        assert_eq!(workflow.running_count, 1);
        assert_eq!(workflow.paused_count, 1);
        assert_eq!(workflow.runs.len(), 2);
        assert!(!serde_json::to_string(&inventory)
            .unwrap()
            .contains("private prompt"));
    }

    #[test]
    fn room_inventory_start_permission_belongs_to_endpoint_owner() {
        let mut room = room();
        room.create_workflow(WorkflowDefinition::new("workflow", None));
        let mut endpoint = WorkflowEndpointDefinition::new("endpoint", None, "node");
        endpoint.set_owner_user_id("alice");
        room.workflow_mut("workflow")
            .unwrap()
            .add_endpoint(endpoint);
        assert!(RoomWorkflowInventory::project(&room, "alice").workflows[0].endpoints[0].can_start);
        let inventory = RoomWorkflowInventory::project(&room, "bob");
        assert!(!inventory.workflows[0].endpoints[0].can_start);
        assert!(inventory.workflows[0].endpoints[0]
            .start_disabled_reason
            .is_some());
        assert_eq!(inventory.home_kernel_id, "home");
    }
}
