use std::collections::{BTreeMap, BTreeSet};

use crate::config::DaemonConfig;
use crate::error::DaemonError;

use super::{
    unix_epoch_ms, CreateSessionRequest, PromptAttachment, PromptDetachEffect, PromptQueueItem,
    PromptSubmissionOutcome, RuntimeSession, SessionConfigState, SessionStatus, SessionStore,
    WorkflowCompletionSnapshot, WorkflowDefinition, WorkflowEdgeDefinition,
    WorkflowEndpointDefinition, WorkflowHandoffPayload, WorkflowMessage, WorkflowNodeDefinition,
    WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub node_run: WorkflowNodeRun,
    pub messages: Vec<WorkflowMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCompletionUpdate {
    pub workflow_run: WorkflowRun,
    pub dispatches: Vec<WorkflowDispatch>,
}

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    host_machine_id: String,
    host_daemon_id: String,
    next_prompt_number: u64,
    next_workflow_number: u64,
    next_workflow_endpoint_number: u64,
    next_workflow_node_number: u64,
    next_workflow_edge_number: u64,
    next_workflow_run_number: u64,
    next_workflow_node_run_number: u64,
    next_workflow_message_number: u64,
}

impl SessionService {
    pub fn new(config: &DaemonConfig) -> Self {
        Self {
            store: SessionStore::new(),
            host_machine_id: config.host_machine_id.clone(),
            host_daemon_id: config.daemon_id.clone(),
            next_prompt_number: 0,
            next_workflow_number: 0,
            next_workflow_endpoint_number: 0,
            next_workflow_node_number: 0,
            next_workflow_edge_number: 0,
            next_workflow_run_number: 0,
            next_workflow_node_run_number: 0,
            next_workflow_message_number: 0,
        }
    }

    pub fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        let alias = normalize_session_alias(request.alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_alias_available(&request.workspace_id, alias)?;
        }
        let session = RuntimeSession::new(
            self.store.next_session_id(),
            alias,
            request.workspace_id,
            request.worktree_id,
            self.host_machine_id.clone(),
            self.host_daemon_id.clone(),
        );

