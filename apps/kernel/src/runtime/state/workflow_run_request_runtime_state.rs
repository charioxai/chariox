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
                            *workflow_run = refreshed;
                        }
                        Ok(LocalDaemonResponse::WorkflowRunInvoked {
                            workflow_run: *workflow_run,
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
                        queued_prompt: *queued_prompt,
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

    pub(super) async fn execute_workflow_cancel_run_request(
        &self,
        request: crate::local::CancelWorkflowRunRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let session_id = request.session_id.clone();
        let result = self
            .execute_workflow_interrupt_run(&request.session_id, &request.workflow_run_ref, false)
            .await
            .map(
                |(workflow_run, session)| LocalDaemonResponse::WorkflowRunCancelled {
                    workflow_run,
                    session,
                },
            );
        let owned = &self.owned;
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or_else(|| owned.session_snapshot(&session_id).ok());
        (result, session)
    }

    pub(super) async fn execute_workflow_pause_run_request(
        &self,
        request: crate::local::PauseWorkflowRunRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let session_id = request.session_id.clone();
        let result = self
            .execute_workflow_interrupt_run(&request.session_id, &request.workflow_run_ref, true)
            .await
            .map(
                |(workflow_run, session)| LocalDaemonResponse::WorkflowRunPaused {
                    workflow_run,
                    session,
                },
            );
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or_else(|| self.owned.session_snapshot(&session_id).ok());
        (result, session)
    }

    pub(super) async fn execute_workflow_interrupt_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        pause: bool,
    ) -> Result<(crate::session::WorkflowRun, crate::session::RuntimeSession), DaemonError> {
        let owned = &self.owned;
        let resolved_workflow_run = owned
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?;
        let workflow_run_id = resolved_workflow_run.id().to_string();
        let mut provider_run_ids = owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter_map(|agent| {
                owned
                    .provider_store
                    .get_run_for_agent(session_id, agent.id())
                    .map(|run| run.id().to_string())
            })
            .collect::<Vec<_>>();
        provider_run_ids.sort();
        provider_run_ids.dedup();
        let mut _provider_run_permits = Vec::with_capacity(provider_run_ids.len());
        for provider_run_id in provider_run_ids {
            _provider_run_permits.push(self.provider_runtime_lanes.acquire(&provider_run_id).await);
        }
        let session_before_interrupt = owned.session_store.get_session(session_id)?;
        let _ = owned
            .prompt_state_owner
            .remove_queued_prompts_by_workflow_run(&session_before_interrupt, &workflow_run_id);
        let expected_status = if pause {
            crate::session::WorkflowRunStatus::Paused
        } else {
            crate::session::WorkflowRunStatus::Stopped
        };
        let workflow_run = if resolved_workflow_run.status() == expected_status {
            resolved_workflow_run
        } else if pause {
            owned
                .session_store
                .write()
                .pause_workflow_run(session_id, workflow_run_ref)?
        } else {
            owned
                .session_store
                .write()
                .cancel_workflow_run(session_id, workflow_run_ref)?
        };
        let _ = owned.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id && claim.operation == "workflow_node_dispatch"
        });
        let session = owned.session_store.get_session(session_id)?;
        for agent in owned.agent_store.get_session_agents(session_id) {
            let (active_prompt, queued_prompts) =
                owned.prompt_state_owner.state_parts(&session, agent.id());
            let _ = owned.mirror_prompt_owner_agent_state(
                session_id,
                agent.id(),
                active_prompt,
                queued_prompts,
            );
        }
        let mut workflow_dispatches = owned.workflow_maybe_start_next_queued_prompt(session_id);
        workflow_dispatches.extend(owned.workflow_retry_blocked_claims());
        self.spawn_workflow_prompt_dispatches(workflow_dispatches);
        let session = owned.session_store.get_session(session_id)?;
        let active_agents = owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter_map(|agent| {
                owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent.id())
                    .filter(|prompt| prompt.workflow_run_id() == Some(workflow_run_id.as_str()))
                    .map(|prompt| {
                        (
                            agent.id().to_string(),
                            prompt.source_attachment_id().to_string(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        for (agent_id, attachment_id) in active_agents {
            let cancellation = match self
                .cancel_agent_prompt(session_id, &agent_id, &attachment_id)
                .await
            {
                Ok(cancellation) => cancellation,
                Err(DaemonError::NoActivePrompt { .. }) => continue,
                Err(error) => return Err(error),
            };
            if let Some(dispatch) = cancellation.dispatch {
                self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
            }
        }
        Ok((workflow_run, owned.session_snapshot(session_id)?))
    }

    pub(super) async fn execute_workflow_resume_run_request(
        &self,
        request: crate::local::ResumeWorkflowRunRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;
        let session_id = request.session_id.clone();
        if let Err(error) = self
            .wait_for_workflow_prompt_cancellation_settlement(
                &request.session_id,
                &request.workflow_run_ref,
            )
            .await
        {
            return (Err(error), owned.session_snapshot(&session_id).ok());
        }
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

    async fn wait_for_workflow_prompt_cancellation_settlement(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(), DaemonError> {
        let workflow_run_id = self
            .owned
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?
            .id()
            .to_string();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            self.owned.reap_structured_prompt_jobs();
            let session = self.owned.session_store.get_session(session_id)?;
            let cancelling = self
                .owned
                .agent_store
                .get_session_agents(session_id)
                .into_iter()
                .filter_map(|agent| {
                    self.owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent.id())
                })
                .any(|prompt| {
                    prompt.workflow_run_id() == Some(workflow_run_id.as_str())
                        && prompt.status() == crate::session::PromptStatus::Cancelling
                });
            if !cancelling {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(DaemonError::LocalTransport {
                    operation: "resume workflow run",
                    message: format!(
                        "workflow run `{workflow_run_id}` is still settling its paused provider turn"
                    ),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}
