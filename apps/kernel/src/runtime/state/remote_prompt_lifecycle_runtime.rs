//! Remote leased-prompt cancellation and completion runtime.
//!
//! This module owns relay calls that settle or cancel an already-admitted remote prompt.

use super::*;

impl KernelRuntimeState {
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
            let required_mcps = self.required_remote_mcps_for_agent(&agent)?;
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
                                required_mcps,
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