        Ok(self.store.insert(session))
    }

    pub fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .get(session_id)
            .cloned()
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.store.non_ended_sessions().cloned().collect()
    }

    pub fn list_workflows(&self, session_id: &str) -> Result<Vec<WorkflowDefinition>, DaemonError> {
        Ok(self.get_session(session_id)?.workflows().to_vec())
    }

    pub fn resolve_workflow_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let normalized_ref = workflow_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflows = session.workflows();
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.id() == normalized_ref)
        {
            return Ok(workflow.clone());
        }
        if let Some(workflow) = workflows
            .iter()
            .find(|workflow| workflow.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(workflow.clone());
        }
        let id_matches = workflows
            .iter()
            .filter(|workflow| workflow.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflows
            .iter()
            .filter(|workflow| {
                workflow
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow_ref.to_string(),
        })
    }

    pub fn create_workflow(
        &mut self,
        session_id: &str,
        alias: Option<String>,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let alias = normalize_workflow_alias(alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_alias_available(session_id, alias)?;
        }
        let workflow = WorkflowDefinition::new(self.next_workflow_id(), alias);
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow(workflow))
    }

    pub fn assign_workflow_alias(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        alias: String,
    ) -> Result<WorkflowDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidWorkflowAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;
        self.ensure_workflow_alias_available_for_update(session_id, &workflow_id, &alias)?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        workflow.set_alias(Some(alias));
        Ok(workflow.clone())
    }

    pub fn resolve_workflow_endpoint_ref(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let normalized_ref = endpoint_ref.trim().to_lowercase();
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.id() == normalized_ref)
        {
            return Ok(endpoint.clone());
        }
        if let Some(endpoint) = workflow
            .endpoints()
            .iter()
            .find(|endpoint| endpoint.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(endpoint.clone());
        }
        let id_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| endpoint.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = workflow
            .endpoints()
            .iter()
            .filter(|endpoint| {
                endpoint
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::WorkflowEndpointNotFound {
            session_id: session_id.to_string(),
            workflow_id: workflow.id().to_string(),
            endpoint_id: endpoint_ref.to_string(),
        })
    }

    pub fn list_workflow_runs(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<Vec<WorkflowRun>, DaemonError> {
        let workflow_id = workflow_ref
            .map(|reference| self.resolve_workflow_ref(session_id, reference))
            .transpose()?
            .map(|workflow| workflow.id().to_string());
        let session = self.get_session(session_id)?;
        Ok(session
            .workflow_runs()
            .iter()
            .filter(|workflow_run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|id| workflow_run.workflow_id() == id)
            })
            .cloned()
            .collect())
    }

    pub fn resolve_workflow_run_ref(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let normalized_ref = workflow_run_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let workflow_runs = session.workflow_runs();
        if let Some(workflow_run) = workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == normalized_ref)
        {
            return Ok(workflow_run.clone());
        }
        let id_matches = workflow_runs
            .iter()
            .filter(|workflow_run| workflow_run.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        Err(DaemonError::WorkflowRunNotFound {
            session_id: session_id.to_string(),
            workflow_run_id: workflow_run_ref.to_string(),
        })
    }

    pub fn invoke_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<WorkflowRun, DaemonError> {
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint =
            self.resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        let entry_node = workflow.node(endpoint.entry_node_id()).ok_or_else(|| {
            DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: endpoint.entry_node_id().to_string(),
            }
        })?;

        let node_run = WorkflowNodeRun::new(
            self.next_workflow_node_run_id(),
            entry_node.id().to_string(),
            entry_node.agent_id().to_string(),
            WorkflowNodeRunStatus::Ready,
        );
        let messages = prompt
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                let mut message = WorkflowMessage::new(
                    self.next_workflow_message_id(),
                    None,
                    entry_node.id().to_string(),
                    "invocation",
                    "workflow invoked",
                    value.clone(),
                );
                message.set_consumed_by_node_run_id(node_run.id().to_string());
                vec![message]
            })
            .unwrap_or_default();
        let workflow_run = WorkflowRun::new(
            self.next_workflow_run_id(),
            workflow.id().to_string(),
            endpoint.id().to_string(),
            entry_node.id().to_string(),
            prompt,
            vec![node_run],
            messages,
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_run(workflow_run))
    }

    pub fn cancel_workflow_run(
        &mut self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let workflow_run_id = self
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(&workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.clone(),
            }
        })?;
        if matches!(
            workflow_run.status(),
            WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
        ) {
            return Err(DaemonError::InvalidWorkflowRunState {
                workflow_run_id: workflow_run_id.clone(),
                status: workflow_run.status(),
                operation: "cancel workflow run",
            });
        }
        workflow_run.set_status(WorkflowRunStatus::Stopped);
        workflow_run.clear_active_node_run();
        for node_run in workflow_run.node_runs_mut() {
            if !matches!(
                node_run.status(),
                WorkflowNodeRunStatus::Completed
                    | WorkflowNodeRunStatus::Failed
                    | WorkflowNodeRunStatus::Stopped
            ) {
                node_run.set_status(WorkflowNodeRunStatus::Stopped);
            }
        }
        Ok(workflow_run.clone())
    }

    pub fn start_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::Running);
        workflow_run.set_active_node_run(workflow_node_run_id.to_string());
        workflow_run.set_status(WorkflowRunStatus::Running);
        Ok(workflow_run.clone())
    }

    pub fn complete_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        completion: Option<WorkflowCompletionSnapshot>,
    ) -> Result<WorkflowCompletionUpdate, DaemonError> {
        let (workflow_run, source_node_run, workflow) = {
            let session =
                self.store
                    .get(session_id)
                    .ok_or_else(|| DaemonError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
            let workflow_run = session
                .workflow_run(workflow_run_id)
                .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                    session_id: session_id.to_string(),
                    workflow_run_id: workflow_run_id.to_string(),
                })?
                .clone();
            let source_node_run = workflow_run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == workflow_node_run_id)
                .cloned()
                .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_run.workflow_id().to_string(),
                    reference: workflow_node_run_id.to_string(),
                    message: "workflow node run was not found",
                })?;
            let workflow = session
                .workflow(workflow_run.workflow_id())
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_run.workflow_id().to_string(),
                })?
                .clone();
            (workflow_run, source_node_run, workflow)
        };
        let emitted_messages = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == source_node_run.node_id())
            .map(|edge| {
                let target_node = workflow.node(edge.to_node_id()).ok_or_else(|| {
                    DaemonError::InvalidWorkflowGraphReference {
                        session_id: session_id.to_string(),
                        workflow_id: workflow.id().to_string(),
                        reference: edge.to_node_id().to_string(),
                        message: "target node does not exist",
                    }
                })?;
                let payload = WorkflowHandoffPayload::new(
                    workflow_run.id().to_string(),
                    workflow.id().to_string(),
                    source_node_run.id().to_string(),
                    source_node_run.node_id().to_string(),
                    source_node_run.agent_id().to_string(),
                    target_node.id().to_string(),
                    workflow_run.invocation_prompt().map(str::to_string),
                    completion.clone(),
                );
                let message = WorkflowMessage::new(
                    self.next_workflow_message_id(),
                    Some(source_node_run.id().to_string()),
                    target_node.id().to_string(),
                    "handoff",
                    format!(
                        "handoff from `{}` to `{}`",
                        source_node_run.node_id(),
                        target_node.id()
                    ),
                    serde_json::to_string(&payload).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "serialize workflow handoff payload",
                            message: error.to_string(),
                        }
                    })?,
                );
                Ok(message)
            })
            .collect::<Result<Vec<_>, DaemonError>>()?;

        let next_workflow_node_run_number = &mut self.next_workflow_node_run_number;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id_for_error = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id_for_error,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::Completed);
        node_run.set_summary(Some(
            completion
                .as_ref()
                .map(|value| value.summary().to_string())
                .unwrap_or_else(|| "completed".to_string()),
        ));
        node_run.set_completion(completion);
        workflow_run.clear_active_node_run();
        for message in emitted_messages {
            workflow_run.add_message(message);
        }
        let workflow_id = workflow_run.workflow_id().to_string();
        let dispatches = collect_ready_workflow_dispatches(
            next_workflow_node_run_number,
            session_id,
            &workflow_id,
            &workflow,
            workflow_run,
        )?;
        let has_unconsumed_messages = workflow_run
            .messages()
            .iter()
            .any(|message| message.consumed_by_node_run_id().is_none());
        let has_pending_node_runs = workflow_run.node_runs().iter().any(|node_run| {
            !matches!(
                node_run.status(),
                WorkflowNodeRunStatus::Completed
                    | WorkflowNodeRunStatus::Failed
                    | WorkflowNodeRunStatus::Stopped
            )
        });
        workflow_run.set_status(if has_unconsumed_messages || has_pending_node_runs {
            WorkflowRunStatus::Waiting
        } else {
            WorkflowRunStatus::Completed
        });
        Ok(WorkflowCompletionUpdate {
            workflow_run: workflow_run.clone(),
            dispatches,
        })
    }

    pub fn stop_workflow_node_run(
        &mut self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<WorkflowRun, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow_run = session.workflow_run_mut(workflow_run_id).ok_or_else(|| {
            DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_id.to_string(),
            }
        })?;
        let workflow_id = workflow_run.workflow_id().to_string();
        let node_run = workflow_run
            .node_runs_mut()
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id,
                reference: workflow_node_run_id.to_string(),
                message: "workflow node run was not found",
            })?;
        node_run.set_status(WorkflowNodeRunStatus::Stopped);
        workflow_run.clear_active_node_run();
        workflow_run.set_status(WorkflowRunStatus::Stopped);
        Ok(workflow_run.clone())
    }

    pub fn add_workflow_node(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        agent_id: &str,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let node = WorkflowNodeDefinition::new(self.next_workflow_node_id(), agent_id.to_string());
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        Ok(workflow.add_node(node))
    }

    pub fn remove_workflow_node(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
    ) -> Result<WorkflowNodeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow
            .remove_node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                node_id: node_id.to_string(),
            })
    }

    pub fn add_workflow_edge(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        from_node_id: &str,
        to_node_id: &str,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let edge = WorkflowEdgeDefinition::new(
            self.next_workflow_edge_id(),
            from_node_id.to_string(),
            to_node_id.to_string(),
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        if workflow.node(from_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: from_node_id.to_string(),
                message: "source node does not exist",
            });
        }
        if workflow.node(to_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: to_node_id.to_string(),
                message: "target node does not exist",
            });
        }
        if from_node_id == to_node_id {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: from_node_id.to_string(),
                message: "source and target nodes must be different",
            });
        }
        if workflow.has_edge(from_node_id, to_node_id) {
            return Err(DaemonError::WorkflowEdgeConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                from_node_id: from_node_id.to_string(),
                to_node_id: to_node_id.to_string(),
            });
        }
        Ok(workflow.add_edge(edge))
    }

    pub fn remove_workflow_edge(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
    ) -> Result<WorkflowEdgeDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow
            .remove_edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                edge_id: edge_id.to_string(),
            })
    }

    pub fn create_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        entry_node_id: &str,
        alias: Option<String>,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_endpoint_alias(alias)?;
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_endpoint_alias_available(session_id, &workflow_id, alias)?;
        }
        let endpoint = WorkflowEndpointDefinition::new(
            self.next_workflow_endpoint_id(),
            alias,
            entry_node_id.to_string(),
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        if workflow.node(entry_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: entry_node_id.to_string(),
                message: "entry node does not exist",
            });
        }
        Ok(workflow.add_endpoint(endpoint))
    }

    pub fn assign_workflow_endpoint_alias(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        alias: String,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
            .id()
            .to_string();
        let alias = normalize_workflow_endpoint_alias(Some(alias))?.ok_or_else(|| {
            DaemonError::InvalidWorkflowEndpointAlias {
                alias: String::new(),
                message: "alias cannot be empty",
            }
        })?;
        self.ensure_workflow_endpoint_alias_available_for_update(
            session_id,
            &workflow_id,
            &endpoint_id,
            &alias,
        )?;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
            DaemonError::WorkflowEndpointNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                endpoint_id: endpoint_id.clone(),
            }
        })?;
        endpoint.set_alias(Some(alias));
        Ok(endpoint.clone())
    }

    pub fn bind_workflow_endpoint(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        entry_node_id: &str,
    ) -> Result<WorkflowEndpointDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        if workflow.node(entry_node_id).is_none() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                reference: entry_node_id.to_string(),
                message: "entry node does not exist",
            });
        }
        let endpoint = workflow.endpoint_mut(&endpoint_id).ok_or_else(|| {
            DaemonError::WorkflowEndpointNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.clone(),
                endpoint_id: endpoint_id.clone(),
            }
        })?;
        endpoint.set_entry_node_id(entry_node_id.to_string());
        Ok(endpoint.clone())
    }

    pub fn resolve_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        let normalized_ref = session_ref.trim().to_lowercase();
        if normalized_ref.is_empty() {
            return Err(DaemonError::SessionNotFound {
                session_id: normalized_ref,
            });
        }

        let all_sessions = self.store.non_ended_sessions().cloned().collect::<Vec<_>>();
        let workspace_sessions = all_sessions
            .iter()
            .filter(|session| {
                workspace_id.is_none_or(|workspace| session.workspace_id() == workspace)
            })
            .cloned()
            .collect::<Vec<_>>();

        if let Some(session) = all_sessions
            .iter()
            .find(|session| session.id() == normalized_ref)
        {
            return Ok(session.clone());
        }
        if let Some(session) = workspace_sessions
            .iter()
            .find(|session| session.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(session.clone());
        }

        let id_matches = all_sessions
            .iter()
            .filter(|session| session.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        if id_matches.len() > 1 {
            return Err(DaemonError::AmbiguousSessionRef {
                session_ref: normalized_ref,
                matches: id_matches
                    .into_iter()
                    .map(|session| describe_session_match(&session))
                    .collect(),
            });
        }

        let alias_matches = workspace_sessions
            .iter()
            .filter(|session| {
                session
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        if alias_matches.len() > 1 {
            return Err(DaemonError::AmbiguousSessionRef {
                session_ref: normalized_ref,
                matches: alias_matches
                    .into_iter()
                    .map(|session| describe_session_match(&session))
                    .collect(),
            });
        }

        Err(DaemonError::SessionNotFound {
            session_id: normalized_ref,
        })
    }

    pub fn transition_session(
        &mut self,
        session_id: &str,
        next_status: SessionStatus,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if !session.transition_to(next_status) {
            return Err(DaemonError::InvalidSessionTransition {
                session_id: session_id.to_string(),
                from: session.status(),
                to: next_status,
            });
        }

        Ok(session.clone())
    }

    pub fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.transition_session(session_id, SessionStatus::Ended)
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store
            .remove(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub fn add_attachment_to_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.status() == SessionStatus::Ended {
            let _ = session.transition_to(SessionStatus::Parked);
        }

        session.add_attachment(attachment_id);
        Ok(session.clone())
    }

    pub fn remove_attachment_from_session(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(RuntimeSession, PromptDetachEffect), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "detach")?;

        if !session.remove_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let removed_queued_prompt_count =
            session.remove_queued_prompts_by_attachment(attachment_id);

        Ok((
            session.clone(),
            PromptDetachEffect {
                removed_active_prompt: false,
                removed_queued_prompt_count,
            },
        ))
    }

    pub fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "set active provider run")?;

        session.set_active_provider_run(provider_run_id);

        let target_status = if session.active_provider_run_id().is_some() {
            SessionStatus::Active
        } else if session.status() == SessionStatus::Active {
            SessionStatus::Parked
        } else {
            session.status()
        };

        let _ = session.transition_to(target_status);
        Ok(session.clone())
    }

    pub fn set_focused_agent(
        &mut self,
        session_id: &str,
        agent_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        session.set_focused_agent(agent_id);
        Ok(session.clone())
    }

    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: &str,
        prompt: impl Into<String>,
        attachments: Vec<PromptAttachment>,
    ) -> Result<(RuntimeSession, PromptSubmissionOutcome), DaemonError> {
        let prompt_id = self.next_prompt_id();
        let prompt = PromptQueueItem::new(
            prompt_id,
            attachment_id,
            target_agent_id,
            prompt,
            super::PromptStatus::Queued,
        )
        .with_attachments(attachments);
        let session = self.get_session_mut_for_operation(session_id, "submit prompt")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let outcome = session.submit_prompt(prompt);
        Ok((session.clone(), outcome))
    }

    pub fn submit_workflow_prompt(
        &mut self,
        session_id: &str,
        source_attachment_id: &str,
        target_agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        prompt: impl Into<String>,
    ) -> Result<(RuntimeSession, PromptSubmissionOutcome), DaemonError> {
        let prompt_id = self.next_prompt_id();
        let prompt = PromptQueueItem::new(
            prompt_id,
            source_attachment_id,
            target_agent_id,
            prompt,
            super::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run_id, workflow_node_run_id);
        let session = self.get_session_mut_for_operation(session_id, "submit workflow prompt")?;
        let outcome = session.submit_prompt(prompt);
        Ok((session.clone(), outcome))
    }

    pub fn cancel_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "cancel prompt")?;
        let cancelled =
            session
                .cancel_active_prompt_only()
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session_id.to_string(),
                })?;
        Ok((session.clone(), cancelled))
    }

    pub fn begin_cancelling_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "begin cancelling prompt")?;
        let prompt = session.begin_cancelling_active_prompt().ok_or_else(|| {
            DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            }
        })?;
        Ok((session.clone(), prompt))
    }

    pub fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, PromptQueueItem), DaemonError> {
        let session =
            self.get_session_mut_for_operation(session_id, "finalize prompt cancellation")?;
        let prompt = session
            .finalize_active_prompt_cancellation()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        Ok((session.clone(), prompt))
    }

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete prompt")?;
        let completed =
            session
                .complete_active_prompt_only()
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session_id.to_string(),
                })?;
        Ok((session.clone(), completed))
    }

    pub fn complete_active_prompt_only(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "complete prompt")?;
        let completed =
            session
                .complete_active_prompt_only()
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session_id.to_string(),
                })?;
        Ok((session.clone(), completed))
    }

    pub fn peek_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> Result<Option<super::PromptQueueItem>, DaemonError> {
        let session = self.get_session(session_id)?;
        Ok(session.peek_next_queued_prompt())
    }

    pub fn activate_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "activate next prompt")?;
        let next = session
            .pop_next_queued_prompt()
            .map(|prompt| session.activate_prompt(prompt));
        Ok((session.clone(), next))
    }

    pub fn activate_prompt(
        &mut self,
        session_id: &str,
        prompt: super::PromptQueueItem,
    ) -> Result<(RuntimeSession, super::PromptQueueItem), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "activate prompt")?;
        let active = session.activate_prompt(prompt);
        Ok((session.clone(), active))
    }

    pub fn pop_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<super::PromptQueueItem>), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "pop next prompt")?;
        let next = session.pop_next_queued_prompt();
        Ok((session.clone(), next))
    }

    pub fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        let session = self.get_session_mut_for_operation(session_id, "update config")?;

        if !session.has_attachment(attachment_id) {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        if requires_idle && session.active_prompt().is_some() {
            return Err(DaemonError::ConfigChangeRejectedWhilePromptRunning {
                session_id: session_id.to_string(),
            });
        }

        session.apply_config_changes(values, attachment_id);
        Ok((session.clone(), session.config_state().clone()))
    }

    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    pub fn active_session_count(&self) -> usize {
        self.store.active_session_count()
    }

    fn ensure_alias_available(&self, workspace_id: &str, alias: &str) -> Result<(), DaemonError> {
        if self
            .store
            .non_ended_sessions()
            .any(|session| session.workspace_id() == workspace_id && session.alias() == Some(alias))
        {
            return Err(DaemonError::SessionAliasConflict {
                workspace_id: workspace_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_workflow_alias_available(
        &self,
        session_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        if session
            .workflows()
            .iter()
            .any(|workflow| workflow.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowAliasConflict {
                session_id: session_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_workflow_alias_available_for_update(
        &self,
        session_id: &str,
        workflow_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        if session
            .workflows()
            .iter()
            .any(|workflow| workflow.id() != workflow_id && workflow.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowAliasConflict {
                session_id: session_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_workflow_endpoint_alias_available(
        &self,
        session_id: &str,
        workflow_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let workflow =
            session
                .workflow(workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        if workflow
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowEndpointAliasConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_workflow_endpoint_alias_available_for_update(
        &self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let workflow =
            session
                .workflow(workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        if workflow
            .endpoints()
            .iter()
            .any(|endpoint| endpoint.id() != endpoint_id && endpoint.alias() == Some(alias))
        {
            return Err(DaemonError::WorkflowEndpointAliasConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                alias: alias.to_string(),
            });
        }
        Ok(())
    }

    fn get_session_mut_for_operation(
        &mut self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<&mut RuntimeSession, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id: session_id.to_string(),
                status: session.status(),
                operation,
            });
        }

        Ok(session)
    }

    fn next_prompt_id(&mut self) -> String {
        self.next_prompt_number += 1;
        format!("prompt-{}", self.next_prompt_number)
    }
}

fn collect_ready_workflow_dispatches(
    next_workflow_node_run_number: &mut u64,
    session_id: &str,
    workflow_id: &str,
    workflow: &WorkflowDefinition,
    workflow_run: &mut WorkflowRun,
) -> Result<Vec<WorkflowDispatch>, DaemonError> {
    let target_node_ids = workflow_run
        .messages()
        .iter()
        .filter(|message| message.consumed_by_node_run_id().is_none())
        .map(|message| message.target_node_id().to_string())
        .collect::<BTreeSet<_>>();
    let mut dispatches = Vec::new();

    for target_node_id in target_node_ids {
        if workflow_run.node_runs().iter().any(|node_run| {
            node_run.node_id() == target_node_id
                && !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                )
        }) {
            continue;
        }

        let target_node = workflow.node(&target_node_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_id.to_string(),
                reference: target_node_id.clone(),
                message: "target node does not exist",
            }
        })?;
        let expected_source_node_ids = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == target_node_id)
            .map(|edge| edge.from_node_id().to_string())
            .collect::<BTreeSet<_>>();
        if expected_source_node_ids.is_empty() {
            continue;
        }

        let mut latest_message_index_by_source = BTreeMap::new();
        for (index, message) in workflow_run.messages().iter().enumerate() {
            if message.target_node_id() != target_node_id
                || message.consumed_by_node_run_id().is_some()
            {
                continue;
            }
            let Some(source_node_run_id) = message.source_node_run_id() else {
                continue;
            };
            let Some(source_node_run) = workflow_run
                .node_runs()
                .iter()
                .find(|node_run| node_run.id() == source_node_run_id)
            else {
                continue;
            };
            let source_node_id = source_node_run.node_id().to_string();
            let should_replace = latest_message_index_by_source
                .get(&source_node_id)
                .and_then(|existing_index| workflow_run.messages().get(*existing_index))
                .is_none_or(|existing_message: &WorkflowMessage| {
                    existing_message.created_at_ms() <= message.created_at_ms()
                });
            if should_replace {
                latest_message_index_by_source.insert(source_node_id, index);
            }
        }

        if !expected_source_node_ids
            .iter()
            .all(|source_node_id| latest_message_index_by_source.contains_key(source_node_id))
        {
            continue;
        }

        let node_run = WorkflowNodeRun::new(
            next_workflow_node_run_id(next_workflow_node_run_number),
            target_node.id().to_string(),
            target_node.agent_id().to_string(),
            WorkflowNodeRunStatus::Ready,
        );
        let selected_indices = expected_source_node_ids
            .iter()
            .filter_map(|source_node_id| {
                latest_message_index_by_source.get(source_node_id).copied()
            })
            .collect::<Vec<_>>();
        let selected_messages = selected_indices
            .iter()
            .filter_map(|index| workflow_run.messages().get(*index).cloned())
            .collect::<Vec<_>>();
        for index in selected_indices {
            if let Some(message) = workflow_run.messages_mut().get_mut(index) {
                message.set_consumed_by_node_run_id(node_run.id().to_string());
            }
        }
        let node_run = workflow_run.add_node_run(node_run);
        dispatches.push(WorkflowDispatch {
            node_run,
            messages: selected_messages,
        });
    }

    Ok(dispatches)
}

fn next_workflow_node_run_id(next_workflow_node_run_number: &mut u64) -> String {
    *next_workflow_node_run_number += 1;
    format!("workflow-node-run-{}", next_workflow_node_run_number)
}

fn normalize_session_alias(alias: Option<String>) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidSessionAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized
        .chars()
        .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || matches!(char, '-' | '_'))
    {
        return Err(DaemonError::InvalidSessionAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

fn normalize_workflow_alias(alias: Option<String>) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidWorkflowAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized.chars().all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    }) {
        return Err(DaemonError::InvalidWorkflowAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

fn normalize_workflow_endpoint_alias(alias: Option<String>) -> Result<Option<String>, DaemonError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    let normalized = alias.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(DaemonError::InvalidWorkflowEndpointAlias {
            alias,
            message: "alias cannot be empty",
        });
    }
    if !normalized.chars().all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    }) {
        return Err(DaemonError::InvalidWorkflowEndpointAlias {
            alias,
            message: "alias must use lowercase letters, digits, `-`, or `_`",
        });
    }
    Ok(Some(normalized))
}

impl SessionService {
    fn next_workflow_id(&mut self) -> String {
        loop {
            self.next_workflow_number = self.next_workflow_number.wrapping_add(1);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(self.next_workflow_number);
            let candidate = format!("{:016x}", nanos ^ self.next_workflow_number.rotate_left(11));
            let exists = self
                .store
                .list()
                .iter()
                .flat_map(|session| session.workflows().iter())
                .any(|workflow| workflow.id() == candidate);
            if !exists {
                return candidate;
            }
        }
    }

    fn next_workflow_endpoint_id(&mut self) -> String {
        self.next_workflow_endpoint_number = self.next_workflow_endpoint_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_endpoint_number.rotate_left(9)
        )
    }

    fn next_workflow_node_id(&mut self) -> String {
        self.next_workflow_node_number = self.next_workflow_node_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_node_number.rotate_left(7)
        )
    }

    fn next_workflow_edge_id(&mut self) -> String {
        self.next_workflow_edge_number = self.next_workflow_edge_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_edge_number.rotate_left(5)
        )
    }

    fn next_workflow_run_id(&mut self) -> String {
        self.next_workflow_run_number = self.next_workflow_run_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_run_number.rotate_left(3)
        )
    }

    fn next_workflow_node_run_id(&mut self) -> String {
        self.next_workflow_node_run_number = self.next_workflow_node_run_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_node_run_number.rotate_left(13)
        )
    }

    fn next_workflow_message_id(&mut self) -> String {
        self.next_workflow_message_number = self.next_workflow_message_number.wrapping_add(1);
        format!(
            "{:016x}",
            unix_epoch_ms() ^ self.next_workflow_message_number.rotate_left(15)
        )
    }
}

