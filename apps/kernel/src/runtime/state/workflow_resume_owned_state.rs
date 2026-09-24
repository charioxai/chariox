//! Workflow run resume transitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        let activity_mutation = self.begin_managed_activity_mutation();
        let durable_state_store = self.durable_state_store.clone();
        let (workflow_run, resumable_node_ids, pending_entry_node) = durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let mut sessions = self.session_store.write();
                let session_before_resume = sessions.get_session(session_id)?;
                let original = sessions.resolve_workflow_run_ref(session_id, workflow_run_ref)?;
                // Classify an unsubmitted original entry before publishing the run
                // as runnable to the owned pump. A concurrent pump admission
                // afterward must suppress this retry, not turn it into an
                // intentional second user invocation.
                let pending_entry_node = self
                    .workflow_entry_intent(session_id, original.id())?
                    .filter(|intent| !intent.submitted)
                    .map(|intent| intent.node_id);
                let resumable_node_ids = original
                    .node_runs()
                    .iter()
                    .filter(|node_run| {
                        node_run.status() == crate::session::WorkflowNodeRunStatus::Stopped
                            && node_run.completion().is_none()
                            && node_run
                                .turn_envelope()
                                .and_then(|envelope| envelope.rendered_prompt())
                                .is_some()
                    })
                    .map(|node_run| node_run.id().to_string())
                    .collect::<std::collections::BTreeSet<_>>();
                let workflow_run = sessions.resume_workflow_run(session_id, workflow_run_ref)?;
                let durable_session = sessions.get_session(session_id)?;
                if let Err(error) = durable_state_store
                    .persist_workflow_runtime_transition(&durable_session, "workflow_run_resumed")
                {
                    // Restore while the write guard still excludes unrelated session mutations.
                    sessions.restore_session(session_before_resume);
                    return Err(error);
                }
                Ok((workflow_run, resumable_node_ids, pending_entry_node))
            })?;
        activity_mutation.record();
        // Prompt admission and provider dispatch must observe only a durably resumed run.
        self.session_snapshot(session_id)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter(|node_run| resumable_node_ids.contains(node_run.id()))
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            if pending_entry_node.as_deref() == Some(workflow_node_run_id.as_str()) {
                if let Some(prepared) = self.workflow_retry_durable_entry(
                    session_id,
                    workflow_run.id(),
                    &workflow_node_run_id,
                )? {
                    dispatches.extend(prepared);
                    continue;
                }
            }
            // A deliberate user resume of previously submitted work is a new
            // invocation; only an unsubmitted original entry reuses its intent.
            let prompt = crate::session::PromptQueueItem::new(
                format!(
                    "pending-draft:workflow-resume:{}:{}",
                    workflow_run.id(),
                    workflow_node_run_id
                ),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn resume_append_failure_rolls_back_before_snapshot_and_retries_once() {
        let (runtime, session_id, workflow_id, workflow_run_id, node_run_id, agent_id, _worktree) =
            runtime_with_paused_workflow();
        let baseline_provider_runs = runtime.owned.provider_store.list_runs().len();
        let baseline_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("paused session should remain available");
        let baseline_prompt_state = runtime
            .owned
            .prompt_state_owner
            .state_parts(&baseline_session, &agent_id);
        let claim_id =
            runtime
                .owned
                .workflow_dispatch_claim_id(&session_id, &workflow_run_id, &node_run_id);
        assert!(!runtime.owned.prompt_workspace_claims.contains(&claim_id));

        let state_path = runtime.owned.durable_state_store.path().to_path_buf();
        let connection = rusqlite::Connection::open(state_path)
            .expect("durable database should open for resume failure injection");
        connection
            .execute_batch(
                r#"CREATE TRIGGER fail_workflow_resume_append
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'workflow.runtime.updated'
                   AND instr(NEW.payload_json, '"reason":"workflow_run_resumed"') > 0
                 BEGIN
                   SELECT RAISE(FAIL, 'injected workflow resume append failure');
                 END;"#,
            )
            .expect("resume append failure trigger should install");
        let request = crate::local::ResumeWorkflowRunRequest {
            session_id: session_id.clone(),
            workflow_run_ref: workflow_run_id.clone(),
        };

        let (failed, projected) = runtime
            .execute_workflow_resume_run_request(request.clone())
            .await;
        assert!(failed
            .expect_err("failed durable append should reject workflow resume")
            .to_string()
            .contains("injected workflow resume append failure"));
        assert!(
            projected.is_none(),
            "failed resume must not publish a fallback session snapshot"
        );
        let rolled_back = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("rolled-back session should remain available");
        assert_paused_run(&rolled_back, &workflow_run_id, &node_run_id);
        assert_eq!(
            runtime.owned.provider_store.list_runs().len(),
            baseline_provider_runs,
            "failed resume must not create or launch a provider run"
        );
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .state_parts(&rolled_back, &agent_id),
            baseline_prompt_state,
            "failed resume must not admit a provider prompt"
        );
        assert!(
            !runtime.owned.prompt_workspace_claims.contains(&claim_id),
            "failed resume must not acquire a workflow workspace claim"
        );

        // A later unrelated projection must still expose the durable paused state.
        let unrelated_snapshot = runtime
            .owned
            .session_snapshot(&session_id)
            .expect("unrelated session snapshot should remain available");
        assert_paused_run(&unrelated_snapshot, &workflow_run_id, &node_run_id);
        let owner_id = unrelated_snapshot.host_daemon_id().to_string();
        let durable_before_retry = runtime
            .owned
            .durable_state_store
            .list_workflow_runs_page(&owner_id, &session_id, Some(&workflow_id), None, 10)
            .expect("durable paused workflow run should load");
        assert_eq!(durable_before_retry.workflow_runs.len(), 1);
        assert_paused_workflow_run(&durable_before_retry.workflow_runs[0], &node_run_id);

        connection
            .execute_batch("DROP TRIGGER fail_workflow_resume_append;")
            .expect("resume append failure trigger should be removed");
        let (retried, projected) = runtime.execute_workflow_resume_run_request(request).await;
        let resumed = match retried.expect("workflow resume retry should succeed") {
            crate::local::LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => {
                workflow_run
            }
            other => panic!("unexpected workflow resume response: {other:?}"),
        };
        assert_ne!(resumed.status(), crate::session::WorkflowRunStatus::Paused);
        assert_ne!(
            resumed.node_runs()[0].status(),
            crate::session::WorkflowNodeRunStatus::Stopped
        );
        let projected = projected.expect("successful resume should publish a session snapshot");
        assert_eq!(projected.workflow_runs().len(), 1);
        assert_ne!(
            projected.workflow_runs()[0].status(),
            crate::session::WorkflowRunStatus::Paused
        );
        let resumed_prompt_state = runtime
            .owned
            .prompt_state_owner
            .state_parts(&projected, &agent_id);
        assert_eq!(
            (if resumed_prompt_state.0.is_some() {
                1
            } else {
                0
            }) + resumed_prompt_state.1.len(),
            1,
            "successful retry should admit exactly one resumed prompt"
        );
        let durable_after_retry = runtime
            .owned
            .durable_state_store
            .list_workflow_runs_page(&owner_id, &session_id, Some(&workflow_id), None, 10)
            .expect("durable resumed workflow run should load");
        assert_eq!(durable_after_retry.workflow_runs.len(), 1);
        assert_ne!(
            durable_after_retry.workflow_runs[0].status(),
            crate::session::WorkflowRunStatus::Paused
        );
    }

    fn assert_paused_run(
        session: &crate::session::RuntimeSession,
        workflow_run_id: &str,
        node_run_id: &str,
    ) {
        let workflow_run = session
            .workflow_runs()
            .iter()
            .find(|candidate| candidate.id() == workflow_run_id)
            .expect("paused workflow run should remain available");
        assert_paused_workflow_run(workflow_run, node_run_id);
    }

    fn assert_paused_workflow_run(workflow_run: &crate::session::WorkflowRun, node_run_id: &str) {
        assert_eq!(
            workflow_run.status(),
            crate::session::WorkflowRunStatus::Paused
        );
        let node_run = workflow_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == node_run_id)
            .expect("paused workflow node run should remain available");
        assert_eq!(
            node_run.status(),
            crate::session::WorkflowNodeRunStatus::Stopped
        );
    }

    fn runtime_with_paused_workflow() -> (
        KernelRuntimeState,
        String,
        String,
        String,
        String,
        String,
        crate::test_support::TestWorktree,
    ) {
        let worktree = crate::test_support::TestWorktree::new("workflow-resume-durability");
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .expect("session should be created");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("resume-durability-agent"),
            )
            .expect("workflow agent should be created");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("resume-durability".to_string()))
            .expect("workflow should be created");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should be created");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");
        let workflow_run = app
            .sessions_mut()
            .invoke_workflow_endpoint(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("resume exactly once".to_string()),
            )
            .expect("workflow run should be created");
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        app.sessions_mut()
            .prepare_workflow_turn(
                session.id(),
                workflow_run.id(),
                &node_run_id,
                format!("workflow-ack:{node_run_id}"),
                "resume exactly once".to_string(),
                None,
                None,
            )
            .expect("workflow turn should be prepared");
        app.sessions_mut()
            .pause_workflow_run(session.id(), workflow_run.id())
            .expect("workflow run should pause");

        let session_id = session.id().to_string();
        let workflow_id = workflow.id().to_string();
        let workflow_run_id = workflow_run.id().to_string();
        let agent_id = agent.id().to_string();
        let runtime = runtime_state_from_app(app);
        runtime
            .owned
            .persist_workflow_runtime_session(&session_id, "resume_failure_test_baseline")
            .expect("paused workflow baseline should persist");
        (
            runtime,
            session_id,
            workflow_id,
            workflow_run_id,
            node_run_id,
            agent_id,
            worktree,
        )
    }

    fn runtime_state_from_app(app: DaemonApp) -> KernelRuntimeState {
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
}
