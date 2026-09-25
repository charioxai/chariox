//! Async direct-prompt dispatch entry points.
//!
//! This layer validates runtime state, starts or queues prompts, and hands provider-specific
//! submission work to the provider runtime without exposing owned-state internals to transports.

use super::*;

struct RemoteQueuedPromptSteerReservation {
    prompt: crate::session::PromptQueueItem,
    id: u64,
}

impl RemoteQueuedPromptSteerReservation {
    fn new(prompt: crate::session::PromptQueueItem, id: u64) -> Self {
        Self { prompt, id }
    }

    fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for RemoteQueuedPromptSteerReservation {
    fn drop(&mut self) {
        let _ = self.prompt.release_remote_steer(self.id);
    }
}

impl KernelRuntimeState {
    async fn release_failed_remote_steer_and_advance_if_idle(
        &self,
        session_id: &str,
        agent_id: &str,
        reservation: RemoteQueuedPromptSteerReservation,
    ) {
        drop(reservation);
        let owned = &self.owned;
        let admitted = self
            .with_app_side_effect(|app| {
                let session = owned.session_store.get_session(session_id)?;
                if owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
                    .is_some()
                {
                    return Ok(None);
                }
                crate::app::KernelAgentService::new(app)
                    .admit_next_queued_remote_prompt(session_id, agent_id, None)
            })
            .await;
        match admitted {
            Ok(Some((_, intent))) => self.spawn_remote_prompt_dispatch(intent.dispatch),
            Ok(None) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "failed to advance queued prompt after rejected remote steer",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    async fn refresh_remote_agent_binding_for_steer(
        &self,
        agent_id: &str,
        expected_binding: &crate::agent::RemoteAgentBinding,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let expected_binding = expected_binding.clone();
        let agent_id = agent_id.to_string();
        let plan = self
            .with_app_side_effect(move |app| {
                app.prepare_remote_agent_binding_refresh(&agent_id, &expected_binding)
            })
            .await?;
        let refresh = crate::app::DaemonApp::execute_remote_agent_binding_refresh(plan).await?;
        let (committed, binding_committed) = self
            .with_app_side_effect(|app| app.commit_remote_agent_binding_refresh(&refresh))
            .await;
        match committed {
            Ok(agent) => Ok(agent),
            Err(error) => {
                if !binding_committed {
                    crate::app::DaemonApp::cleanup_remote_agent_binding_refresh(&refresh).await;
                }
                Err(error)
            }
        }
    }

    pub(in crate::runtime::state) async fn steer_remote_agent_message(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<String>, DaemonError> {
        let owned = &self.owned;
        let agent_id = prompt.target_agent_id();
        let agent = owned.agent_store.get_agent(agent_id)?;
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(None);
        };
        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if active_prompt.status() != crate::session::PromptStatus::Running {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "target agent is stopping; message was not delivered".to_string(),
            });
        }
        if active_prompt.is_external() {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "agent messages cannot steer an externally started provider turn"
                    .to_string(),
            });
        }
        if let Some(projected) = owned
            .provider_run_projection
            .get_for_agent(session_id, agent_id)
        {
            if projected.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: projected.id().to_string(),
                    state: projected.state(),
                    operation: "steer agent message",
                });
            }
        }
        let worker_provider_run_id = remote_execution
            .active_worker_provider_run_id
            .clone()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let (remote_prompt, required_skills) = self
            .prepare_remote_prompt_skill_context(&agent, prompt.prompt())
            .await?;
        let prompt_attachments = prompt.attachments().to_vec();
        let attachments = tokio::task::spawn_blocking(move || {
            crate::app::serialize_remote_prompt_attachments(&prompt_attachments)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "serialize remote agent message attachments",
            message: error.to_string(),
        })??;
        let payload = RemoteQueuedPromptSteerPayload {
            steer_id: prompt.id().to_string(),
            target_home_prompt_id: active_prompt.id().to_string(),
            prompt: remote_prompt,
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments,
            required_skills,
        };
        let (mut remote_execution, mut relay_config) = self
            .with_app_side_effect(|app| {
            let current_agent = owned.agent_store.get_agent(agent_id)?;
            let remote_execution = current_agent
                .remote_execution()
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "steer agent message",
                    message: format!("agent `{agent_id}` is no longer remote"),
                })?;
            let current_session = owned.session_store.get_session(session_id)?;
            let current_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(&current_session, agent_id)
                .ok_or_else(|| DaemonError::NoActivePrompt {
                    session_id: session_id.to_string(),
                })?;
            if current_active.id() != payload.target_home_prompt_id
                || current_active.status() != crate::session::PromptStatus::Running
                || remote_execution.active_worker_provider_run_id.as_deref()
                    != Some(worker_provider_run_id.as_str())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "steer agent message",
                    message: "active remote provider turn changed before delivery".to_string(),
                });
            }
            Ok((
                remote_execution.clone(),
                app.relay_config_for_remote_execution(&remote_execution),
            ))
        })
        .await?;
        let mut response = send_remote_queued_prompt_steer(
            &relay_config,
            &remote_execution,
            &payload,
        )
        .await;
        if response.as_ref().is_err_and(
            super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding,
        ) {
            let refreshed = self
                .refresh_remote_agent_binding_for_steer(agent_id, &remote_execution)
                .await?;
            remote_execution = refreshed
                .remote_execution()
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "refresh remote agent message binding",
                    message: format!("agent `{agent_id}` did not retain remote execution"),
                })?;
            let prepared = self
                .with_app_side_effect(|app| {
                    let current_agent = owned.agent_store.get_agent(agent_id)?;
                    let current_execution = current_agent
                        .remote_execution()
                        .cloned()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "retry remote agent message",
                            message: format!("agent `{agent_id}` is no longer remote"),
                        })?;
                    let current_session = owned.session_store.get_session(session_id)?;
                    let current_active = owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&current_session, agent_id)
                        .ok_or_else(|| DaemonError::NoActivePrompt {
                            session_id: session_id.to_string(),
                        })?;
                    if current_execution != remote_execution
                        || current_active.id() != payload.target_home_prompt_id
                        || current_active.status() != crate::session::PromptStatus::Running
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "retry remote agent message",
                            message: "active remote provider turn or binding changed before retry".to_string(),
                        });
                    }
                    Ok(app.relay_config_for_remote_execution(&current_execution))
                })
                .await?;
            relay_config = prepared;
            response = send_remote_queued_prompt_steer(
                &relay_config,
                &remote_execution,
                &payload,
            )
            .await;
        }
        let provider_run_id = match response? {
            RelayPeerResponse::LeasedPromptSteered {
                provider_run_id,
                steer_id,
                ..
            } if steer_id == payload.steer_id => provider_run_id,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "steer agent message",
                    message: format!("unexpected remote message response: {other:?}"),
                });
            }
        };
        let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
            &remote_execution.leased_agent_id,
            &provider_run_id,
        );
        self.with_app_side_effect(|_app| {
            let current_agent = owned.agent_store.get_agent(agent_id)?;
            if !remote_steer_binding_identity_matches(
                current_agent.remote_execution(),
                Some(&remote_execution),
            ) {
                return Err(DaemonError::LocalTransport {
                    operation: "commit remote agent message steer",
                    message: "remote binding changed while the agent message was in flight".to_string(),
                });
            }
            let current_session = owned.session_store.get_session(session_id)?;
            let current_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(&current_session, agent_id);
            if current_active.as_ref().is_none_or(|active| {
                active.id() != payload.target_home_prompt_id
                    || active.status() != crate::session::PromptStatus::Running
            }) || current_agent
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
                != Some(worker_provider_run_id.as_str())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "commit remote agent message steer",
                    message: "active remote provider turn changed while the agent message was in flight"
                        .to_string(),
                });
            }
            owned.append_steering_prompt_history(
                session_id,
                &provider_run_id,
                &payload.target_home_prompt_id,
                prompt.source_attachment_id(),
                agent_id,
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
            )?;
            owned.echo_steering_prompt_to_other_attachments(
                session_id,
                &projected_provider_run_id,
                agent_id,
                prompt.id(),
                prompt.source_attachment_id(),
                prompt.source_attachment_id(),
                prompt.prompt(),
                prompt.attachments(),
                prompt.prompt_origin(),
            );
            Ok(Some(projected_provider_run_id))
        })
        .await
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        self.submit_prepared_prompt_with_queue_policy(prepared, true)
            .await
    }

    pub(crate) async fn submit_prepared_prompt_with_queue_policy(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
        allow_queue: bool,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        self.owned.require_publication_activation()?;
        {
            let owned = &self.owned;
            let session = owned.session_store.get_session(&prepared.session_id)?;
            if let Some(outcome) = owned
                .prompt_state_owner
                .replay_durable_submission(&session, &prepared.prompt)?
            {
                return Ok(crate::app::KernelPromptSubmission {
                    outcome,
                    session: owned.session_snapshot(&prepared.session_id)?,
                    dispatch: None,
                    remote_dispatch: None,
                });
            }
            if let Some(mut submission) =
                owned.submit_local_prepared_prompt_with_queue_policy(&prepared, allow_queue)?
            {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            if let Some(mut submission) =
                owned.submit_remote_prepared_prompt_with_queue_policy(&prepared, allow_queue)?
            {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                self.spawn_remote_prompt_projection_drain_if_needed(&submission);
                return Ok(submission);
            }
            let session_id = prepared.session_id.clone();
            let target_agent_id = prepared.prompt.target_agent_id().to_string();
            let attachment_id = prepared.prompt.source_attachment_id().to_string();
            let has_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &owned.session_store.get_session(&session_id)?,
                    &target_agent_id,
                )
                .is_some();
            let has_run = owned
                .provider_store
                .get_run_for_agent(&session_id, &target_agent_id)
                .is_some();
            if !has_active && !has_run {
                let is_remote_agent = owned
                    .agent_store
                    .get_agent(&target_agent_id)?
                    .remote_execution()
                    .is_some();
                if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                    let (event_reply_enabled, event_context_enabled, event_actions_enabled) = owned
                        .workflow_event_capabilities_for_prompt(&session_id, &prepared.prompt)?;
                    let fresh_context = owned.workflow_prompt_requires_fresh_provider_context(
                        &session_id,
                        &target_agent_id,
                        &prepared.prompt,
                    )?;
                    let (_provider_run_id, _) = owned.workflow_ensure_provider_run(
                        &session_id,
                        &target_agent_id,
                        event_reply_enabled,
                        event_context_enabled,
                        event_actions_enabled,
                        fresh_context,
                        prepared.prompt.workflow_node_run_id(),
                    )?;
                } else if is_remote_agent {
                    if let Some(mut submission) = owned
                        .submit_remote_prepared_prompt_with_queue_policy(&prepared, allow_queue)?
                    {
                        self.finish_owned_prompt_submission_workflow_start(&mut submission)
                            .await?;
                        self.spawn_remote_prompt_projection_drain_if_needed(&submission);
                        return Ok(submission);
                    }
                } else {
                    self.with_app_side_effect(|app| {
                        app.ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)
                    })
                    .await?;
                };
                if let Some(mut submission) =
                    owned.submit_local_prepared_prompt_with_queue_policy(&prepared, allow_queue)?
                {
                    self.finish_owned_prompt_submission_workflow_start(&mut submission)
                        .await?;
                    return Ok(submission);
                }
            }
            Err(DaemonError::LocalTransport {
                operation: "submit prepared prompt",
                message:
                    "owned prompt runtime could not admit prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(super) async fn finish_owned_prompt_submission_workflow_start(
        &self,
        submission: &mut crate::app::KernelPromptSubmission,
    ) -> Result<(), DaemonError> {
        let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome
        else {
            return Ok(());
        };
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(prompt.source_attachment_id())
        {
            return Ok(());
        }
        let session_id = submission.session.id().to_string();
        let prompt = prompt.clone();
        if let Some(remote_dispatch) = submission.remote_dispatch.as_mut() {
            remote_dispatch.workflow_context = Some(
                self.with_app_side_effect(|app| {
                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                        .remote_workflow_turn_context_for_prompt(
                            &session_id,
                            prompt.target_agent_id(),
                            &prompt,
                        )
                })
                .await?,
            );
        }
        self.owned.workflow_start_prompt(&session_id, &prompt)
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(cancellation) = self
                .cancel_remote_agent_prompt_if_remote(session_id, target_agent_id, attachment_id)
                .await?
            {
                return Ok(cancellation);
            }
            if let Some(cancellation) =
                owned.cancel_local_prompt(session_id, target_agent_id, attachment_id)?
            {
                return Ok(cancellation);
            }
            Err(DaemonError::LocalTransport {
                operation: "cancel prompt",
                message:
                    "owned prompt runtime could not cancel prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(crate) async fn steer_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptSteer, DaemonError> {
        let owned = &self.owned;
        if owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .is_none()
        {
            if let Some(steer) =
                owned.steer_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)?
            {
                return Ok(steer);
            }
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "local queued prompt steer did not produce a dispatch".to_string(),
            });
        }
        let prepared = owned
            .prepare_remote_queued_prompt_steer(
                session_id,
                target_agent_id,
                attachment_id,
                prompt_id,
            )?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer remote queued prompt",
                message: format!("agent `{target_agent_id}` is no longer remote"),
            })?;
        let (remote_prompt, required_skills) = self
            .prepare_remote_prompt_skill_context(&prepared.agent, prepared.prompt.prompt())
            .await?;
        let prompt_attachments = prepared.prompt.attachments().to_vec();
        let attachments = tokio::task::spawn_blocking(move || {
            crate::app::serialize_remote_prompt_attachments(&prompt_attachments)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "serialize remote steering prompt attachments",
            message: error.to_string(),
        })??;
        let payload = RemoteQueuedPromptSteerPayload {
            steer_id: prepared.prompt.id().to_string(),
            target_home_prompt_id: prepared.target_active_prompt_id.clone(),
            prompt: remote_prompt,
            hidden_system_context: prepared.prompt.hidden_system_context().to_string(),
            attachments,
            required_skills,
        };
        let session_id = session_id.to_string();
        let target_agent_id = target_agent_id.to_string();
        let attachment_id = attachment_id.to_string();
        let prompt_id = prompt_id.to_string();
        let (mut remote_execution, mut relay_config, reservation_guard) = self
            .with_app_side_effect(|app| {
                let current = owned
                    .prepare_remote_queued_prompt_steer(
                        &session_id,
                        &target_agent_id,
                        &attachment_id,
                        &prompt_id,
                    )?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "steer remote queued prompt",
                        message: format!("agent `{target_agent_id}` is no longer remote"),
                    })?;
                if current.target_active_prompt_id != payload.target_home_prompt_id
                    || current.remote_execution != prepared.remote_execution
                    || current.prompt != prepared.prompt
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "steer remote queued prompt",
                        message: "queued prompt, active turn, or remote binding changed before delivery".to_string(),
                    });
                }
                let reservation = owned.reserve_remote_queued_prompt_steer(
                    &session_id,
                    &target_agent_id,
                    &attachment_id,
                    &prompt_id,
                    &prepared,
                )?;
                let remote_execution = current.remote_execution;
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                Ok((
                    remote_execution,
                    relay_config,
                    RemoteQueuedPromptSteerReservation::new(current.prompt, reservation),
                ))
            })
            .await?;
        let reservation_id = reservation_guard.id();

        let mut response = send_remote_queued_prompt_steer(
            &relay_config,
            &remote_execution,
            &payload,
        )
        .await;
        let mut advance_after_error = response
            .as_ref()
            .is_err_and(remote_queued_steer_failure_is_definitely_unaccepted);
        if response.as_ref().is_err_and(
            super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding,
        ) {
            // The stale-binding response is an explicit peer rejection. If the
            // refresh or retry preparation fails after concurrent completion,
            // release this queue head and let ordinary admission continue.
            advance_after_error = true;
            let refresh = self
                .refresh_remote_agent_binding_for_steer(&target_agent_id, &remote_execution)
                .await;
            let refreshed = match refresh {
                Ok(agent) => agent,
                Err(error) => {
                    self.release_failed_remote_steer_and_advance_if_idle(
                        &session_id,
                        &target_agent_id,
                        reservation_guard,
                    )
                    .await;
                    return Err(error);
                }
            };
            remote_execution = match refreshed.remote_execution().cloned() {
                Some(remote_execution) => remote_execution,
                None => {
                    self.release_failed_remote_steer_and_advance_if_idle(
                        &session_id,
                        &target_agent_id,
                        reservation_guard,
                    )
                    .await;
                    return Err(DaemonError::LocalTransport {
                        operation: "refresh remote queued prompt steer binding",
                        message: format!(
                            "agent `{target_agent_id}` did not have remote execution after binding refresh"
                        ),
                    });
                }
            };
            let prepared_retry = self
                .with_app_side_effect(|app| {
                    let current = owned
                        .prepare_remote_queued_prompt_steer(
                            &session_id,
                            &target_agent_id,
                            &attachment_id,
                            &prompt_id,
                        )?
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "retry remote queued prompt steer",
                            message: format!("agent `{target_agent_id}` is no longer remote"),
                        })?;
                    if current.remote_execution != remote_execution
                        || current.target_active_prompt_id != payload.target_home_prompt_id
                        || !current.prompt.remote_steer_reservation_matches(reservation_id)
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "retry remote queued prompt steer",
                            message: "queued prompt, active turn, or remote binding changed before retry".to_string(),
                        });
                    }
                    Ok(app.relay_config_for_remote_execution(&remote_execution))
                })
                .await;
            relay_config = match prepared_retry {
                Ok(relay_config) => relay_config,
                Err(error) => {
                    self.release_failed_remote_steer_and_advance_if_idle(
                        &session_id,
                        &target_agent_id,
                        reservation_guard,
                    )
                    .await;
                    return Err(error);
                }
            };
            response = send_remote_queued_prompt_steer(
                &relay_config,
                &remote_execution,
                &payload,
            )
            .await;
            // A retry timeout or disconnect is ambiguous: the worker might
            // have accepted the steer, so only an explicit peer rejection or
            // a classified pre-send failure permits ordinary promotion.
            advance_after_error = response
                .as_ref()
                .is_err_and(remote_queued_steer_failure_is_definitely_unaccepted);
        }
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if advance_after_error {
                    self.release_failed_remote_steer_and_advance_if_idle(
                        &session_id,
                        &target_agent_id,
                        reservation_guard,
                    )
                    .await;
                }
                return Err(error);
            }
        };
        let provider_run_id = match response {
            RelayPeerResponse::LeasedPromptSteered {
                provider_run_id,
                steer_id,
                ..
            } if steer_id == payload.steer_id => provider_run_id,
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "steer remote queued prompt",
                    message: format!("unexpected remote prompt steer response: {other:?}"),
                });
            }
        };
        let committed = self
            .with_app_side_effect(|app| {
                let steer = owned.finish_remote_queued_prompt_steer(
                    &session_id,
                    &target_agent_id,
                    &attachment_id,
                    &prepared,
                    &remote_execution,
                    reservation_id,
                    &provider_run_id,
                )?;
                let has_active = owned
                    .prompt_state_owner
                    .active_prompt_for_agent(
                        &owned.session_store.get_session(&session_id)?,
                        &target_agent_id,
                    )
                    .is_some();
                let next_dispatch = if has_active {
                    None
                } else {
                    crate::app::KernelAgentService::new(app)
                        .admit_next_queued_remote_prompt(&session_id, &target_agent_id, None)?
                        .map(|(_, intent)| intent.dispatch)
                };
                Ok((steer, next_dispatch))
            })
            .await;
        match committed {
            Ok((steer, Some(dispatch))) => {
                self.spawn_remote_prompt_dispatch(dispatch);
                Ok(steer)
            }
            Ok((steer, None)) => Ok(steer),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn cancel_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        {
            self.owned
                .cancel_queued_prompt(session_id, target_agent_id, attachment_id, prompt_id)
        }
    }

    pub(crate) async fn update_queued_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt: &str,
    ) -> Result<crate::app::KernelQueuedPromptUpdate, DaemonError> {
        {
            self.owned.update_queued_prompt(
                session_id,
                target_agent_id,
                attachment_id,
                prompt_id,
                prompt,
            )
        }
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let owned = &self.owned;
        let owned_provider_run_id = owned
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                owned
                    .provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .map(|run| run.id().to_string());
        if let Some(completion) = self
            .complete_remote_agent_prompt_if_remote(
                session_id,
                target_agent_id,
                owned_provider_run_id.clone(),
                next_queued_prompt,
            )
            .await?
        {
            self.inject_orphaned_metaagent_task_event_after_turn(
                session_id,
                target_agent_id,
                &completion,
            )?;
            return Ok(completion);
        }
        if next_queued_prompt.is_none() {
            {
                let owned = &self.owned;
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id.as_deref(),
                )? {
                    self.observe_git_after_completed_prompt(
                        Some(session_id),
                        owned_provider_run_id.as_deref(),
                        &completion.completion.completed,
                    )
                    .await;
                    if completion.completion.completed.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            session_id,
                            &completion.completion.completed,
                            owned_provider_run_id.as_deref(),
                        )?;
                        self.spawn_workflow_prompt_dispatches(dispatches);
                    }
                    if completion.released_claim
                        && completion.completion.completed.workflow_run_id().is_none()
                    {
                        self.spawn_workflow_prompt_dispatches(
                            owned.workflow_retry_blocked_claims(),
                        );
                    }
                    self.inject_orphaned_metaagent_task_event_after_turn(
                        session_id,
                        target_agent_id,
                        &completion.completion,
                    )?;
                    self.spawn_workflow_prompt_dispatches(
                        owned.workflow_maybe_start_next_queued_prompt(session_id),
                    );
                    return Ok(completion.completion);
                }
            }
        } else if let Some(next_queued_prompt) = next_queued_prompt {
            if let Some(completion) = owned.complete_local_prompt_with_queued_advance(
                session_id,
                target_agent_id,
                owned_provider_run_id.as_deref(),
                next_queued_prompt,
            )? {
                let completion_result = completion.completion;
                self.observe_git_after_completed_prompt(
                    Some(session_id),
                    owned_provider_run_id.as_deref(),
                    &completion_result.completed,
                )
                .await;
                if completion_result.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion_result.completed,
                        owned_provider_run_id.as_deref(),
                    )?;
                    self.spawn_workflow_prompt_dispatches(dispatches);
                }
                if let Some(started_next) = completion_result.started_next.as_ref() {
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(
                        started_next.source_attachment_id(),
                    ) {
                        owned.workflow_mark_prompt_started(session_id, started_next)?;
                    }
                }
                if let Some(dispatch) = completion.dispatch {
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                    }
                }
                self.inject_orphaned_metaagent_task_event_after_turn(
                    session_id,
                    target_agent_id,
                    &completion_result,
                )?;
                self.spawn_workflow_prompt_dispatches(
                    owned.workflow_maybe_start_next_queued_prompt(session_id),
                );
                return Ok(completion_result);
            }
        }
        Err(DaemonError::LocalTransport {
            operation: "complete prompt",
            message:
                "owned prompt runtime could not complete prompt without side-effect completion"
                    .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_steer_reservation_releases_when_dispatch_is_dropped() {
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-queued",
            "attachment-1",
            "agent-1",
            "queued",
            crate::session::PromptStatus::Queued,
        );
        let reservation = prompt
            .reserve_remote_steer()
            .expect("queued prompt should reserve for one remote steer");
        let guard = RemoteQueuedPromptSteerReservation::new(prompt.clone(), reservation);

        assert!(prompt.remote_steer_reservation_matches(reservation));
        drop(guard);
        assert!(!prompt.remote_steer_reserved());
    }
}

