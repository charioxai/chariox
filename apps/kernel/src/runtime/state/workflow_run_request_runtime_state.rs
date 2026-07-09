//! Runtime request handlers for workflow run invoke, cancel, and resume operations.

use super::workflow_request_runtime_state::workflow_response_session;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_invoke_endpoint_request(
        &self,
        request: crate::local::InvokeWorkflowEndpointRequest,
        caller_user_id: &str,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;
        let session_id = request.session_id.clone();
        let result = match owned.ensure_workflow_endpoint_owner(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            caller_user_id,
            "invoke workflow endpoint",
        ) {
            Ok(()) => {
                let (outcome, dispatches) = match owned.workflow_enqueue_prompt_and_maybe_start(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt.clone(),
                    request.queue_ref.as_deref(),
                    request.publication_invocation.clone(),
                ) {
                    Ok(outcome) => outcome,
                    Err(error) => return (Err(error), owned.session_snapshot(&session_id).ok()),
                };
                let dev_stub_workflow_run_id = match &outcome {
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        ..
                    } if self.workflow_dispatches_start_only_dev_stub_providers(&dispatches) => {
                        Some(workflow_run.id().to_string())
                    }
                    _ => None,
                };
                self.spawn_workflow_prompt_dispatches(dispatches);
                let refreshed_workflow_run = match dev_stub_workflow_run_id.as_deref() {
                    Some(workflow_run_id) => {
                        self.wait_for_dev_stub_workflow_run_start(
                            &request.session_id,
                            workflow_run_id,
                        )
                        .await
                    }
                    None => None,
                };
                let session = match owned.session_snapshot(&request.session_id) {
                    Ok(session) => session,
                    Err(error) => return (Err(error), None),
                };
                match outcome {
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        mut workflow_run,
                        workflow,
                        endpoint,
                    } => {
                        if let Some(refreshed) = refreshed_workflow_run {
                            workflow_run = refreshed;
                        }
                        Ok(LocalDaemonResponse::WorkflowRunInvoked {
                            workflow_run,
                            workflow,
                            endpoint,
                            session,
                        })
                    }
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                        queued_prompt,
                        workflow,
                        endpoint,
                    } => Ok(LocalDaemonResponse::WorkflowPromptEnqueued {
                        queued_prompt,
                        workflow,
                        endpoint,
                        session,
                    }),
                }
            }
            Err(error) => Err(error),
        };
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or_else(|| owned.session_snapshot(&session_id).ok());
        (result, session)
    }

    fn workflow_dispatches_start_only_dev_stub_providers(
        &self,
        dispatches: &WorkflowPromptDispatches,
    ) -> bool {
        !dispatches.starting_provider_runs.is_empty()
            && dispatches.starting_provider_runs.iter().all(|run_id| {
                self.owned
                    .provider_store
                    .get_run(run_id)
                    .is_ok_and(|run| run.adapter_key() == "dev-stub")
            })
    }

    async fn wait_for_dev_stub_workflow_run_start(
        &self,
        session_id: &str,
        workflow_run_id: &str,
    ) -> Option<crate::session::WorkflowRun> {
        for _ in 0..50 {
            if let Ok(workflow_run) = self
                .owned
                .session_store
                .read()
                .resolve_workflow_run_ref(session_id, workflow_run_id)
            {
                if workflow_run.status() == crate::session::WorkflowRunStatus::Running {
                    return Some(workflow_run);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        self.owned
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()
    }

    pub(super) fn execute_workflow_cancel_run_request(
        &self,
        request: crate::local::CancelWorkflowRunRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;
        let session_id = request.session_id.clone();
        let result = (|| {
            let workflow_run_id = owned
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                .id()
                .to_string();
            let session = owned.session_store.get_session(&request.session_id)?;
            for agent in owned.agent_store.get_session_agents(&request.session_id) {
                if owned
                    .prompt_state_owner
                    .active_prompt_for_agent_or_restore(&session, agent.id())
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
                    .as_deref()
                    == Some(workflow_run_id.as_str())
                {
                    let _ = owned
                        .prompt_state_owner
                        .begin_cancelling_active_prompt(&session, agent.id())
                        .ok_or_else(|| DaemonError::NoActivePrompt {
                            session_id: request.session_id.clone(),
                        })?;
                    let (active_prompt, queued_prompts) =
                        owned.prompt_state_owner.state_parts(&session, agent.id());
                    owned.mirror_prompt_owner_agent_state(
                        &request.session_id,
                        agent.id(),
                        active_prompt,
                        queued_prompts,
                    )?;
                }
            }
            let workflow_run = owned
                .session_store
                .write()
                .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
            let _ = owned.prompt_workspace_claims.remove_matching(|claim| {
                claim.session_id == request.session_id
                    && claim.operation == "workflow_node_dispatch"
            });
            let workflow = owned
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, workflow_run.workflow_id())?;
            for node in workflow.nodes() {
                if let Some(run) = owned
                    .provider_store
                    .get_run_for_agent(&request.session_id, node.agent_id())
                {
                    let _ = owned.clear_prompt_activity(run.id());
                }
            }
            let session = owned.session_store.get_session(&request.session_id)?;
            let _ = owned
                .prompt_state_owner
                .remove_queued_prompts_by_workflow_run(&session, &workflow_run_id);
            for agent in owned.agent_store.get_session_agents(&request.session_id) {
                let (active_prompt, queued_prompts) =
                    owned.prompt_state_owner.state_parts(&session, agent.id());
                let _ = owned.mirror_prompt_owner_agent_state(
                    &request.session_id,
                    agent.id(),
                    active_prompt,
                    queued_prompts,
                );
            }
            owned.workflow_maybe_start_next_queued_prompt(&request.session_id);
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
            let session = owned.session_snapshot(&request.session_id)?;
            Ok(LocalDaemonResponse::WorkflowRunCancelled {
                workflow_run,
                session,
            })
        })();
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or_else(|| owned.session_snapshot(&session_id).ok());
        (result, session)
    }

    pub(super) fn execute_workflow_resume_run_request(
        &self,
        request: crate::local::ResumeWorkflowRunRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;
        let session_id = request.session_id.clone();
        let result = match owned.workflow_resume_run(&request.session_id, &request.workflow_run_ref)
        {
            Ok((workflow_run, dispatches)) => {
                self.spawn_workflow_prompt_dispatches(dispatches);
                owned.workflow_session(&request.session_id).map(|session| {
                    LocalDaemonResponse::WorkflowRunResumed {
                        workflow_run,
                        session,
                    }
                })
            }
            Err(error) => Err(error),
        };
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or_else(|| owned.session_snapshot(&session_id).ok());
        (result, session)
    }
}
