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
        let already_interrupted = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .is_ok_and(|run| {
                matches!(
                    run.status(),
                    crate::session::WorkflowRunStatus::Paused
                        | crate::session::WorkflowRunStatus::Stopped
                )
            });
        if already_interrupted {
            let _ = self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            let _ = self.session_snapshot(session_id)?;
            return Ok(());
        }
        let activity_mutation = self.begin_managed_activity_mutation();
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
        self.persist_workflow_runtime_session_with_activity_mutation(
            session_id,
            "workflow_prompt_cancelled",
            activity_mutation,
        )?;
        self.workflow_maybe_start_next_queued_prompt(session_id);
        Ok(())
    }

    pub(super) fn workflow_fail_provider_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(WorkflowPromptDispatches::default());
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        self.workflow_fail_provider_prompt_state(session_id, prompt, provider_run_id, message)?;
        self.persist_workflow_runtime_session_with_activity_mutation(
            session_id,
            "workflow_provider_prompt_failed",
            activity_mutation,
        )?;
        let dispatches = self.workflow_maybe_start_next_queued_prompt(session_id);
        Ok(dispatches)
    }

    pub(super) fn workflow_fail_provider_prompt_without_queue_advance(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<bool, DaemonError> {
        if prompt.workflow_run_id().is_none() || prompt.workflow_node_run_id().is_none() {
            return Ok(false);
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        let released_claim =
            self.workflow_fail_provider_prompt_state(session_id, prompt, provider_run_id, message)?;
        self.persist_workflow_runtime_session_with_activity_mutation(
            session_id,
            "workflow_provider_prompt_failed",
            activity_mutation,
        )?;
        Ok(released_claim)
    }

    fn workflow_fail_provider_prompt_state(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<bool, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(false);
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
        let released_claim = self.release_workflow_node_workspace_claim(
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
        Ok(released_claim)
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

    struct TestWorktree(std::path::PathBuf);

    impl Drop for TestWorktree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct WorkflowFailureFixture {
        runtime: KernelRuntimeState,
        session_id: String,
        host_daemon_id: String,
        attachment_id: String,
        workflow_run_id: String,
        workflow_node_run_id: String,
        prompt: PromptQueueItem,
        claim_id: String,
        _worktree: TestWorktree,
    }

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

    async fn running_workflow_fixture(label: &str) -> WorkflowFailureFixture {
        let worktree = TestWorktree(std::env::temp_dir().join(format!(
            "chariox-workflow-{label}-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        )));
        std::fs::create_dir_all(&worktree.0).expect("workflow test worktree should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                format!("workspace-{label}"),
                worktree.0.to_string_lossy(),
            ))
            .expect("session should create");
        let attachment = KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                format!("client-{label}"),
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("test client should attach");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some(format!("workflow-{label}")))
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
                Some(format!("run {label}")),
            )
            .expect("workflow run should create");
        let workflow_node_run_id = workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &workflow_node_run_id,
                format!("workflow-ack:{workflow_node_run_id}"),
                format!("run {label}"),
                None,
                None,
            )
            .expect("workflow turn should prepare");
        app.sessions_mut()
            .start_workflow_node_run(
                session.id(),
                workflow_run.id(),
                &workflow_node_run_id,
            )
            .expect("workflow node should start");
        let prompt = PromptQueueItem::new(
            format!("prompt-{label}"),
            attachment.id(),
            agent.id(),
            format!("run {label}"),
            PromptStatus::Running,
        )
        .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
        let session_id = session.id().to_string();
        let host_daemon_id = session.host_daemon_id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let workflow_run_id = workflow_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let claim_id = runtime.owned.workflow_dispatch_claim_id(
            &session_id,
            &workflow_run_id,
            &workflow_node_run_id,
        );
        runtime
            .owned
            .acquire_workflow_node_workspace_claim(
                &session_id,
                &claim_id,
                &agent_id,
                &workflow_run_id,
                &workflow_node_run_id,
            )
            .expect("workflow node should own its workspace claim");
        runtime
            .ensure_managed_activity_tracking(&format!("kernel-{label}"))
            .expect("managed activity tracking should activate");
        runtime
            .owned
            .session_snapshot(&session_id)
            .expect("running workflow should publish a baseline projection");
        WorkflowFailureFixture {
            runtime,
            session_id,
            host_daemon_id,
            attachment_id,
            workflow_run_id,
            workflow_node_run_id,
            prompt,
            claim_id,
            _worktree: worktree,
        }
    }

    fn install_workflow_append_failure(
        fixture: &WorkflowFailureFixture,
        reason: &str,
    ) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(
            fixture.runtime.owned.durable_state_store.path(),
        )
        .expect("durable database should open for failure injection");
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER fail_workflow_prompt_failure_append
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'workflow.runtime.updated'
                   AND json_extract(NEW.payload_json, '$.reason') = '{reason}'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected workflow prompt failure append');
                 END;"
            ))
            .expect("workflow prompt failure trigger should install");
        connection
    }

    fn assert_failed_transition_remains_retryable(
        fixture: &WorkflowFailureFixture,
        projection_sequence: u64,
        activity_sequence: u64,
        notice_count: usize,
    ) {
        let session = fixture
            .runtime
            .owned
            .session_store
            .get_session(&fixture.session_id)
            .expect("session should remain available");
        let workflow_run = session
            .workflow_run(&fixture.workflow_run_id)
            .expect("failed append must retain the workflow run");
        assert_eq!(workflow_run.status(), crate::session::WorkflowRunStatus::Running);
        assert_eq!(
            workflow_run
                .node_runs()
                .iter()
                .find(|node| node.id() == fixture.workflow_node_run_id.as_str())
                .expect("workflow node should remain available")
                .status(),
            crate::session::WorkflowNodeRunStatus::Running
        );
        assert!(workflow_run.failure_events().is_empty());
        assert!(fixture
            .runtime
            .owned
            .prompt_workspace_claims
            .contains(&fixture.claim_id));
        assert_eq!(
            fixture.runtime.owned.session_projection.change_sequence(),
            projection_sequence,
            "a rejected workflow transition must not publish a terminal projection"
        );
        assert_eq!(
            fixture
                .runtime
                .owned
                .session_projection
                .get(&fixture.session_id)
                .expect("baseline projection should remain available")
                .workflow_run(&fixture.workflow_run_id)
                .expect("projected workflow should remain active")
                .status(),
            crate::session::WorkflowRunStatus::Running
        );
        assert_eq!(
            fixture.runtime.managed_activity_change_sequence(),
            activity_sequence,
            "a rejected workflow transition must not publish activity"
        );
        assert_eq!(
            fixture.runtime.owned.terminal_stream.notice_records().len(),
            notice_count,
            "a rejected workflow transition must not emit its failure notice"
        );
        let later_projection = fixture
            .runtime
            .owned
            .session_snapshot(&fixture.session_id)
            .expect("a later session projection should succeed");
        assert_eq!(
            later_projection
                .workflow_run(&fixture.workflow_run_id)
                .expect("later projection should retain the workflow run")
                .status(),
            crate::session::WorkflowRunStatus::Running,
            "a later snapshot must not leak the rejected terminal transition"
        );
        assert_eq!(
            fixture.runtime.owned.session_projection.change_sequence(),
            projection_sequence
        );
    }

    fn committed_workflow_event_count(fixture: &WorkflowFailureFixture, reason: &str) -> usize {
        fixture
            .runtime
            .owned
            .durable_state_store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("workflow events should load")
            .into_iter()
            .filter(|event| event.payload["reason"] == reason)
            .count()
    }

    #[tokio::test]
    async fn workflow_cancellation_append_failure_retains_run_claim_and_notice_for_retry() {
        let fixture = running_workflow_fixture("cancel-append-retry").await;
        let projection_sequence = fixture.runtime.owned.session_projection.change_sequence();
        let activity_sequence = fixture.runtime.managed_activity_change_sequence();
        let notice_count = fixture.runtime.owned.terminal_stream.notice_records().len();
        let connection = install_workflow_append_failure(&fixture, "workflow_prompt_cancelled");

        let error = fixture
            .runtime
            .owned
            .workflow_cancel_prompt(&fixture.session_id, &fixture.prompt)
            .expect_err("failed append must reject workflow cancellation");
        assert!(
            error
                .to_string()
                .contains("injected workflow prompt failure append"),
            "{error}"
        );
        assert_failed_transition_remains_retryable(
            &fixture,
            projection_sequence,
            activity_sequence,
            notice_count,
        );
        assert!(fixture
            .runtime
            .owned
            .terminal_stream
            .drain_workflow_run_updates(&fixture.session_id, &fixture.attachment_id)
            .is_empty());
        assert_eq!(
            committed_workflow_event_count(&fixture, "workflow_prompt_cancelled"),
            0
        );

        connection
            .execute_batch("DROP TRIGGER fail_workflow_prompt_failure_append;")
            .expect("failure trigger should clear");
        fixture
            .runtime
            .owned
            .workflow_cancel_prompt(&fixture.session_id, &fixture.prompt)
            .expect("retained cancellation should retry after storage recovery");
        assert!(!fixture
            .runtime
            .owned
            .prompt_workspace_claims
            .contains(&fixture.claim_id));
        assert_eq!(
            fixture.runtime.owned.terminal_stream.notice_records().len(),
            notice_count + 1
        );
        assert_eq!(
            committed_workflow_event_count(&fixture, "workflow_prompt_cancelled"),
            1
        );
        let durable_run = fixture
            .runtime
            .owned
            .durable_state_store
            .resolve_workflow_run(
                &fixture.host_daemon_id,
                &fixture.session_id,
                &fixture.workflow_run_id,
            )
            .expect("durable workflow run should load")
            .expect("durable workflow run should exist");
        assert_eq!(durable_run.status(), crate::session::WorkflowRunStatus::Stopped);
    }

    #[tokio::test]
    async fn workflow_provider_failure_append_failure_retains_run_claim_and_notice_for_retry() {
        let fixture = running_workflow_fixture("provider-failure-append-retry").await;
        let projection_sequence = fixture.runtime.owned.session_projection.change_sequence();
        let activity_sequence = fixture.runtime.managed_activity_change_sequence();
        let notice_count = fixture.runtime.owned.terminal_stream.notice_records().len();
        let connection =
            install_workflow_append_failure(&fixture, "workflow_provider_prompt_failed");

        let error = fixture
            .runtime
            .owned
            .workflow_fail_provider_prompt_without_queue_advance(
                &fixture.session_id,
                &fixture.prompt,
                None,
                "provider unavailable",
            )
            .expect_err("failed append must reject workflow provider failure");
        assert!(
            error
                .to_string()
                .contains("injected workflow prompt failure append"),
            "{error}"
        );
        assert_failed_transition_remains_retryable(
            &fixture,
            projection_sequence,
            activity_sequence,
            notice_count,
        );
        assert!(fixture
            .runtime
            .owned
            .terminal_stream
            .drain_workflow_run_updates(&fixture.session_id, &fixture.attachment_id)
            .is_empty());
        assert_eq!(
            committed_workflow_event_count(&fixture, "workflow_provider_prompt_failed"),
            0
        );

        connection
            .execute_batch("DROP TRIGGER fail_workflow_prompt_failure_append;")
            .expect("failure trigger should clear");
        assert!(fixture
            .runtime
            .owned
            .workflow_fail_provider_prompt_without_queue_advance(
                &fixture.session_id,
                &fixture.prompt,
                None,
                "provider unavailable",
            )
            .expect("retained provider failure should retry after storage recovery"));
        assert!(!fixture
            .runtime
            .owned
            .prompt_workspace_claims
            .contains(&fixture.claim_id));
        assert_eq!(
            fixture.runtime.owned.terminal_stream.notice_records().len(),
            notice_count + 1
        );
        assert_eq!(
            committed_workflow_event_count(&fixture, "workflow_provider_prompt_failed"),
            1
        );
        let durable_run = fixture
            .runtime
            .owned
            .durable_state_store
            .resolve_workflow_run(
                &fixture.host_daemon_id,
                &fixture.session_id,
                &fixture.workflow_run_id,
            )
            .expect("durable workflow run should load")
            .expect("durable workflow run should exist");
        assert_eq!(durable_run.status(), crate::session::WorkflowRunStatus::Failed);
    }

    #[tokio::test]
    async fn workflow_provider_failure_persists_terminal_run_state_for_restart() {
        let worktree = TestWorktree(std::env::temp_dir().join(format!(
            "chariox-workflow-failure-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        )));
        std::fs::create_dir_all(&worktree.0).expect("workflow test worktree should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-failure",
                worktree.0.to_string_lossy(),
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
        app.sessions_mut()
            .enqueue_workflow_prompt(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("review the next exact revision".to_string()),
                None,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
            )
            .expect("a subsequent workflow invocation should queue");
        let session_id = session.id().to_string();
        let workflow_run_id = workflow_run.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let activity_lock = Arc::clone(&runtime.owned.managed_activity_mutation_lock);
        super::super::workflow_completion_owned_state::
            set_before_workflow_activity_persistence_hook(move || {
                assert!(matches!(
                    activity_lock.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
            });

        let dispatches = runtime
            .owned
            .workflow_fail_provider_prompt(&session_id, &prompt, None, "provider unavailable")
            .expect("provider failure should settle workflow");
        assert!(
            !dispatches.is_empty(),
            "the failure transition must return the queued invocation's provider launch to its runtime caller: local={}, remote={}, starting_runs={}, starting_meta={}, admitted={}",
            dispatches.local.len(),
            dispatches.remote.len(),
            dispatches.starting_provider_runs.len(),
            dispatches.starting_metaagent_tasks.len(),
            dispatches.admitted_workflow_prompt,
        );
        assert_eq!(dispatches.starting_provider_runs.len(), 1);

        runtime
            .owned
            .durable_state_store
            .load_events_by_kind("workflow.runtime.updated")
            .expect("durable workflow events should load")
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
            .expect("workflow provider failure should persist a bounded transition");
        let durable_run = runtime
            .owned
            .durable_state_store
            .resolve_workflow_run(session.host_daemon_id(), &session_id, &workflow_run_id)
            .expect("durable workflow run should load")
            .expect("durable workflow run should exist");
        assert_eq!(
            durable_run.status(),
            crate::session::WorkflowRunStatus::Failed,
        );
    }
}