#[derive(Clone)]
struct RemoteQueuedPromptSteerPayload {
    steer_id: String,
    target_home_prompt_id: String,
    prompt: String,
    hidden_system_context: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
}

fn remote_queued_steer_failure_is_definitely_unaccepted(error: &DaemonError) -> bool {
    match error {
        DaemonError::RelayTransport {
            operation: "read temporary relay peer response",
            ..
        } => true,
        DaemonError::LocalTransport { operation, .. } => matches!(
            *operation,
            "connect temporary relay peer socket"
                | "serialize temporary relay register"
                | "write temporary relay register"
                | "serialize temporary relay peer request"
        ),
        _ => false,
    }
}

fn remote_steer_binding_identity_matches(
    current: Option<&crate::agent::RemoteAgentBinding>,
    expected: Option<&crate::agent::RemoteAgentBinding>,
) -> bool {
    let (Some(current), Some(expected)) = (current, expected) else {
        return false;
    };
    current.worker_kernel_id == expected.worker_kernel_id
        && current.worker_machine_id == expected.worker_machine_id
        && current.execution_lease_id == expected.execution_lease_id
        && current.leased_agent_id == expected.leased_agent_id
        && current.relay_url == expected.relay_url
        && current.relay_token == expected.relay_token
        && current.relay_peer_protocol_version == expected.relay_peer_protocol_version
}

async fn send_remote_queued_prompt_steer(
    relay_config: &crate::config::DaemonConfig,
    remote_execution: &crate::agent::RemoteAgentBinding,
    payload: &RemoteQueuedPromptSteerPayload,
) -> Result<RelayPeerResponse, DaemonError> {
    crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            relay_config,
            ClientTarget {
                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::SteerLeasedPrompt {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                steer_id: payload.steer_id.clone(),
                target_home_prompt_id: payload.target_home_prompt_id.clone(),
                prompt: payload.prompt.clone(),
                hidden_system_context: payload.hidden_system_context.clone(),
                attachments: payload.attachments.clone(),
                required_skills: payload.required_skills.clone(),
            },
            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
    )
    .await
}
