//! Workflow prompt cancellation and provider-failure transitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let paused = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .is_ok_and(|run| run.status() == crate::session::WorkflowRunStatus::Paused);
        if paused {
            let _ = self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.session_snapshot(session_id)?;
            return Ok(());
        }
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_prompt(session_id);
        self.persist_workflow_runtime_session(session_id, "workflow_prompt_cancelled")?;
        Ok(())
    }

    pub(super) fn workflow_fail_provider_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.workflow_fail_provider_prompt_with_queue_advance(
            session_id,
            prompt,
            provider_run_id,
            message,
            true,
        )
    }

    pub(super) fn workflow_fail_provider_prompt_without_queue_advance(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<(), DaemonError> {
        self.workflow_fail_provider_prompt_with_queue_advance(
            session_id,
            prompt,
            provider_run_id,
            message,
            false,
        )
    }

    fn workflow_fail_provider_prompt_with_queue_advance(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
        advance_queue: bool,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            ),
        );
        let workflow_run = self.session_store.write().fail_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.record_notice(
            session_id,
            provider_run_id,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` failed after provider turn failure: {}",
                workflow_run.id(),
                message
            ),
        );
        if advance_queue {
            self.workflow_maybe_start_next_queued_prompt(session_id);
        }
        self.persist_workflow_runtime_session(session_id, "workflow_provider_prompt_failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::KernelSessionService;
    use crate::config::DaemonConfig;
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
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
        ) = {
            let app = app.lock().await;
            (
                app.config_projection_store(),
                app.session_state_store(),
                app.agents().clone(),
                app.attachments().clone(),
                app.providers().clone(),
                app.provider_process_tracking_store(),
                app.slices(),
                app.session_state_projection_store(),
                app.provider_run_projection_store(),
                app.operational_history_store(),
                app.durable_state_store(),
                app.prompt_state_owner(),
                app.active_turn_store(),
                app.prompt_activity_store(),
                app.prompt_workspace_claim_store(),
                app.structured_output_record_store(),
                app.terminal_stream_store(),
                app.workflow_design_event_store(),
                app.metaagent_event_store(),
                app.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
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

    #[tokio::test]
    async fn workflow_provider_failure_persists_terminal_run_state_for_restart() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-failure",
                "worktree-workflow-failure",
            ))
            .expect("session should create");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("failure-test".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("endpoint should create");
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("fail visibly".to_string()),
            )
            .expect("workflow run should create");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        let prompt = PromptQueueItem::new(
            "prompt-failure",
            "attachment-failure",
            agent.id(),
            "fail visibly",
            PromptStatus::Running,
        )
        .with_workflow_context(workflow_run.id(), &node_run_id);
        let session_id = session.id().to_string();
        let workflow_run_id = workflow_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        runtime
            .owned
            .workflow_fail_provider_prompt(&session_id, &prompt, None, "provider unavailable")
            .expect("provider failure should settle workflow");

        let durable_session = runtime
            .owned
            .durable_state_store
            .load_events_by_kind("session.updated")
            .expect("durable session events should load")
            .into_iter()
            .rev()
            .find(|event| {
                event.subject_id.as_deref() == Some(session_id.as_str())
                    && event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        == Some("workflow_provider_prompt_failed")
            })
            .and_then(|event| event.payload.get("session").cloned())
            .map(serde_json::from_value::<crate::session::RuntimeSession>)
            .transpose()
            .expect("durable workflow session should decode")
            .expect("workflow provider failure should persist its session");
        assert_eq!(
            durable_session
                .workflow_run(&workflow_run_id)
                .expect("durable workflow run should exist")
                .status(),
            crate::session::WorkflowRunStatus::Failed,
        );
    }

    #[tokio::test]
    async fn event_provider_resource_failure_is_retried_once_after_backoff() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-event-retry",
                "worktree-event-retry",
            ))
            .expect("session should create");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("event-retry".to_string()))
            .expect("workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("endpoint should create");
        let publication = app
            .sessions_mut()
            .create_workflow_publication_idempotent(
                session.id(),
                workflow.id(),
                endpoint.id(),
                None,
                None,
                None,
                Some("event-retry-publication".to_string()),
                Some(crate::session::WORKFLOW_PUBLICATION_KIND_EVENT_BASED.to_string()),
                None,
                Vec::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                vec![agent.clone()],
                "local".to_string(),
            )
            .expect("event publication should create");
        let binding = app
            .sessions_mut()
            .create_workflow_event_binding(
                session.id(),
                publication.id(),
                "github".to_string(),
                "1".to_string(),
                "sha256:test".to_string(),
                "connection-event-retry".to_string(),
                "repository:charioxai/chariox".to_string(),
                "pull_request.synchronize".to_string(),
                1,
                serde_json::Value::Null,
                None,
                None,
                Some("disabled".to_string()),
            )
            .expect("event binding should create");
        let invocation = crate::session::WorkflowPublicationInvocationEnvelope {
            publication_id: publication.id().to_string(),
            hook_id: Some(binding.id.clone()),
            invocation_id: "delivery-event-retry".to_string(),
            transport: "event".to_string(),
            endpoint_id: endpoint.id().to_string(),
            queue_ref: None,
            input: serde_json::Value::Null,
            artifacts: Vec::new(),
            mode: None,
            caller: serde_json::Value::Null,
        };
        let node_run = crate::session::WorkflowNodeRun::new(
            "node-run-event-retry",
            node.id(),
            agent.id(),
            0,
            crate::session::WorkflowNodeRunStatus::Failed,
        );
        let mut failed_run = crate::session::WorkflowRun::new(
            "run-event-retry",
            workflow.id(),
            endpoint.id(),
            node.id(),
            Some("review exact PR head".to_string()),
            Some(invocation),
            vec![node_run],
            Vec::new(),
        );
        failed_run.add_failure_event(crate::session::WorkflowFailureEvent::new(
            crate::session::WorkflowFailureKind::ProviderFailure,
            "node-run-event-retry",
            Vec::new(),
            "Provider prompt dispatch failed: You've hit your usage limit.",
        ));
        failed_run.set_status(crate::session::WorkflowRunStatus::Failed);
        let failure_at_ms = failed_run.failure_events()[0].timestamp_ms();
        let session_id = session.id().to_string();
        let mut failed_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should resolve");
        failed_session.create_workflow_run(failed_run);
        app.sessions_mut().restore_session(failed_session);
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let early = runtime
            .owned
            .workflow_collect_due_event_retry_dispatches(failure_at_ms + 29 * 60_000);
        assert!(early.is_empty());
        assert_eq!(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should resolve")
                .workflow_runs()
                .len(),
            1,
        );

        let due = runtime
            .owned
            .workflow_collect_due_event_retry_dispatches(failure_at_ms + 30 * 60_000);
        assert!(due.admitted_workflow_prompt);
        let retried = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("session should resolve");
        assert_eq!(retried.workflow_runs().len(), 2);
        let latest = retried
            .workflow_runs()
            .last()
            .expect("retry run should exist");
        assert_eq!(latest.invocation_prompt(), Some("review exact PR head"));
        assert_eq!(
            latest
                .publication_invocation()
                .map(|invocation| invocation.invocation_id.as_str()),
            Some("delivery-event-retry"),
        );

        let duplicate = runtime
            .owned
            .workflow_collect_due_event_retry_dispatches(failure_at_ms + 31 * 60_000);
        assert!(duplicate.is_empty());
        assert_eq!(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("session should resolve")
                .workflow_runs()
                .len(),
            2,
        );
    }
}
