//! Remote prompt dispatch transport runtime.
//!
//! This module owns leased-agent prompt submission, binding refresh, and remote dispatch result
//! settlement after owned prompt state has already admitted the prompt.

use super::remote_prompt_worker_submission_runtime::submit_remote_prompt_to_worker_with_binding_refresh;
use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_remote_queued_prompt_projection_drain_if_needed(
        &self,
        submission: &crate::app::KernelPromptSubmission,
    ) {
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = &submission.outcome else {
            return;
        };
        let session_id = submission.session.id().to_string();
        let agent_id = prompt.target_agent_id().to_string();
        self.spawn_remote_queued_prompt_projection_drain(session_id, agent_id);
    }

    fn spawn_remote_queued_prompt_projection_drain(&self, session_id: String, agent_id: String) {
        let state = self.clone();
        tokio::spawn(async move {
            for _ in 0..120 {
                let Some((remote_execution, provider_run_id)) =
                    state.remote_queued_prompt_drain_target(&session_id, &agent_id)
                else {
                    return;
                };
                let drained = state
                    .with_app_side_effect(|app| {
                        let relay_config =
                            app.relay_config_for_remote_execution(&remote_execution);
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                &relay_config,
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::DrainLeasedRuntimeProjection {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                    provider_run_id: provider_run_id.clone(),
                                    pump_output: true,
                                },
                            ),
                        )
                    })
                    .await;
                match drained {
                    Ok(RelayPeerResponse::LeasedRuntimeProjectionDrained { event }) => {
                        if let Some(event) = event {
                            if let Err(error) =
                                state.project_remote_runtime_projection_event(event).await
                            {
                                crate::logging::warn_with_fields(
                                    "daemon.remote_prompt_dispatch",
                                    "failed to project drained remote prompt output",
                                    serde_json::json!({
                                        "session_id": session_id,
                                        "agent_id": agent_id,
                                        "provider_run_id": provider_run_id,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                    }
                    Ok(other) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "unexpected remote projection drain response",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "provider_run_id": provider_run_id,
                                "response": format!("{other:?}"),
                            }),
                        );
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote projection drain failed",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "provider_run_id": provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                if state
                    .remote_queued_prompt_drain_target(&session_id, &agent_id)
                    .is_none()
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote queued prompt projection drain timed out",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                }),
            );
        });
    }

    fn remote_queued_prompt_drain_target(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(crate::agent::RemoteAgentBinding, String)> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(session_id).ok()?;
        if owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_none()
        {
            return None;
        }
        owned
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)?;
        let remote_execution = owned
            .agent_store
            .get_agent(agent_id)
            .ok()?
            .remote_execution()
            .cloned()?;
        let provider_run_id = remote_execution.active_worker_provider_run_id.clone()?;
        Some((remote_execution, provider_run_id))
    }

    async fn project_remote_runtime_projection_event(
        &self,
        event: crate::transport::relay_peer::RelayPeerEvent,
    ) -> Result<(), DaemonError> {
        match event {
            crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id,
                home_agent_id,
                provider_run_id,
                prompts,
                output_chunks,
                notices,
                completions,
            } => {
                self.project_relay_remote_runtime_projection(
                    &home_session_id,
                    &home_agent_id,
                    &provider_run_id,
                    prompts,
                    output_chunks,
                    notices,
                    completions,
                )
                .await
            }
        }
    }

    pub(super) async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    let _ = owned
                        .agent_store
                        .set_remote_execution_active_worker_provider_run_id(
                            &dispatch.agent_id,
                            Some(remote_provider_run_id.clone()),
                        )?;
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
                    let _ = owned
                        .agent_store
                        .set_remote_execution_active_worker_provider_run_id(
                            &dispatch.agent_id,
                            None,
                        );
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
            let remote_extension_manifest = match state.remote_extension_manifest_for_agent(&agent)
            {
                Ok(manifest) => manifest,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
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
            let result = submit_remote_prompt_to_worker_with_binding_refresh(
                &state,
                &mut dispatch,
                prompt,
                attachments,
                required_mcps,
                remote_extension_manifest,
            )
            .await;
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
