//! Remote leased-prompt cancellation and completion runtime.
//!
//! This module owns relay calls that settle or cancel an already-admitted remote prompt.

use super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn cancel_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        authority: Option<(&str, &crate::local::LocalDaemonRequest)>,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        self.authorize_prompt_command(authority)?;
        let owned = &self.owned;
        if owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .is_none()
        {
            return Ok(None);
        }
        let session = owned.session_store.get_session(session_id)?;
        let mut active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if matches!(
            active_prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
                | Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        ) {
            // Persist the user's intent immediately. The dispatch remains held as this same
            // prompt until its normal ACK or a restart receipt proves the worker run.
            let intent = owned.begin_remote_prompt_cancellation(
                session_id,
                target_agent_id,
                attachment_id,
            )?;
            let current_session = owned.session_store.get_session(session_id)?;
            let current_prompt = owned
                .prompt_state_owner
                .active_prompt_for_agent(&current_session, target_agent_id)
                .filter(|prompt| prompt.id() == active_prompt.id());
            if let Some(current_prompt) = current_prompt.filter(|prompt| {
                prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            }) {
                // The ACK may have settled between the initial read and durable intent write.
                // Continue through the exact-run path rather than leaving that race unserviced.
                active_prompt = current_prompt;
            } else {
                return Ok(Some(intent));
            }
        }

        // Route cancellation using the binding that owns the durable worker ACK,
        // never an unverified or stale target.
        let Some(remote_execution) = owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        if !remote_execution.relay_peer_protocol_compatible() {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "worker relay peer protocol does not support run-scoped cancellation"
                    .to_string(),
            });
        }
        let delivered_run_id = active_prompt.durable_delivery_provider_run_id();
        if active_prompt.durable_delivery_phase()
            != Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            || delivered_run_id.is_none()
            || remote_execution.active_worker_provider_run_id.as_deref() != delivered_run_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "remote prompt submission acknowledgement did not bind its worker run"
                    .to_string(),
            });
        }
        let cancellation_intent =
            owned.begin_remote_prompt_cancellation(session_id, target_agent_id, attachment_id)?;
        let Some(mut cancellation_claim) =
            self.try_claim_remote_prompt_cancellation_send(session_id, target_agent_id)
        else {
            return Ok(Some(cancellation_intent));
        };
        cancellation_claim.mark_active_prompt(active_prompt.id());
        let cancellation = self
            .send_remote_agent_prompt_cancellation_with_claim(
                session_id,
                target_agent_id,
                attachment_id,
                &active_prompt,
                cancellation_intent,
                &mut cancellation_claim,
            )
            .await;
        if cancellation_claim.release_or_restart() {
            let state = self.clone();
            let session_id = session_id.to_string();
            let target_agent_id = target_agent_id.to_string();
            tokio::spawn(async move {
                // Consume any deferred successor before rebuilding from authoritative state.
                // The pending dispatch is a wake signal, not permission to bypass phase checks.
                let _ = cancellation_claim.take_pending_dispatch();
                let dispatch = match state
                    .remote_prompt_dispatch_after_claim_restart(&session_id, &target_agent_id)
                    .await
                {
                    Ok(dispatch) => dispatch,
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote prompt claim restart could not prepare the current prompt",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": target_agent_id,
                                "error": error.to_string(),
                            }),
                        );
                        None
                    }
                };
                state
                    .run_remote_prompt_dispatch_with_claim(cancellation_claim, dispatch)
                    .await;
            });
        }
        cancellation
    }

    pub(super) async fn resume_remote_prompt_cancellation_with_claim(
        &self,
        session_id: &str,
        target_agent_id: &str,
        claim: &mut super::remote_prompt_claim_runtime::RemotePromptAgentClaim,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(target_agent_id)?;
        if agent.session_id() != session_id || agent.remote_execution().is_none() {
            return Ok(false);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .filter(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling)
        else {
            return Ok(false);
        };
        match active_prompt.durable_delivery_phase() {
            Some(crate::session::DurablePromptDeliveryPhase::Accepted) => {
                self.finalize_remote_prompt_cancellation_and_advance(
                    session_id,
                    target_agent_id,
                    active_prompt.source_attachment_id(),
                )?;
            }
            Some(crate::session::DurablePromptDeliveryPhase::Delivered) => {
                let attachment_id = active_prompt.source_attachment_id().to_string();
                let cancellation_intent = self.owned.begin_remote_prompt_cancellation(
                    session_id,
                    target_agent_id,
                    &attachment_id,
                )?;
                if let Err(error) = self
                    .send_remote_agent_prompt_cancellation_with_claim(
                        session_id,
                        target_agent_id,
                        &attachment_id,
                        &active_prompt,
                        cancellation_intent,
                        claim,
                    )
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "durable remote cancellation intent remains pending under the recovery claim",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": target_agent_id,
                            "prompt_id": active_prompt.id(),
                            "provider_run_id": active_prompt.durable_delivery_provider_run_id(),
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching) | None => {
                // The worker may own a run, but there is no durable ACK binding it yet.
                // Keep the exact prompt held for receipt recovery without sending by guess.
            }
        }
        Ok(true)
    }

    async fn send_remote_agent_prompt_cancellation_with_claim(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
        active_prompt: &crate::session::PromptQueueItem,
        cancellation_intent: crate::app::KernelPromptCancellation,
        cancellation_claim: &mut super::remote_prompt_claim_runtime::RemotePromptAgentClaim,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let remote_execution = self
            .owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "remote worker binding disappeared before cancellation".to_string(),
            })?;
        if !remote_execution.relay_peer_protocol_compatible() {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "worker relay peer protocol does not support run-scoped cancellation"
                    .to_string(),
            });
        }
        let prompt_id = active_prompt.id().to_string();
        let delivered_run_id = active_prompt
            .durable_delivery_provider_run_id()
            .filter(|_| {
                active_prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            })
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "remote prompt submission acknowledgement did not bind its worker run"
                    .to_string(),
            })?
            .to_string();
        if remote_execution.active_worker_provider_run_id.as_deref()
            != Some(delivered_run_id.as_str())
        {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "remote prompt submission acknowledgement did not bind its worker run"
                    .to_string(),
            });
        }
        self.verify_remote_prompt_cancellation_run(
            session_id,
            target_agent_id,
            &prompt_id,
            &remote_execution.worker_kernel_id,
            &remote_execution.leased_agent_id,
            &delivered_run_id,
        )?;
        if cancellation_claim.cancellation_was_sent(&prompt_id, &delivered_run_id) {
            self.spawn_remote_prompt_projection_drain(
                session_id.to_string(),
                target_agent_id.to_string(),
            );
            return Ok(Some(cancellation_intent));
        }
        let relay_config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(&remote_execution))
            .await;
        self.verify_remote_prompt_cancellation_run(
            session_id,
            target_agent_id,
            &prompt_id,
            &remote_execution.worker_kernel_id,
            &remote_execution.leased_agent_id,
            &delivered_run_id,
        )?;
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let request = RelayPeerRequest::CancelLeasedPrompt {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            home_prompt_id: prompt_id.clone(),
            worker_provider_run_id: delivered_run_id.clone(),
        };
        let cancellation_response =
            if let Some(relay_state) = self.connected_relay_state_for_config(&relay_config).await {
                crate::transport::relay_client::send_peer_request_via_connected_relay(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                )
                .await
            } else {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &relay_config,
                    target,
                    request,
                )
                .await
            };
        match cancellation_response {
            Ok(RelayPeerResponse::LeasedPromptCancelled { cancellation: _ }) => {
                // The response contains the worker's local prompt projection, whose ID is
                // intentionally distinct from the home prompt ID. The request carried both
                // durable identities and the worker compared them atomically before recording
                // its cancellation intent. A successful request is not a terminal receipt: the
                // worker still has to publish the run's completion projection.
                self.verify_remote_prompt_cancellation_run(
                    session_id,
                    target_agent_id,
                    &prompt_id,
                    &remote_execution.worker_kernel_id,
                    &remote_execution.leased_agent_id,
                    &delivered_run_id,
                )?;
                cancellation_claim.mark_cancellation_sent(&prompt_id, &delivered_run_id);
                self.spawn_remote_prompt_projection_drain(
                    session_id.to_string(),
                    target_agent_id.to_string(),
                );
                Ok(Some(cancellation_intent))
            }
            Ok(other) => Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: format!("unexpected remote prompt cancellation response: {other:?}"),
            }),
            Err(error) if remote_prompt_completion_should_treat_as_settled(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt cancellation already settled on worker",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": target_agent_id,
                        "worker_kernel_id": remote_execution.worker_kernel_id,
                        "leased_agent_id": remote_execution.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
                self.verify_remote_prompt_cancellation_run(
                    session_id,
                    target_agent_id,
                    &prompt_id,
                    &remote_execution.worker_kernel_id,
                    &remote_execution.leased_agent_id,
                    &delivered_run_id,
                )?;
                Ok(Some(self.finalize_remote_prompt_cancellation_and_advance(
                    session_id,
                    target_agent_id,
                    attachment_id,
                )?))
            }
            Err(error) => Err(error),
        }
    }

    /// Activates the next queued prompt of an idle remote agent and starts its
    /// worker dispatch.
    pub(super) async fn spawn_next_queued_remote_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let Some(mut submission) = self
            .owned
            .advance_next_queued_remote_prompt_dispatch(session_id, agent_id)?
        else {
            return Ok(None);
        };
        if let Err(error) = self
            .finish_owned_prompt_submission_workflow_start(&mut submission)
            .await
        {
            // The prompt is already active; settle it as a failed dispatch so
            // the agent is not left holding a prompt the worker never got.
            let Some(dispatch) = submission.remote_dispatch.take() else {
                return Err(error);
            };
            // Given an error, this settles the prompt and returns that error.
            self.finish_remote_prompt_dispatch(dispatch, Err(error))
                .await?;
            return Ok(None);
        }
        self.spawn_remote_prompt_projection_drain_if_needed(&submission);
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(Some(submission))
    }

    pub(super) fn finalize_remote_prompt_cancellation_and_advance(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let mut cancellation = self
            .owned
            .finalize_remote_prompt_cancellation_after_worker_settled(
                session_id,
                target_agent_id,
                attachment_id,
            )?;
        // Local cancellation finalization cannot dispatch to the worker, so
        // advance the remote queue here and report the promoted prompt.
        if let Some(mut submission) = self
            .owned
            .advance_next_queued_remote_prompt_dispatch(session_id, target_agent_id)?
        {
            if let Some(dispatch) = submission.remote_dispatch.take() {
                self.spawn_remote_prompt_dispatch(dispatch);
            }
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = submission.outcome
            {
                cancellation.cancellation.started_next = Some(prompt);
            }
            cancellation.session = submission.session;
        }
        Ok(cancellation)
    }

    /// A10: a leased turn's completion settles its home task through the same
    /// settlement as a local turn, so delegators and reply waiters wake.
    pub(super) async fn settle_leased_agent_task(
        &self,
        session_id: &str,
        agent_id: &str,
        completed: &crate::session::PromptQueueItem,
        leased_agent_id: &str,
        worker_provider_run_id: &str,
    ) {
        let run = crate::provider::projected_leased_provider_run_id(
            leased_agent_id,
            worker_provider_run_id,
        );
        let cancelled = completed.status() == crate::session::PromptStatus::Cancelled;
        let settled = match self.owned.settle_agent_task_answered_by(
            session_id,
            agent_id,
            completed,
            &run,
            worker_provider_run_id,
            cancelled,
        ) {
            Ok(settlement) => {
                Box::pin(self.finish_agent_task_settlement(settlement, completed)).await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = settled {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "MP-08/MP-09/MP-10/MP-11 A10: leased task settlement retained for sweep",
                serde_json::json!({"error": crate::secret_redaction::redact_secrets(&error.to_string())}),
            );
        }
    }

    fn verify_remote_prompt_cancellation_run(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let prompt_matches = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .is_some_and(|prompt| {
                prompt.id() == prompt_id
                    && prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                    && prompt.durable_delivery_provider_run_id() == Some(provider_run_id)
            });
        let binding_matches = self
            .owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .is_some_and(|binding| {
                binding.worker_kernel_id.as_str() == worker_kernel_id
                    && binding.leased_agent_id.as_str() == leased_agent_id
                    && binding.active_worker_provider_run_id.as_deref() == Some(provider_run_id)
            });
        if !prompt_matches || !binding_matches {
            return Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "the active prompt or worker run changed before cancellation settled"
                    .to_string(),
            });
        }
        Ok(())
    }

    pub(super) async fn complete_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        owned_provider_run_id: Option<String>,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
        authority: Option<(&str, &crate::local::LocalDaemonRequest)>,
    ) -> Result<Option<crate::session::PromptCompletion>, DaemonError> {
        let owned = &self.owned;
        let Some(remote_execution) = owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        let session = owned.session_store.get_session(session_id)?;
        let completion_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.into(),
            })?;
        let revalidate_receipt = || {
            let agent = owned.agent_store.get_agent(target_agent_id)?;
            let session = owned.session_store.get_session(session_id)?;
            if agent.session_id() != session_id
                || agent.remote_execution() != Some(&remote_execution)
                || owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, target_agent_id)
                    .as_ref()
                    .map(|prompt| prompt.id())
                    != Some(completion_prompt.id())
            {
                return Err(DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "completion receipt was superseded".into(),
                });
            }
            Ok(())
        };
        let completion_response = self
            .with_app_side_effect(|app| {
                self.authorize_prompt_command(authority)?;
                revalidate_receipt()?;
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection_authorized(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CompleteLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                        std::time::Duration::from_millis(relay_config.relay_request_timeout_ms),
                        || self.authorize_prompt_command(authority),
                    ),
                )
            })
            .await;
        let (remote_provider_run_id, provider_diagnostic, provider_termination) =
            match completion_response {
                Ok(RelayPeerResponse::LeasedPromptCompleted {
                    provider_run_id,
                    provider_diagnostic,
                    provider_termination,
                    git_observations,
                    workspace_live_sync_change,
                    ..
                }) => {
                    revalidate_receipt()?;
                    if provider_run_id.as_deref()
                        != remote_execution.active_worker_provider_run_id.as_deref()
                    {
                        return Err(DaemonError::LocalTransport {
                            operation: "complete remote prompt",
                            message: "completion receipt names another provider run".into(),
                        });
                    }
                    if let Err(error) = crate::git_observer::append_observations(
                        &owned.operational_history_store,
                        git_observations,
                    ) {
                        crate::logging::warn_with_fields(
                            "daemon.git_observer",
                            "failed to append remote git observations",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": target_agent_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    if let Some(change) = workspace_live_sync_change {
                        self.record_and_fanout_workspace_live_sync_change(
                            change,
                            Some(&remote_execution.worker_kernel_id),
                            Some(&remote_execution.worker_machine_id),
                        )
                        .await;
                    }
                    revalidate_receipt()?;
                    (
                        provider_run_id
                            .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                        provider_diagnostic,
                        provider_termination,
                    )
                }
                Err(error) if remote_prompt_completion_should_treat_as_settled(&error) => {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt completion already settled on worker",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": target_agent_id,
                            "worker_kernel_id": remote_execution.worker_kernel_id,
                            "leased_agent_id": remote_execution.leased_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    (
                        remote_execution
                            .active_worker_provider_run_id
                            .clone()
                            .or_else(|| owned_provider_run_id.clone())
                            .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                        None,
                        None,
                    )
                }
                Err(error)
                    if remote_prompt_error_should_refresh_binding(&error)
                        && remote_prompt_completion_should_wait_for_binding_repair(
                            owned
                                .agent_store
                                .get_agent(target_agent_id)?
                                .remote_execution(),
                            &remote_execution,
                        ) =>
                {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt completion waiting for stale binding repair",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": target_agent_id,
                            "worker_kernel_id": remote_execution.worker_kernel_id,
                            "leased_agent_id": remote_execution.leased_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: "remote prompt worker binding is being repaired; retry completion"
                            .to_string(),
                    });
                }
                Err(error) => return Err(error),
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            };
        // The worker completion is committed; settle its home receipt even after revocation.
        let completion = owned.complete_remote_prompt_owner_for_receipt(
            session_id,
            target_agent_id,
            &remote_provider_run_id,
            next_queued_prompt,
            provider_termination.clone(),
            Some((completion_prompt.id(), &remote_execution)),
        )?;
        // A10: home alone commits the leased task outcome.
        self.settle_leased_agent_task(
            session_id,
            target_agent_id,
            &completion.completed,
            &remote_execution.leased_agent_id,
            &remote_provider_run_id,
        )
        .await;
        if completion.completed.workflow_run_id().is_some() {
            if let Some(diagnostic) = provider_termination
                .as_ref()
                .map(|termination| termination.reason.as_str())
                .or(provider_diagnostic.as_deref())
            {
                let dispatches = owned.workflow_fail_provider_prompt(
                    session_id,
                    &completion.completed,
                    Some(&remote_provider_run_id),
                    diagnostic,
                )?;
                self.spawn_workflow_prompt_dispatches(dispatches);
            } else {
                let dispatches = owned.workflow_complete_prompt(
                    session_id,
                    &completion.completed,
                    Some(&remote_provider_run_id),
                )?;
                self.spawn_workflow_prompt_dispatches(dispatches);
            }
        }
        if let Some(started_next) = completion.started_next.as_ref() {
            let agent = self.owned.agent_store.get_agent(target_agent_id)?;
            let (remote_prompt, _) = self
                .prepare_remote_prompt_skill_context(&agent, started_next.prompt())
                .await?;
            let attachments = self
                .with_app_side_effect(|app| {
                    app.serialize_remote_prompt_attachments(started_next.attachments())
                })
                .await?;
            let workflow_context = if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                Some(
                    self.with_app_side_effect(|app| {
                        crate::app::RemoteWorkflowTurnContextResolver::new(app)
                            .remote_workflow_turn_context_for_prompt(
                                session_id,
                                target_agent_id,
                                started_next,
                            )
                    })
                    .await?,
                )
            } else {
                None
            };
            let session = owned.session_store.get_session(session_id)?;
            let mut dispatch = crate::app::KernelRemotePromptDispatch {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
                prompt_id: started_next.id().to_string(),
                worker_kernel_id: remote_execution.worker_kernel_id.clone(),
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                relay_url: remote_execution.relay_url.clone(),
                relay_token: remote_execution.relay_token.clone(),
                source_attachment_id: started_next.source_attachment_id().to_string(),
                prompt: remote_prompt.clone(),
                hidden_system_context: started_next.hidden_system_context().to_string(),
                attachments: started_next.attachments().to_vec(),
                workspace_live_sync_mode: Some(
                    crate::provider::provider_workspace_live_sync_mode_for_session(
                        agent.provider(),
                        &owned.config_projection.snapshot(),
                        Some(&session),
                    ),
                ),
                prompt_origin: started_next.prompt_origin(),
                external_provider: started_next.external_provider().map(str::to_string),
                external_provider_session_id: started_next
                    .external_provider_session_id()
                    .map(str::to_string),
                external_provider_turn_id: started_next
                    .external_provider_turn_id()
                    .map(str::to_string),
                workflow_context,
            };
            let provider_run_id = super::remote_prompt_worker_submission_runtime::submit_remote_prompt_to_worker_with_binding_refresh(
                self,
                &mut dispatch,
                remote_prompt,
                attachments,
            )
            .await?;
            owned
                .agent_store
                .set_remote_execution_active_worker_provider_run_id(
                    target_agent_id,
                    Some(provider_run_id.clone()),
                )?;
            owned.mark_active_prompt_delivery(
                session_id,
                target_agent_id,
                started_next.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(provider_run_id.clone()),
                None,
            )?;
            let _ = owned.session_snapshot(session_id)?;
            let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
                &dispatch.leased_agent_id,
                &provider_run_id,
            );
            owned.echo_promoted_queued_prompt_to_attachments(
                session_id,
                &projected_provider_run_id,
                started_next.id(),
                started_next.source_attachment_id(),
                started_next.prompt(),
                started_next.attachments(),
            );
        }
        Ok(Some(completion))
    }
}