fn describe_session_match(session: &RuntimeSession) -> String {
    match session.alias() {
        Some(alias) => format!("{} ({alias})", session.id()),
        None => session.id().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::SessionService;
    use crate::config::DaemonConfig;
    use crate::error::DaemonError;
    use crate::session::{
        CreateSessionRequest, PromptSubmissionOutcome, SchedulerState, SessionStatus,
        WorkflowHandoffPayload, WorkflowNodeRunStatus, WorkflowRunStatus, WorktreeIsolationMode,
    };

    fn test_config() -> DaemonConfig {
        DaemonConfig::for_tests()
    }

    #[test]
    fn creates_gets_and_lists_sessions() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        assert_eq!(created.id().len(), 16);
        assert!(created.id().chars().all(|char| char.is_ascii_hexdigit()));
        assert_eq!(created.alias(), None);
        assert_eq!(created.workspace_id(), "workspace-1");
        assert_eq!(created.worktree_id(), "worktree-1");
        assert_eq!(created.host_machine_id(), "machine-test");
        assert_eq!(created.host_daemon_id(), "daemon-test");
        assert_eq!(created.status(), SessionStatus::Created);
        assert!(created.active_provider_run_id().is_none());
        assert!(created.attachment_ids().is_empty());
        assert!(created.active_prompt().is_none());
        assert!(created.queued_prompts().is_empty());
        assert_eq!(created.scheduler_state(), SchedulerState::Idle);
        assert_eq!(created.config_state().version(), 0);
        assert_eq!(created.worktree_assignments().len(), 1);
        assert_eq!(
            created.worktree_assignments()[0].isolation_mode(),
            WorktreeIsolationMode::SharedSession
        );
        assert_eq!(service.active_session_count(), 1);

        let fetched = service
            .get_session(created.id())
            .expect("lookup should succeed");
        assert_eq!(fetched, created);
        assert_eq!(service.list_sessions(), vec![created]);
    }

    #[test]
    fn normalizes_aliases_and_resolves_ids_and_aliases() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_alias(" Feature_Main "),
            )
            .expect("session should be created");

        assert_eq!(created.alias(), Some("feature_main"));
        assert_eq!(
            service
                .resolve_session_ref(created.id(), Some("workspace-1"))
                .expect("full id should resolve")
                .id(),
            created.id()
        );
        assert_eq!(
            service
                .resolve_session_ref(&created.id()[..8], Some("workspace-1"))
                .expect("id prefix should resolve")
                .id(),
            created.id()
        );
        assert_eq!(
            service
                .resolve_session_ref("feature_main", Some("workspace-1"))
                .expect("alias should resolve")
                .id(),
            created.id()
        );
        assert_eq!(
            service
                .resolve_session_ref("feature", Some("workspace-1"))
                .expect("alias prefix should resolve")
                .id(),
            created.id()
        );
    }

    #[test]
    fn rejects_duplicate_alias_in_same_workspace() {
        let mut service = SessionService::new(&test_config());
        service
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
            )
            .expect("first session should be created");

        let error = service
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-2").with_alias("MAIN"),
            )
            .expect_err("duplicate alias should be rejected");

        match error {
            DaemonError::SessionAliasConflict {
                workspace_id,
                alias,
            } => {
                assert_eq!(workspace_id, "workspace-1");
                assert_eq!(alias, "main");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn creates_lists_and_resolves_workflows_by_id_and_alias_prefix() {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let first = service
            .create_workflow(session.id(), Some("review_loop".to_string()))
            .expect("workflow should be created");
        let second = service
            .create_workflow(session.id(), Some("deploy".to_string()))
            .expect("workflow should be created");

        let workflows = service
            .list_workflows(session.id())
            .expect("workflow list should succeed");
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0], first);
        assert_eq!(workflows[1], second);

        let unique_prefix_len = (1..=first.id().len())
            .find(|length| {
                let prefix = &first.id()[..*length];
                workflows
                    .iter()
                    .filter(|workflow| workflow.id().starts_with(prefix))
                    .count()
                    == 1
            })
            .expect("workflow id should have a unique prefix");
        let unique_prefix = &first.id()[..unique_prefix_len];

        assert_eq!(
            service
                .resolve_workflow_ref(session.id(), first.id())
                .expect("workflow id should resolve")
                .id(),
            first.id()
        );
        assert_eq!(
            service
                .resolve_workflow_ref(session.id(), unique_prefix)
                .expect("workflow id prefix should resolve")
                .id(),
            first.id()
        );
        assert_eq!(
            service
                .resolve_workflow_ref(session.id(), "review_loop")
                .expect("workflow alias should resolve")
                .id(),
            first.id()
        );
        assert_eq!(
            service
                .resolve_workflow_ref(session.id(), "review")
                .expect("workflow alias prefix should resolve")
                .id(),
            first.id()
        );
    }

    #[test]
    fn manages_workflow_nodes_edges_and_endpoints() {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let workflow = service
            .create_workflow(session.id(), Some("review".to_string()))
            .expect("workflow should be created");

        let planner = service
            .add_workflow_node(session.id(), workflow.id(), "agent-1")
            .expect("planner node should be added");
        let reviewer = service
            .add_workflow_node(session.id(), workflow.id(), "agent-2")
            .expect("reviewer node should be added");

        let edge = service
            .add_workflow_edge(session.id(), workflow.id(), planner.id(), reviewer.id())
            .expect("edge should be added");
        assert_eq!(edge.from_node_id(), planner.id());
        assert_eq!(edge.to_node_id(), reviewer.id());

        let duplicate_edge = service
            .add_workflow_edge(session.id(), workflow.id(), planner.id(), reviewer.id())
            .expect_err("duplicate edge should be rejected");
        assert!(matches!(
            duplicate_edge,
            DaemonError::WorkflowEdgeConflict { .. }
        ));

        let self_edge = service
            .add_workflow_edge(session.id(), workflow.id(), planner.id(), planner.id())
            .expect_err("self edge should be rejected");
        assert!(matches!(
            self_edge,
            DaemonError::InvalidWorkflowGraphReference { .. }
        ));

        let endpoint = service
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                planner.id(),
                Some("entry".to_string()),
            )
            .expect("endpoint should be created");
        assert_eq!(endpoint.entry_node_id(), planner.id());

        assert_eq!(
            service
                .resolve_workflow_endpoint_ref(session.id(), workflow.id(), "entry")
                .expect("endpoint alias should resolve")
                .id(),
            endpoint.id()
        );

        let rebound = service
            .bind_workflow_endpoint(session.id(), workflow.id(), endpoint.id(), reviewer.id())
            .expect("endpoint should be rebound");
        assert_eq!(rebound.entry_node_id(), reviewer.id());

        let aliased = service
            .assign_workflow_endpoint_alias(
                session.id(),
                workflow.id(),
                endpoint.id(),
                "review-entry".to_string(),
            )
            .expect("endpoint alias should be updated");
        assert_eq!(aliased.alias(), Some("review-entry"));

        let removed_edge = service
            .remove_workflow_edge(session.id(), workflow.id(), edge.id())
            .expect("edge should be removed");
        assert_eq!(removed_edge.id(), edge.id());

        let removed_node = service
            .remove_workflow_node(session.id(), workflow.id(), planner.id())
            .expect("node should be removed");
        assert_eq!(removed_node.id(), planner.id());
    }

    #[test]
    fn creates_lists_resolves_and_cancels_workflow_runs() {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let workflow = service
            .create_workflow(session.id(), Some("review".to_string()))
            .expect("workflow should be created");
        let node = service
            .add_workflow_node(session.id(), workflow.id(), "agent-1")
            .expect("workflow node should be added");
        let endpoint = service
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");

        let workflow_run = service
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review this diff".to_string()),
            )
            .expect("workflow run should be created");
        assert_eq!(workflow_run.workflow_id(), workflow.id());
        assert_eq!(workflow_run.endpoint_id(), endpoint.id());
        assert_eq!(workflow_run.entry_node_id(), node.id());
        assert_eq!(workflow_run.status(), WorkflowRunStatus::Created);
        assert_eq!(workflow_run.node_runs().len(), 1);
        assert_eq!(
            workflow_run.node_runs()[0].status(),
            WorkflowNodeRunStatus::Ready
        );
        assert_eq!(workflow_run.messages().len(), 1);
        assert_eq!(workflow_run.messages()[0].target_node_id(), node.id());

        let listed = service
            .list_workflow_runs(session.id(), Some(workflow.id()))
            .expect("workflow runs should list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id(), workflow_run.id());

        let resolved = service
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve");
        assert_eq!(resolved.id(), workflow_run.id());

        let cancelled = service
            .cancel_workflow_run(session.id(), workflow_run.id())
            .expect("workflow run should cancel");
        assert_eq!(cancelled.status(), WorkflowRunStatus::Stopped);
        assert_eq!(cancelled.active_node_run_id(), None);
        assert_eq!(
            cancelled.node_runs()[0].status(),
            WorkflowNodeRunStatus::Stopped
        );

        let error = service
            .cancel_workflow_run(session.id(), workflow_run.id())
            .expect_err("terminal workflow run should reject a second cancellation");
        assert!(matches!(error, DaemonError::InvalidWorkflowRunState { .. }));
    }

    #[test]
    fn completing_a_workflow_node_run_creates_structured_downstream_dispatches() {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let workflow = service
            .create_workflow(session.id(), Some("review".to_string()))
            .expect("workflow should be created");
        let first = service
            .add_workflow_node(session.id(), workflow.id(), "agent-1")
            .expect("first workflow node should be added");
        let second = service
            .add_workflow_node(session.id(), workflow.id(), "agent-2")
            .expect("second workflow node should be added");
        service
            .add_workflow_edge(session.id(), workflow.id(), first.id(), second.id())
            .expect("workflow edge should be added");
        let endpoint = service
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                first.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");

        let workflow_run = service
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review this diff".to_string()),
            )
            .expect("workflow run should be created");
        let started = service
            .start_workflow_node_run(
                session.id(),
                workflow_run.id(),
                workflow_run.node_runs()[0].id(),
            )
            .expect("entry node should start");
        assert_eq!(started.status(), WorkflowRunStatus::Running);
        assert_eq!(
            started.active_node_run_id(),
            Some(workflow_run.node_runs()[0].id())
        );

        let completion = service
            .complete_workflow_node_run(
                session.id(),
                workflow_run.id(),
                workflow_run.node_runs()[0].id(),
                None,
            )
            .expect("entry node completion should route downstream work");
        assert_eq!(completion.workflow_run.status(), WorkflowRunStatus::Waiting);
        assert_eq!(completion.dispatches.len(), 1);
        assert_eq!(completion.dispatches[0].node_run.node_id(), second.id());
        assert_eq!(completion.dispatches[0].messages.len(), 1);
        assert_eq!(
            completion.dispatches[0].messages[0].target_node_id(),
            second.id()
        );
        let payload: WorkflowHandoffPayload =
            serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
                .expect("handoff payload should deserialize");
        assert_eq!(payload.workflow_run_id(), workflow_run.id());
        assert_eq!(payload.workflow_id(), workflow.id());
        assert_eq!(
            payload.source_node_run_id(),
            workflow_run.node_runs()[0].id()
        );
        assert_eq!(payload.source_node_id(), first.id());
        assert_eq!(payload.source_agent_id(), "agent-1");
        assert_eq!(payload.target_node_id(), second.id());
        assert_eq!(payload.invocation_prompt(), Some("review this diff"));
        assert!(payload.completion().is_none());

        let resolved = service
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve");
        assert_eq!(resolved.status(), WorkflowRunStatus::Waiting);
        assert_eq!(resolved.node_runs().len(), 2);
        assert_eq!(resolved.messages().len(), 2);
    }

    #[test]
    fn join_nodes_wait_for_all_inputs_before_dispatching_once() {
        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let workflow = service
            .create_workflow(session.id(), Some("join".to_string()))
            .expect("workflow should be created");
        let entry = service
            .add_workflow_node(session.id(), workflow.id(), "agent-1")
            .expect("entry node should be added");
        let branch_one = service
            .add_workflow_node(session.id(), workflow.id(), "agent-2")
            .expect("branch one node should be added");
        let branch_two = service
            .add_workflow_node(session.id(), workflow.id(), "agent-3")
            .expect("branch two node should be added");
        let join = service
            .add_workflow_node(session.id(), workflow.id(), "agent-4")
            .expect("join node should be added");
        service
            .add_workflow_edge(session.id(), workflow.id(), entry.id(), branch_one.id())
            .expect("entry should connect to branch one");
        service
            .add_workflow_edge(session.id(), workflow.id(), entry.id(), branch_two.id())
            .expect("entry should connect to branch two");
        service
            .add_workflow_edge(session.id(), workflow.id(), branch_one.id(), join.id())
            .expect("branch one should connect to join");
        service
            .add_workflow_edge(session.id(), workflow.id(), branch_two.id(), join.id())
            .expect("branch two should connect to join");
        let endpoint = service
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                entry.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");

        let workflow_run = service
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("run the join drill".to_string()),
            )
            .expect("workflow run should be created");
        let started = service
            .start_workflow_node_run(
                session.id(),
                workflow_run.id(),
                workflow_run.node_runs()[0].id(),
            )
            .expect("entry node should start");
        let entry_completion = service
            .complete_workflow_node_run(
                session.id(),
                workflow_run.id(),
                started.node_runs()[0].id(),
                None,
            )
            .expect("entry node should dispatch both branches");
        assert_eq!(entry_completion.dispatches.len(), 2);

        let branch_one_run = entry_completion
            .dispatches
            .iter()
            .find(|dispatch| dispatch.node_run.node_id() == branch_one.id())
            .expect("branch one dispatch should exist")
            .node_run
            .clone();
        let branch_two_run = entry_completion
            .dispatches
            .iter()
            .find(|dispatch| dispatch.node_run.node_id() == branch_two.id())
            .expect("branch two dispatch should exist")
            .node_run
            .clone();
        service
            .start_workflow_node_run(session.id(), workflow_run.id(), branch_one_run.id())
            .expect("branch one should start");
        let branch_one_completion = service
            .complete_workflow_node_run(session.id(), workflow_run.id(), branch_one_run.id(), None)
            .expect("branch one completion should succeed");
        assert!(branch_one_completion.dispatches.is_empty());
        let waiting = service
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve after first branch");
        assert_eq!(waiting.node_runs().len(), 3);
        assert_eq!(
            waiting
                .messages()
                .iter()
                .filter(|message| message.target_node_id() == join.id())
                .count(),
            1
        );
        assert!(waiting
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .all(|message| message.consumed_by_node_run_id().is_none()));

        service
            .start_workflow_node_run(session.id(), workflow_run.id(), branch_two_run.id())
            .expect("branch two should start");
        let branch_two_completion = service
            .complete_workflow_node_run(session.id(), workflow_run.id(), branch_two_run.id(), None)
            .expect("branch two completion should succeed");
        assert_eq!(branch_two_completion.dispatches.len(), 1);
        let join_dispatch = &branch_two_completion.dispatches[0];
        assert_eq!(join_dispatch.node_run.node_id(), join.id());
        assert_eq!(join_dispatch.messages.len(), 2);
        assert_eq!(
            join_dispatch
                .messages
                .iter()
                .map(|message| message.target_node_id())
                .collect::<Vec<_>>(),
            vec![join.id(), join.id()]
        );
        let resolved = service
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve");
        assert_eq!(resolved.node_runs().len(), 4);
        assert_eq!(
            resolved
                .messages()
                .iter()
                .filter(|message| message.target_node_id() == join.id())
                .count(),
            2
        );
        assert!(resolved
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join.id())
            .all(|message| message.consumed_by_node_run_id() == Some(join_dispatch.node_run.id())));
    }

    #[test]
    fn delete_session_removes_it_from_registry() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let deleted = service
            .delete_session(created.id())
            .expect("session should delete");

        assert_eq!(deleted.id(), created.id());
        assert!(matches!(
            service.get_session(created.id()),
            Err(DaemonError::SessionNotFound { .. })
        ));
        assert!(service.list_sessions().is_empty());
    }

    #[test]
    fn prompt_queue_starts_then_queues_then_advances() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");
        service
            .add_attachment_to_session(created.id(), "attachment-2")
            .expect("attachment should be added");

        let (_, first) = service
            .submit_prompt(
                created.id(),
                "attachment-1",
                "agent-1",
                "first prompt",
                Vec::new(),
            )
            .expect("first prompt should start");
        let (_, second) = service
            .submit_prompt(
                created.id(),
                "attachment-2",
                "agent-1",
                "second prompt",
                Vec::new(),
            )
            .expect("second prompt should queue");

        match first {
            PromptSubmissionOutcome::Started { prompt } => assert_eq!(prompt.id(), "prompt-1"),
            _ => panic!("expected running prompt"),
        }
        match second {
            PromptSubmissionOutcome::Queued { prompt } => assert_eq!(prompt.id(), "prompt-2"),
            _ => panic!("expected queued prompt"),
        }

        assert_eq!(
            service
                .get_session(created.id())
                .expect("session should exist")
                .scheduler_state(),
            SchedulerState::Waiting
        );

        let (_session, completed) = service
            .complete_active_prompt(created.id())
            .expect("active prompt should complete");
        assert_eq!(completed.id(), "prompt-1");
        let (session, started_next) = service
            .activate_next_queued_prompt(created.id())
            .expect("next prompt should activate");
        assert_eq!(
            started_next.expect("next prompt should start").id(),
            "prompt-2"
        );
        assert_eq!(
            session.active_prompt().expect("active prompt exists").id(),
            "prompt-2"
        );
        assert_eq!(session.scheduler_state(), SchedulerState::Running);
    }

    #[test]
    fn config_updates_are_versioned() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");

        let mut changes = BTreeMap::new();
        changes.insert("theme".to_string(), "compact".to_string());
        let (_, config) = service
            .update_config(created.id(), "attachment-1", changes, false)
            .expect("config should update");

        assert_eq!(config.version(), 1);
        assert_eq!(
            config.values().get("theme").map(String::as_str),
            Some("compact")
        );
        assert_eq!(config.updated_by_attachment_id(), Some("attachment-1"));
    }

    #[test]
    fn rejects_idle_required_config_update_while_prompt_running() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");
        service
            .submit_prompt(
                created.id(),
                "attachment-1",
                "agent-1",
                "first prompt",
                Vec::new(),
            )
            .expect("prompt should start");

        let error = service
            .update_config(created.id(), "attachment-1", BTreeMap::new(), true)
            .expect_err("idle-required config change should be rejected");

        match error {
            DaemonError::ConfigChangeRejectedWhilePromptRunning { session_id } => {
                assert_eq!(session_id, created.id())
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn detaching_an_attachment_keeps_its_active_prompt_running() {
        let mut service = SessionService::new(&test_config());
        let created = service
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        service
            .add_attachment_to_session(created.id(), "attachment-1")
            .expect("attachment should be added");

        let (_, outcome) = service
            .submit_prompt(
                created.id(),
                "attachment-1",
                "agent-1",
                "background prompt",
                Vec::new(),
            )
            .expect("prompt should start");
        let prompt_id = match outcome {
            PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            other => panic!("expected running prompt, got {other:?}"),
        };

        let (session, effect) = service
            .remove_attachment_from_session(created.id(), "attachment-1")
            .expect("detach should succeed");

        assert!(!effect.removed_active_prompt);
        assert_eq!(effect.removed_queued_prompt_count, 0);
        assert!(session.attachment_ids().is_empty());
        assert_eq!(
            session.active_prompt().map(|prompt| prompt.id()),
            Some(prompt_id.as_str())
        );
        assert_eq!(session.scheduler_state(), SchedulerState::Running);
    }
}
