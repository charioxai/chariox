//! Provider launch failure recovery.
//!
//! Owns notices, failed run cleanup, remote leased workflow failure forwarding, and automatic
//! substitute activation after a provider run fails before it becomes ready.

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        let mut durable_agent_update = None;
        {
            let owned = &self.owned;
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": started.run.id(),
                    "session_id": started.run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(started.run.session_id());
            owned.record_notice(
                started.run.session_id(),
                Some(started.run.id()),
                recipients,
                format!(
                    "Provider launch `{}` failed before it became ready: {}",
                    started.run.id(),
                    error
                ),
            );
            let diagnostic = format!(
                "Provider launch `{}` failed before it became ready: {}",
                started.run.id(),
                error
            );
            if let Ok(run) = owned
                .provider_store
                .record_terminal_diagnostic(started.run.id(), diagnostic.clone())
            {
                owned.provider_run_projection.update(run);
            }
            if let Some(agent) = clear_failed_codex_resume_state_for_runtime(owned, started, error)
            {
                durable_agent_update = Some(agent);
            }
            let leased_context = self
                .with_app_side_effect(|app| {
                    crate::app::RemoteLeaseRuntime::new(app)
                        .leased_workflow_turn_context_for_provider_run(started.run.id())
                })
                .await;
            if let Some(context) = leased_context {
                let _ = self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                arroba_relay::protocol::ClientTarget {
                                    daemon_id: Some(context.home_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                crate::transport::relay_peer::RelayPeerRequest::ForwardWorkflowProviderFailure {
                                    context,
                                    message: diagnostic.clone(),
                                },
                            ),
                        )
                    })
                    .await;
                let _ = self
                    .with_app_side_effect(|app| {
                        crate::app::RemoteLeaseRuntime::new(app)
                            .complete_leased_workflow_prompt_for_provider_run(started.run.id())
                    })
                    .await;
            } else if let Some(agent_id) = started.run.agent_instance_id() {
                if let Ok(session) = owned.session_store.get_session(started.run.session_id()) {
                    if let Some(active_prompt) = owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent_id)
                    {
                        let _ = self.inject_metaagent_turn_failure_event(
                            started.run.session_id(),
                            agent_id,
                            &active_prompt,
                            Some(started.run.id()),
                            &diagnostic,
                        );
                        if active_prompt.workflow_run_id().is_some() {
                            let _ = owned.workflow_fail_provider_prompt(
                                started.run.session_id(),
                                &active_prompt,
                                Some(started.run.id()),
                                &diagnostic,
                            );
                        }
                        let _ = owned.complete_local_prompt_without_advance(
                            started.run.session_id(),
                            agent_id,
                            Some(started.run.id()),
                        );
                    }
                }
            }
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(started.run.id())
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(started.run.id(), process_key);
            owned.provider_store.clear_runtime(started.run.id());
            if let Ok(outcome) = owned
                .provider_store
                .terminate_run_provider_only(started.run.session_id(), started.run.id())
            {
                let _ = owned.clear_active_provider_run_session_pointer(
                    started.run.session_id(),
                    outcome.run().id(),
                );
                owned.provider_run_projection.update(outcome.into_run());
            }
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                let _ = owned.resume_provider_run_for_session(
                    started.run.session_id(),
                    previous_active_run_id,
                );
            }
            let _ = owned.session_snapshot(started.run.session_id());
        }
        if let Some(agent) = durable_agent_update.as_ref() {
            if let Err(error) = self
                .append_agent_durable_event("agent.runtime_profile_updated", agent, None)
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "failed to persist cleared Codex resume state",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": agent.id(),
                        "provider_run_id": started.run.id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
        if let (Some(agent_id), Some(reason)) = (
            started.run.agent_instance_id(),
            crate::provider::classify_provider_substitutable_failure_text(
                started.run.adapter_key(),
                &error.to_string(),
            ),
        ) {
            if let Err(substitute_error) = self
                .activate_next_agent_substitute_after_failure(
                    started.run.session_id(),
                    agent_id,
                    &reason,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "automatic substitute activation after launch failure failed",
                    serde_json::json!({
                        "session_id": started.run.session_id(),
                        "agent_id": agent_id,
                        "provider_run_id": started.run.id(),
                        "error": substitute_error.to_string(),
                    }),
                );
            }
        }
    }
}

fn clear_failed_codex_resume_state_for_runtime(
    owned: &KernelRuntimeOwnedState,
    started: &crate::app::StartedProviderLaunch,
    error: &DaemonError,
) -> Option<crate::agent::AgentInstance> {
    let replacement_resume_state =
        crate::app::failed_codex_resume_state_replacement(&started.run, error)?;
    let agent_id = started.run.agent_instance_id()?;
    let stale_thread_id = started.run.resume_state().codex_thread_id()?.to_string();
    let current = owned.agent_store.get_agent(agent_id).ok()?;
    if current.provider_resume_state().codex_thread_id() != Some(stale_thread_id.as_str()) {
        return None;
    }
    let agent = owned
        .agent_store
        .set_agent_runtime_profile(
            agent_id,
            started.run.provider(),
            Some(started.run.model().to_string()),
            started.run.variant().map(str::to_string),
            replacement_resume_state,
        )
        .ok()?;
    owned.record_notice(
        started.run.session_id(),
        Some(started.run.id()),
        owned
            .attachment_store
            .list_session_attachment_ids(started.run.session_id()),
        format!(
            "Codex resume thread `{stale_thread_id}` is no longer available. Arroba cleared it from the agent profile so the next prompt can start a new durable Codex thread."
        ),
    );
    Some(agent)
}
