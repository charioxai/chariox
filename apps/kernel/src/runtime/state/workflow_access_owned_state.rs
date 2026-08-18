//! Workflow ownership and revision guards.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_workflow_request_controlled_by_metaagent(
        &self,
        request: &LocalDaemonRequest,
        metaagent_id: &str,
    ) -> Result<(), DaemonError> {
        match request {
            LocalDaemonRequest::CreateWorkflow(_) | LocalDaemonRequest::ListWorkflows(_) => Ok(()),
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                if let Some(workflow_ref) = request.workflow_ref.as_deref() {
                    self.ensure_workflow_controlled_by_metaagent(
                        &request.session_id,
                        workflow_ref,
                        metaagent_id,
                        "list workflow runs",
                    )
                } else {
                    Ok(())
                }
            }
            LocalDaemonRequest::GetWorkflowRun(request) => self
                .ensure_workflow_run_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_run_ref,
                    metaagent_id,
                    "get workflow run",
                ),
            LocalDaemonRequest::CancelWorkflowRun(request) => self
                .ensure_workflow_run_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_run_ref,
                    metaagent_id,
                    "cancel workflow run",
                ),
            LocalDaemonRequest::PauseWorkflowRun(request) => self
                .ensure_workflow_run_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_run_ref,
                    metaagent_id,
                    "pause workflow run",
                ),
            LocalDaemonRequest::ResumeWorkflowRun(request) => self
                .ensure_workflow_run_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_run_ref,
                    metaagent_id,
                    "resume workflow run",
                ),
            LocalDaemonRequest::AliasWorkflow(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "alias workflow",
                ),
            LocalDaemonRequest::ResolveWorkflow(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "resolve workflow",
                ),
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "create workflow endpoint",
                ),
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "alias workflow endpoint",
                ),
            LocalDaemonRequest::BindWorkflowEndpoint(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "bind workflow endpoint",
                ),
            LocalDaemonRequest::AddWorkflowNode(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "add workflow node",
                ),
            LocalDaemonRequest::RemoveWorkflowNode(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "remove workflow node",
                ),
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "update workflow node instructions",
                ),
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow node completion",
                ),
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow node intermediate output",
                ),
            LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow node input join",
                ),
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow node output schema",
                ),
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow node max turns",
                ),
            LocalDaemonRequest::AddWorkflowEdge(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "add workflow edge",
                ),
            LocalDaemonRequest::RemoveWorkflowEdge(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "remove workflow edge",
                ),
            LocalDaemonRequest::UpdateWorkflowCanvasLayout(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "update workflow canvas",
                ),
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow output schema",
                ),
            LocalDaemonRequest::SetWorkflowFlushContext(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "set workflow context flush",
                ),
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => self
                .ensure_workflow_controlled_by_metaagent(
                    &request.session_id,
                    &request.workflow_ref,
                    metaagent_id,
                    "invoke workflow endpoint",
                ),
            _ => Ok(()),
        }
    }

    pub(super) fn ensure_workflow_controlled_by_metaagent(
        &self,
        session_id: &str,
        workflow_ref: &str,
        metaagent_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        if workflow.controlled_by_metaagent_id() == Some(metaagent_id) {
            Ok(())
        } else {
            Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "workflow `{workflow_ref}` is not controlled by metaagent `{metaagent_id}`"
                ),
            })
        }
    }

    pub(super) fn ensure_workflow_run_controlled_by_metaagent(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        metaagent_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let session = sessions.get_session(session_id)?;
        let workflow_run = sessions
            .resolve_workflow_run_ref(session_id, workflow_run_ref)
            .ok()
            .or(self.durable_state_store.resolve_workflow_run(
                session.host_daemon_id(),
                session.id(),
                workflow_run_ref,
            )?)
            .ok_or_else(|| DaemonError::WorkflowRunNotFound {
                session_id: session_id.to_string(),
                workflow_run_id: workflow_run_ref.to_string(),
            })?;
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
        if workflow.controlled_by_metaagent_id() == Some(metaagent_id) {
            Ok(())
        } else {
            Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "workflow run `{workflow_run_ref}` is not controlled by metaagent `{metaagent_id}`"
                ),
            })
        }
    }

    pub(super) fn deny_owner(
        user_id: &str,
        owner_user_id: &str,
        resource: String,
        operation: &'static str,
    ) -> DaemonError {
        DaemonError::OwnershipAccessDenied {
            user_id: user_id.to_string(),
            owner_user_id: owner_user_id.to_string(),
            resource,
            operation,
        }
    }

    pub(super) fn ensure_workflow_node_editor(
        &self,
        session_id: &str,
        workflow_ref: &str,
        node_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let session = sessions.get_session(session_id)?;
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let node = workflow
            .node(node_id)
            .ok_or_else(|| DaemonError::WorkflowNodeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node_id.to_string(),
            })?;
        let full_collaboration = session
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_prompt_agent_directly());
        if node.owner_user_id() == user_id
            || node.created_by_user_id() == user_id
            || full_collaboration
        {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                node.created_by_user_id(),
                format!("workflow node `{node_id}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_endpoint_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let endpoint =
            sessions.resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?;
        if endpoint.owner_user_id() == user_id {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                endpoint.owner_user_id(),
                format!("workflow endpoint `{endpoint_ref}`"),
                operation,
            ))
        }
    }

    pub(super) fn ensure_workflow_revision(
        &self,
        session_id: &str,
        workflow_ref: &str,
        expected_revision: Option<u64>,
    ) -> Result<(), DaemonError> {
        let Some(expected_revision) = expected_revision else {
            return Ok(());
        };
        let sessions = self.session_store.read();
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let current_revision = workflow.revision();
        if current_revision == expected_revision {
            Ok(())
        } else {
            Err(DaemonError::WorkflowRevisionConflict {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                expected_revision,
                current_revision,
            })
        }
    }

    pub(super) fn ensure_workflow_edge_incident_to_owner(
        &self,
        session_id: &str,
        workflow_ref: &str,
        edge_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let sessions = self.session_store.read();
        let session = sessions.get_session(session_id)?;
        let workflow = sessions.resolve_workflow_ref(session_id, workflow_ref)?;
        let edge = workflow
            .edge(edge_id)
            .ok_or_else(|| DaemonError::WorkflowEdgeNotFound {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                edge_id: edge_id.to_string(),
            })?;
        let full_collaboration = session
            .collaboration_level_for_user(user_id)
            .is_some_and(|level| level.can_prompt_agent_directly());
        let can_edit_endpoint = |node_id: &str| {
            workflow.node(node_id).is_some_and(|node| {
                node.owner_user_id() == user_id || node.created_by_user_id() == user_id
            })
        };
        if full_collaboration
            || can_edit_endpoint(edge.from_node_id())
            || can_edit_endpoint(edge.to_node_id())
            || edge.created_by_user_id() == user_id
        {
            Ok(())
        } else {
            Err(Self::deny_owner(
                user_id,
                edge.created_by_user_id(),
                format!("workflow edge `{edge_id}`"),
                operation,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn runtime_state_from_app(app: crate::app::DaemonApp) -> KernelRuntimeState {
        let config_projection = app.config_projection_store();
        let session_store = app.session_state_store();
        let agent_store = app.agents().clone();
        let attachment_store = app.attachments().clone();
        let provider_store = app.providers().clone();
        let provider_process_tracking = app.provider_process_tracking_store();
        let slice_store = app.slices();
        let session_projection = app.session_state_projection_store();
        let provider_run_projection = app.provider_run_projection_store();
        let operational_history_store = app.operational_history_store();
        let durable_state_store = app.durable_state_store();
        let prompt_state_owner = app.prompt_state_owner();
        let active_turns = app.active_turn_store();
        let prompt_activity = app.prompt_activity_store();
        let prompt_workspace_claims = app.prompt_workspace_claim_store();
        let structured_output_records = app.structured_output_record_store();
        let terminal_stream = app.terminal_stream_store();
        let workflow_design_events = app.workflow_design_event_store();
        let metaagent_events = app.metaagent_event_store();
        let workspace_coordinator = app.workspace_coordinator();
        KernelRuntimeState::new_with_owned_state(
            Arc::new(Mutex::new(app)),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[test]
    fn metaagent_guard_resolves_archived_run_after_restart() {
        let config = crate::config::DaemonConfig::for_tests();
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-archived-metaagent",
                "worktree-archived-metaagent",
            ))
            .expect("session should create");
        let session_id = session.id().to_string();
        let (workflow_id, run_id) = {
            let mut sessions = app.sessions_mut();
            let workflow = sessions
                .create_workflow_controlled_by_metaagent(
                    &session_id,
                    Some("archived-metaagent".to_string()),
                    Some("metaagent-owner".to_string()),
                )
                .expect("controlled workflow should create");
            let node = sessions
                .add_workflow_node(&session_id, workflow.id(), agent.id())
                .expect("workflow node should create");
            let endpoint = sessions
                .create_workflow_endpoint(
                    &session_id,
                    workflow.id(),
                    node.id(),
                    Some("default".to_string()),
                )
                .expect("workflow endpoint should create");
            let run = sessions
                .invoke_workflow_endpoint(
                    &session_id,
                    workflow.id(),
                    endpoint.id(),
                    Some("private archived prompt".to_string()),
                )
                .expect("workflow should invoke");
            let mut durable_session = sessions
                .get_session(&session_id)
                .expect("session should resolve");
            durable_session
                .workflow_run_mut(run.id())
                .expect("run should resolve")
                .set_status(crate::session::WorkflowRunStatus::Completed);
            sessions.restore_session(durable_session.clone());
            (workflow.id().to_string(), run.id().to_string())
        };
        let durable_session = app
            .sessions()
            .get_session(&session_id)
            .expect("durable session should resolve");
        app.durable_state_store()
            .persist_workflow_runtime_transition(&durable_session, "test_completed")
            .expect("terminal run should persist");
        app.sessions_mut()
            .archive_terminal_workflow_runs(&session_id)
            .expect("terminal run should archive");
        assert!(app
            .sessions()
            .resolve_workflow_run_ref(&session_id, &run_id)
            .is_err());
        drop(app);

        let restored = crate::app::DaemonApp::bootstrap(config).expect("daemon should restart");
        let runtime = runtime_state_from_app(restored);
        assert!(runtime
            .owned
            .ensure_workflow_run_controlled_by_metaagent(
                &session_id,
                &run_id,
                "metaagent-owner",
                "get workflow run",
            )
            .is_ok());
        assert!(runtime
            .owned
            .ensure_workflow_run_controlled_by_metaagent(
                &session_id,
                &run_id,
                "other-metaagent",
                "get workflow run",
            )
            .is_err());
        assert_eq!(
            runtime
                .owned
                .durable_state_store
                .resolve_workflow_run("daemon-test", &session_id, &run_id)
                .expect("durable lookup should succeed")
                .expect("archived run should remain durable")
                .workflow_id(),
            workflow_id
        );
    }
}
