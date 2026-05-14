//! Remote prompt dispatch transport runtime.
//!
//! This module owns leased-agent prompt submission, cancellation, completion, binding refresh, and
//! remote dispatch result settlement after owned prompt state has already admitted the prompt.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    Ok(())
                }
                Err(error) => {
                    let _ =
                        owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(&dispatch.session_id);
                    owned.record_notice(
                        &dispatch.session_id,
                        None,
                        recipients,
                        format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                    );
                    Err(error)
                }
            }
        }
    }

    pub(super) async fn cancel_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let owned = &self.owned;
        let Some(remote_execution) = owned
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .cloned()
        else {
            return Ok(None);
        };
        match self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CancelLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                    ),
                )
            })
            .await?
        {
            RelayPeerResponse::LeasedPromptCancelled { .. } => {
                Ok(Some(owned.begin_remote_prompt_cancellation(
                    session_id,
                    target_agent_id,
                    attachment_id,
                )?))
            }
            other => Err(DaemonError::LocalTransport {
                operation: "cancel remote prompt",
                message: format!("unexpected remote prompt cancellation response: {other:?}"),
            }),
        }
    }

    pub(super) async fn complete_remote_agent_prompt_if_remote(
        &self,
        session_id: &str,
        target_agent_id: &str,
        owned_provider_run_id: Option<String>,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
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
        let completion_response = self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CompleteLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                    ),
                )
            })
            .await;
        let (remote_provider_run_id, provider_diagnostic) = match completion_response {
            Ok(RelayPeerResponse::LeasedPromptCompleted {
                provider_run_id,
                provider_diagnostic,
                git_observations,
                ..
            }) => {
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
                (
                    provider_run_id.unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    provider_diagnostic,
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
                    owned_provider_run_id
                        .clone()
                        .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    None,
                )
            }
            Err(error) => return Err(error),
            Ok(other) => {
                return Err(DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: format!("unexpected remote prompt completion response: {other:?}"),
                });
            }
        };
        let completion = owned.complete_remote_prompt_owner(
            session_id,
            target_agent_id,
            &remote_provider_run_id,
            next_queued_prompt,
        )?;
        if completion.completed.workflow_run_id().is_some() {
            if let Some(diagnostic) = provider_diagnostic.as_deref() {
                owned.workflow_fail_provider_prompt(
                    session_id,
                    &completion.completed,
                    Some(&remote_provider_run_id),
                    diagnostic,
                )?;
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
            let materialized = self.ensure_remote_skill_packages_for_agent(&agent).await?;
            let remote_prompt = self.apply_remote_materialized_skill_prompt_context(
                &agent,
                started_next.prompt(),
                &materialized,
            )?;
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
            let submit_result = self
                .with_app_side_effect(|app| {
                    let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                    app.block_on_relay_future(
                        crate::transport::relay_client::send_peer_request_via_temporary_connection(
                            &relay_config,
                            ClientTarget {
                                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            RelayPeerRequest::SubmitLeasedPrompt {
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                                prompt: remote_prompt,
                                attachments,
                                workflow_context,
                                git_context: Some(remote_git_turn_context_for_prompt(
                                    session_id,
                                    target_agent_id,
                                    started_next,
                                )),
                                required_mcps: Vec::new(),
                            },
                        ),
                    )
                })
                .await?;
            if let RelayPeerResponse::LeasedPromptSubmitted {
                provider_run_id, ..
            } = submit_result
            {
                owned.echo_prompt_to_other_attachments(
                    session_id,
                    &provider_run_id,
                    started_next.source_attachment_id(),
                    started_next.prompt(),
                    started_next.attachments(),
                );
            }
        }
        Ok(Some(completion))
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            crate::logging::info_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch starting",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "source_attachment_id": dispatch.source_attachment_id,
                }),
            );
            let agent = match state.owned.agent_store.get_agent(&dispatch.agent_id) {
                Ok(agent) => agent,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let materialized = match state.ensure_remote_skill_packages_for_agent(&agent).await {
                Ok(materialized) => materialized,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let prompt = match state.apply_remote_materialized_skill_prompt_context(
                &agent,
                &dispatch.prompt,
                &materialized,
            ) {
                Ok(prompt) => prompt,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let required_mcps = match state.required_remote_mcps_for_agent(&agent) {
                Ok(required_mcps) => required_mcps,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let config = remote_dispatch_relay_config(state.config_snapshot().await, &dispatch);
            let attachments = dispatch.attachments.clone();
            let serialized_attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => Err(DaemonError::LocalTransport {
                    operation: "serialize remote prompt attachments",
                    message: error.to_string(),
                }),
            };
            let attachments = match serialized_attachments {
                Ok(attachments) => attachments,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let result = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(dispatch.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::SubmitLeasedPrompt {
                        leased_agent_id: dispatch.leased_agent_id.clone(),
                        prompt: prompt.clone(),
                        attachments: attachments.clone(),
                        workflow_context: dispatch.workflow_context.clone(),
                        git_context: Some(remote_git_turn_context(&dispatch)),
                        required_mcps: required_mcps.clone(),
                    },
                ),
            )
            .await
            {
                Ok(response) => match response {
                    Ok(RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id, ..
                    }) => Ok(provider_run_id),
                    Ok(other) => Err(DaemonError::LocalTransport {
                        operation: "submit remote prepared prompt",
                        message: format!("unexpected remote prompt response: {other:?}"),
                    }),
                    Err(error) => Err(error),
                },
                Err(_) => Err(DaemonError::LocalTransport {
                    operation: "submit remote prepared prompt",
                    message: "remote prompt dispatch timed out waiting for worker response"
                        .to_string(),
                }),
            };
            let result = if remote_prompt_dispatch_should_refresh_binding(&result) {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt lease stale; refreshing binding",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                    }),
                );
                match state
                    .with_app_side_effect(|app| {
                        app.refresh_remote_agent_binding(&dispatch.agent_id)
                    })
                    .await
                {
                    Ok(agent) => match agent.remote_execution().cloned() {
                        Some(remote_execution) => {
                            dispatch.worker_kernel_id = remote_execution.worker_kernel_id;
                            dispatch.leased_agent_id = remote_execution.leased_agent_id;
                            dispatch.relay_url = remote_execution.relay_url;
                            dispatch.relay_token = remote_execution.relay_token;
                            let config = remote_dispatch_relay_config(
                                state.config_snapshot().await,
                                &dispatch,
                            );
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                &config,
                                ClientTarget {
                                    daemon_id: Some(dispatch.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::SubmitLeasedPrompt {
                                    leased_agent_id: dispatch.leased_agent_id.clone(),
                                    prompt,
                                    attachments,
                                    workflow_context: dispatch.workflow_context.clone(),
                                    git_context: Some(remote_git_turn_context(&dispatch)),
                                    required_mcps,
                                },
                            ))
                            .await
                            {
                                Ok(Ok(RelayPeerResponse::LeasedPromptSubmitted {
                                    provider_run_id, ..
                                })) => Ok(provider_run_id),
                                Ok(Ok(other)) => Err(DaemonError::LocalTransport {
                                    operation: "submit remote prepared prompt",
                                    message: format!("unexpected remote prompt response after binding refresh: {other:?}"),
                                }),
                                Ok(Err(error)) => Err(error),
                                Err(_) => Err(DaemonError::LocalTransport {
                                    operation: "submit remote prepared prompt",
                                    message: "remote prompt dispatch timed out after binding refresh".to_string(),
                                }),
                            }
                        }
                        None => Err(DaemonError::LocalTransport {
                            operation: "refresh remote prompt binding",
                            message: format!(
                                "agent `{}` did not have remote execution after binding refresh",
                                dispatch.agent_id
                            ),
                        }),
                    },
                    Err(error) => Err(error),
                }
            } else {
                result
            };
            match &result {
                Ok(provider_run_id) => crate::logging::info_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt dispatch submitted",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "remote_provider_run_id": provider_run_id,
                    }),
                ),
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "remote prompt dispatch failed",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "error": error.to_string(),
                    }),
                ),
            }
            let _ = state.finish_remote_prompt_dispatch(dispatch, result).await;
        });
    }
}

fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
                || message.contains("timed out waiting for worker response")
        }
        _ => false,
    }
}

fn remote_prompt_completion_should_treat_as_settled(error: &DaemonError) -> bool {
    match error {
        DaemonError::NoActivePrompt { .. } => true,
        DaemonError::LocalTransport { message, .. } => {
            message.contains("no active prompt")
                || message.contains("NoActivePrompt")
                || message.contains("no_active_prompt")
        }
        _ => false,
    }
}

fn remote_git_turn_context(
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

fn remote_git_turn_context_for_prompt(
    session_id: &str,
    agent_id: &str,
    prompt: &crate::session::PromptQueueItem,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: prompt.id().to_string(),
        home_turn_id: prompt.id().to_string(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            prompt.prompt(),
            prompt.attachments(),
        ),
    }
}

fn remote_dispatch_relay_config(
    mut config: crate::config::DaemonConfig,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::config::DaemonConfig {
    if let (Some(relay_url), Some(relay_token)) =
        (dispatch.relay_url.clone(), dispatch.relay_token.clone())
    {
        config.relay_url = Some(relay_url);
        config.relay_token = Some(relay_token);
        config.cloud_relay = None;
    }
    config
}