fn remote_prompt_completion_should_treat_as_settled(error: &DaemonError) -> bool {
    match error {
        DaemonError::NoActivePrompt { .. } => true,
        DaemonError::RelayTransport {
            operation,
            code,
            message,
            retryable: false,
        } => {
            matches!(
                *operation,
                "read relay peer response" | "read temporary relay peer response"
            ) && (code == "no_active_prompt"
                || (code == "relay_request_failed"
                    && message
                        .strip_prefix("session `")
                        .and_then(|message| message.strip_suffix("` has no active prompt"))
                        .is_some_and(|session_id| !session_id.is_empty())))
        }
        _ => false,
    }
}

fn remote_prompt_completion_should_wait_for_binding_repair(
    current_binding: Option<&crate::agent::RemoteAgentBinding>,
    attempted_binding: &crate::agent::RemoteAgentBinding,
) -> bool {
    let Some(current_binding) = current_binding else {
        return false;
    };
    current_binding.leased_agent_id != attempted_binding.leased_agent_id
        || current_binding.active_worker_provider_run_id.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        leased_agent_id: &str,
        active_run: Option<&str>,
    ) -> crate::agent::RemoteAgentBinding {
        crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease".to_string(),
            leased_agent_id: leased_agent_id.to_string(),
            active_worker_provider_run_id: active_run.map(str::to_string),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queued_remote_prompt_settles_when_its_workflow_start_fails() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        app.agents
            .bind_remote_execution(agent.id(), binding("leased-agent-1", None))
            .expect("agent should bind to the worker");
        let workflow_prompt = crate::session::PromptQueueItem::new(
            "pending:workflow",
            crate::scheduler::runtime::workflow_prompt_source_attachment_id("missing-run"),
            agent.id(),
            "workflow turn",
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context("missing-run", "missing-node");
        app.prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, true)
            .expect("workflow prompt should queue");
        let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
        let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
        let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
            std::sync::Arc::clone(&app),
            1,
        )
        .runtime_state();

        assert!(state
            .spawn_next_queued_remote_prompt(&session_id, &agent_id)
            .await
            .is_err());

        let mut app = app.lock().await;
        assert!(
            app.prompt_owner_active_prompt_for_agent(&session_id, &agent_id)
                .expect("prompt state should load")
                .is_none(),
            "a prompt that never reached the worker must not stay active"
        );
        assert_eq!(
            app.agents()
                .get_agent(&agent_id)
                .expect("agent should load")
                .state(),
            crate::agent::AgentState::Error
        );
    }

    #[test]
    fn remote_completion_waits_when_binding_has_no_active_worker_run() {
        let attempted = binding("leased-agent-old", None);

        assert!(remote_prompt_completion_should_wait_for_binding_repair(
            Some(&attempted),
            &attempted,
        ));
    }

    #[test]
    fn remote_completion_waits_when_binding_already_changed() {
        let attempted = binding("leased-agent-old", Some("worker-run-old"));
        let current = binding("leased-agent-new", Some("worker-run-new"));

        assert!(remote_prompt_completion_should_wait_for_binding_repair(
            Some(&current),
            &attempted,
        ));
    }

    #[test]
    fn remote_completion_does_not_wait_when_submitted_binding_matches() {
        let attempted = binding("leased-agent-old", Some("worker-run-old"));

        assert!(!remote_prompt_completion_should_wait_for_binding_repair(
            Some(&attempted),
            &attempted,
        ));
    }

    #[test]
    fn settled_remote_completion_accepts_only_exact_worker_no_active_prompt() {
        assert!(remote_prompt_completion_should_treat_as_settled(
            &DaemonError::NoActivePrompt {
                session_id: "worker-session".to_string(),
            }
        ));
        assert!(remote_prompt_completion_should_treat_as_settled(
            &DaemonError::RelayTransport {
                // map_relay_error falls back to relay_request_failed and the typed
                // NoActivePrompt display string for this worker response.
                operation: "read relay peer response",
                code: "relay_request_failed".to_string(),
                message: "session `worker-session` has no active prompt".to_string(),
                retryable: false,
            }
        ));
        assert!(remote_prompt_completion_should_treat_as_settled(
            &DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "no_active_prompt".to_string(),
                message: "worker reports no active prompt".to_string(),
                retryable: false,
            }
        ));
        for error in [
            DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: "session `worker-session` has no active prompt".to_string(),
            },
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "relay_request_failed".to_string(),
                message: "worker said there is no active prompt while another error occurred"
                    .to_string(),
                retryable: false,
            },
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "unauthorized".to_string(),
                message: "request denied: no_active_prompt was mentioned".to_string(),
                retryable: false,
            },
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "relay_request_failed".to_string(),
                message:
                    "timed out waiting for worker: session `worker-session` has no active prompt"
                        .to_string(),
                retryable: false,
            },
            DaemonError::RelayTransport {
                operation: "write relay peer request",
                code: "relay_request_failed".to_string(),
                message: "session `worker-session` has no active prompt".to_string(),
                retryable: false,
            },
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "relay_request_failed".to_string(),
                message: "session `worker-session` has no active prompt".to_string(),
                retryable: true,
            },
            DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: "no_active_prompt".to_string(),
                message: "worker reports no active prompt".to_string(),
                retryable: true,
            },
        ] {
            assert!(
                !remote_prompt_completion_should_treat_as_settled(&error),
                "untyped remote errors must not prove the worker run settled: {error}"
            );
        }
    }
}

#[cfg(test)]
#[path = "remote_prompt_lifecycle_runtime/cancellation_tests.rs"]
mod cancellation_tests;
