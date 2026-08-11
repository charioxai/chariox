//! Remote prompt dispatch transport runtime.
//!
//! This module owns leased-agent prompt submission, binding refresh, and remote dispatch result
//! settlement after owned prompt state has already admitted the prompt.

use super::remote_prompt_worker_submission_runtime::{
    remote_prompt_error_should_refresh_binding, remote_prompt_error_should_retry_transport,
    remote_prompt_transport_retry_delay, submit_remote_prompt_to_worker_with_binding_refresh,
};
use super::*;

const REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

// Duplicate starts advance the generation so release and restart are one atomic decision.
struct RemotePromptAgentClaim {
    key: (String, String),
    claims: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
    seen_generation: u64,
    released: bool,
}

impl RemotePromptAgentClaim {
    fn try_acquire(
        claims: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
        session_id: &str,
        agent_id: &str,
    ) -> Option<Self> {
        let key = (session_id.to_string(), agent_id.to_string());
        let mut claims_guard = claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(generation) = claims_guard.get_mut(&key) {
            *generation = generation.saturating_add(1);
            return None;
        }
        claims_guard.insert(key.clone(), 0);
        drop(claims_guard);
        Some(Self {
            key,
            claims,
            seen_generation: 0,
            released: false,
        })
    }

    fn release_or_restart(&mut self) -> bool {
        let mut claims = self
            .claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = claims.get(&self.key).copied() else {
            self.released = true;
            return false;
        };
        if generation != self.seen_generation {
            self.seen_generation = generation;
            return true;
        }
        claims.remove(&self.key);
        self.released = true;
        false
    }
}

impl Drop for RemotePromptAgentClaim {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl KernelRuntimeState {
    pub(super) async fn recover_remote_prompt_after_kernel_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        delivery_phase: Option<crate::session::DurablePromptDeliveryPhase>,
        delivery_provider_run_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let active_worker_run = agent
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
            .is_some();
        if delivery_phase != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            && !active_worker_run
        {
            if let Some(provider_run_id) = delivery_provider_run_id {
                self.owned
                    .agent_store
                    .set_remote_execution_active_worker_provider_run_id(
                        agent_id,
                        Some(provider_run_id.to_string()),
                    )?;
            }
        }
        let active_worker_run = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
            .is_some();
        if active_worker_run
            && delivery_phase != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        {
            self.spawn_remote_prompt_projection_drain(session_id.to_string(), agent_id.to_string());
            return Ok(true);
        }
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch(&agent)? else {
            return Ok(false);
        };
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;
        self.spawn_remote_prompt_dispatch(dispatch);
        Ok(true)
    }

    pub(super) fn spawn_remote_prompt_projection_drain_if_needed(
        &self,
        submission: &crate::app::KernelPromptSubmission,
    ) {
        let prompt = match &submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
        };
        let session_id = submission.session.id().to_string();
        let agent_id = prompt.target_agent_id().to_string();
        self.spawn_remote_prompt_projection_drain(session_id, agent_id);
    }

    fn spawn_remote_prompt_projection_drain(&self, session_id: String, agent_id: String) {
        let Some(mut claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_projection_drains),
            &session_id,
            &agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let mut transport_retry_attempt = 0_u32;
            loop {
                match state
                    .drain_remote_prompt_projection_once(&session_id, &agent_id)
                    .await
                {
                    Ok(true) => transport_retry_attempt = 0,
                    Ok(false) => {
                        if claim.release_or_restart() {
                            continue;
                        }
                        return;
                    }
                    Err(error) if remote_prompt_error_should_retry_transport(&error) => {
                        transport_retry_attempt = transport_retry_attempt.saturating_add(1);
                        if transport_retry_attempt == 1 || transport_retry_attempt % 12 == 0 {
                            crate::logging::warn_with_fields(
                                "daemon.remote_prompt_dispatch",
                                "remote projection transport unavailable; retrying active prompt",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "agent_id": agent_id,
                                    "attempt": transport_retry_attempt,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        if state
                            .remote_prompt_projection_drain_target(&session_id, &agent_id)
                            .is_none()
                        {
                            if claim.release_or_restart() {
                                continue;
                            }
                            return;
                        }
                        tokio::time::sleep(remote_prompt_transport_retry_delay(
                            transport_retry_attempt,
                        ))
                        .await;
                        continue;
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote projection drain failed",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "error": error.to_string(),
                            }),
                        );
                        if claim.release_or_restart() {
                            continue;
                        }
                        return;
                    }
                }
                if state
                    .remote_prompt_projection_drain_target(&session_id, &agent_id)
                    .is_none()
                {
                    if claim.release_or_restart() {
                        continue;
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    fn remote_prompt_projection_drain_target(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(crate::agent::RemoteAgentBinding, String)> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(session_id).ok()?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)?;
        if matches!(
            active_prompt.durable_delivery_phase(),
            Some(
                crate::session::DurablePromptDeliveryPhase::Accepted
                    | crate::session::DurablePromptDeliveryPhase::Dispatching
            )
        ) {
            return None;
        }
        let remote_execution = owned
            .agent_store
            .get_agent(agent_id)
            .ok()?
            .remote_execution()
            .cloned()?;
        let provider_run_id = remote_execution.active_worker_provider_run_id.clone()?;
        Some((remote_execution, provider_run_id))
    }

    pub(super) async fn drain_active_remote_prompt_projections_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Result<(), DaemonError> {
        for agent_id in self
            .owned
            .prompt_state_owner
            .active_prompt_agent_ids(session)
        {
            let _ = self
                .drain_remote_prompt_projection_once(session.id(), &agent_id)
                .await?;
        }
        Ok(())
    }

    async fn drain_remote_prompt_projection_once(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some((remote_execution, provider_run_id)) =
            self.remote_prompt_projection_drain_target(session_id, agent_id)
        else {
            return Ok(false);
        };
        let relay_config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(&remote_execution))
            .await;
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let request = RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            provider_run_id: provider_run_id.clone(),
            pump_output: true,
        };
        let response = match self.connected_relay_state_for_config(&relay_config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &relay_config,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(error) if remote_prompt_error_should_refresh_binding(&error) => {
                self.spawn_stale_remote_prompt_recovery(
                    session_id.to_string(),
                    agent_id.to_string(),
                    remote_execution,
                    provider_run_id,
                    error.to_string(),
                );
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        match response {
            RelayPeerResponse::LeasedRuntimeProjectionDrained { event } => {
                if let Some(event) = event {
                    self.project_remote_runtime_projection_event(event).await?;
                }
                Ok(true)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "drain remote prompt projection",
                message: format!("unexpected remote projection drain response: {other:?}"),
            }),
        }
    }

    fn spawn_stale_remote_prompt_recovery(
        &self,
        session_id: String,
        agent_id: String,
        stale_binding: crate::agent::RemoteAgentBinding,
        stale_provider_run_id: String,
        trigger_error: String,
    ) {
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &session_id,
            &agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
            if let Err(error) = state
                .recover_stale_remote_prompt(
                    &session_id,
                    &agent_id,
                    &stale_binding,
                    &stale_provider_run_id,
                    &trigger_error,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "stale remote prompt recovery stopped",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": stale_binding.worker_kernel_id,
                        "leased_agent_id": stale_binding.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
            }
        });
    }

    async fn recover_stale_remote_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        stale_binding: &crate::agent::RemoteAgentBinding,
        stale_provider_run_id: &str,
        trigger_error: &str,
    ) -> Result<(), DaemonError> {
        let rebound = self
            .with_app_side_effect(|app| {
                let agent = app.agents().get_agent(agent_id)?;
                let Some(current_binding) = agent.remote_execution() else {
                    return Ok(None);
                };
                if current_binding.leased_agent_id != stale_binding.leased_agent_id
                    || current_binding.active_worker_provider_run_id.as_deref()
                        != Some(stale_provider_run_id)
                {
                    return Ok(None);
                }
                app.refresh_remote_agent_binding(agent_id).map(Some)
            })
            .await?;
        let Some(rebound) = rebound else {
            return Ok(());
        };
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch(&rebound)? else {
            return Ok(());
        };
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;
        crate::logging::warn_with_fields(
            "daemon.remote_prompt_dispatch",
            "replaying active prompt after stale worker binding",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "previous_worker_kernel_id": stale_binding.worker_kernel_id,
                "previous_leased_agent_id": stale_binding.leased_agent_id,
                "worker_kernel_id": dispatch.worker_kernel_id,
                "leased_agent_id": dispatch.leased_agent_id,
                "prompt_id": dispatch.prompt_id,
                "trigger_error": trigger_error,
            }),
        );

        let prompt_id = dispatch.prompt_id.clone();
        let mut attempt = 0_u32;
        loop {
            if !self.remote_prompt_recovery_is_current(
                session_id,
                agent_id,
                &prompt_id,
                &dispatch.leased_agent_id,
            )? {
                return Ok(());
            }
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            let (prompt, required_skills) = match self
                .prepare_remote_prompt_skill_context(&agent, &dispatch.prompt)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    attempt = attempt.saturating_add(1);
                    self.log_remote_prompt_recovery_retry(
                        session_id, agent_id, &dispatch, attempt, &error,
                    );
                    tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                    continue;
                }
            };
            let (required_mcps, remote_extension_manifest) =
                match self.remote_prompt_mcp_capabilities_for_agent(&agent) {
                    Ok(capabilities) => capabilities,
                    Err(error) => {
                        attempt = attempt.saturating_add(1);
                        self.log_remote_prompt_recovery_retry(
                            session_id, agent_id, &dispatch, attempt, &error,
                        );
                        tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                        continue;
                    }
                };
            let attachments = dispatch.attachments.clone();
            let attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(Ok(attachments)) => attachments,
                Ok(Err(error)) => return Err(error),
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "serialize recovered remote prompt attachments",
                        message: error.to_string(),
                    });
                }
            };
            let result = submit_remote_prompt_to_worker_with_binding_refresh(
                self,
                &mut dispatch,
                prompt,
                attachments,
                required_mcps,
                required_skills,
                remote_extension_manifest,
            )
            .await;
            match result {
                Ok(provider_run_id) => {
                    self.finish_stale_remote_prompt_recovery(
                        session_id,
                        agent_id,
                        stale_binding,
                        stale_provider_run_id,
                        &dispatch,
                        &provider_run_id,
                    )?;
                    crate::logging::info_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "active remote prompt recovered on refreshed worker binding",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "provider_run_id": provider_run_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": attempt + 1,
                        }),
                    );
                    let state = self.clone();
                    let session_id = session_id.to_string();
                    let agent_id = agent_id.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        state.spawn_remote_prompt_projection_drain(session_id, agent_id);
                    });
                    return Ok(());
                }
                Err(error) => {
                    attempt = attempt.saturating_add(1);
                    self.log_remote_prompt_recovery_retry(
                        session_id, agent_id, &dispatch, attempt, &error,
                    );
                    tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                }
            }
        }
    }

    fn remote_prompt_recovery_dispatch(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(None);
        };
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
        else {
            return Ok(None);
        };
        Ok(Some(crate::app::KernelRemotePromptDispatch {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            prompt_id: active_prompt.id().to_string(),
            worker_kernel_id: remote_execution.worker_kernel_id.clone(),
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            relay_url: remote_execution.relay_url.clone(),
            relay_token: remote_execution.relay_token.clone(),
            source_attachment_id: active_prompt.source_attachment_id().to_string(),
            prompt: active_prompt.prompt().to_string(),
            attachments: active_prompt.attachments().to_vec(),
            workspace_live_sync_mode: Some(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    agent.provider(),
                    &self.owned.config_projection.snapshot(),
                    Some(&session),
                ),
            ),
            prompt_origin: active_prompt.prompt_origin(),
            external_provider: active_prompt.external_provider().map(str::to_string),
            external_provider_session_id: active_prompt
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: active_prompt
                .external_provider_turn_id()
                .map(str::to_string),
            workflow_context: None,
        }))
    }

    async fn populate_remote_prompt_recovery_workflow_context(
        &self,
        dispatch: &mut crate::app::KernelRemotePromptDispatch,
    ) -> Result<(), DaemonError> {
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&dispatch.source_attachment_id)
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            })?;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        dispatch.workflow_context = Some(
            self.with_app_side_effect(move |app| {
                crate::app::RemoteWorkflowTurnContextResolver::new(app)
                    .remote_workflow_turn_context_for_prompt(&session_id, &agent_id, &prompt)
            })
            .await?,
        );
        Ok(())
    }

    fn remote_prompt_recovery_is_current(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        leased_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let prompt_is_current = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.id() == prompt_id);
        let binding_is_current = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .is_some_and(|binding| binding.leased_agent_id == leased_agent_id);
        Ok(prompt_is_current && binding_is_current)
    }

    fn finish_stale_remote_prompt_recovery(
        &self,
        session_id: &str,
        agent_id: &str,
        stale_binding: &crate::agent::RemoteAgentBinding,
        stale_provider_run_id: &str,
        recovered_dispatch: &crate::app::KernelRemotePromptDispatch,
        recovered_provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if !self.remote_prompt_recovery_is_current(
            session_id,
            agent_id,
            &recovered_dispatch.prompt_id,
            &recovered_dispatch.leased_agent_id,
        )? {
            return Ok(());
        }
        let stale_projected_run_id = crate::provider::projected_leased_provider_run_id(
            &stale_binding.leased_agent_id,
            stale_provider_run_id,
        );
        if let Some(mut stale_run) = self
            .owned
            .provider_run_projection
            .get(&stale_projected_run_id)
        {
            stale_run.mark_ended();
            self.owned
                .clear_active_provider_run_session_pointer(session_id, stale_run.id())?;
            self.owned.clear_prompt_activity(stale_run.id());
            self.owned.provider_run_projection.update(stale_run);
        }
        self.owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                agent_id,
                Some(recovered_provider_run_id.to_string()),
            )?;
        let _ = self.owned.session_snapshot(session_id)?;
        Ok(())
    }

    fn log_remote_prompt_recovery_retry(
        &self,
        session_id: &str,
        agent_id: &str,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        attempt: u32,
        error: &DaemonError,
    ) {
        if attempt == 1 || attempt % 12 == 0 {
            crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "active remote prompt recovery retrying",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "prompt_id": dispatch.prompt_id,
                    "attempt": attempt,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) async fn connected_relay_state_for_config(
        &self,
        relay_config: &crate::config::DaemonConfig,
    ) -> Option<Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>> {
        let relay_url = relay_config.relay_url.as_deref()?;
        if self
            .owned
            .relay_state
            .read()
            .await
            .connected_relay_url()
            .as_deref()
            == Some(relay_url)
        {
            return Some(Arc::clone(&self.owned.relay_state));
        }
        let slice_states = {
            let connectors = self.owned.slice_private_relay_connectors.lock().await;
            connectors
                .values()
                .filter(|connector| connector.relay_url == relay_url)
                .map(|connector| Arc::clone(&connector.state))
                .collect::<Vec<_>>()
        };
        for state in slice_states {
            if state.read().await.connected_relay_url().as_deref() == Some(relay_url) {
                return Some(state);
            }
        }
        None
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
                provider_run,
                prompts,
                output_chunks,
                notices,
                completions,
            } => {
                self.project_relay_remote_runtime_projection(
                    &home_session_id,
                    &home_agent_id,
                    &provider_run_id,
                    provider_run,
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
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let should_start_projection_drain = {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    if owned.agent_store.get_agent(&dispatch.agent_id)?.state()
                        == crate::agent::AgentState::Error
                    {
                        let _ = owned
                            .agent_store
                            .set_agent_state(&dispatch.agent_id, crate::agent::AgentState::Idle)?;
                    }
                    let _ = owned
                        .agent_store
                        .set_remote_execution_active_worker_provider_run_id(
                            &dispatch.agent_id,
                            Some(remote_provider_run_id.clone()),
                        )?;
                    let _ = owned.session_snapshot(&dispatch.session_id)?;
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.prompt_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                        None,
                    );
                    owned.mark_active_prompt_delivery(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        crate::session::DurablePromptDeliveryPhase::Delivered,
                        Some(remote_provider_run_id),
                        None,
                    )?;
                    Ok(true)
                }
                Err(error) => {
                    let message =
                        format!("Remote prompt dispatch failed after acknowledgement: {error}");
                    let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                    let merge_key = Some(format!("remote-dispatch-error:{}", dispatch.prompt_id));
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(&dispatch.session_id);
                    owned.fan_out_terminal_outputs_to_recipients(
                        &dispatch.session_id,
                        recipients,
                        vec![
                            super::prompt_transcript_owned_state::TerminalOutputBatchAppend {
                                provider_run_id: provider_run_id.clone(),
                                agent_id: Some(dispatch.agent_id.clone()),
                                kind: crate::terminal::TerminalOutputKind::ProviderError,
                                merge_key: merge_key.clone(),
                                bytes: message.as_bytes().to_vec(),
                            },
                        ],
                    );
                    owned.append_history_entry(
                        &dispatch.session_id,
                        SessionHistoryEntry::provider_output(
                            &dispatch.session_id,
                            &provider_run_id,
                            Some(&dispatch.agent_id),
                            crate::terminal::TerminalOutputKind::ProviderError,
                            merge_key,
                            message,
                        )
                        .with_prompt_origin(dispatch.prompt_origin)
                        .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                    );
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                        Some(error.to_string()),
                    );
                    let _ = owned
                        .agent_store
                        .set_remote_execution_active_worker_provider_run_id(
                            &dispatch.agent_id,
                            None,
                        );
                    let _ =
                        owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
                    let _ = owned
                        .agent_store
                        .set_agent_processing(&dispatch.agent_id, false);
                    let _ = owned
                        .agent_store
                        .set_agent_state(&dispatch.agent_id, crate::agent::AgentState::Error);
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    Err(error)
                }
            }
        }?;
        if should_start_projection_drain {
            self.spawn_remote_prompt_projection_drain(session_id, agent_id);
        }
        Ok(())
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        // A stale projection drain can discover a dead lease while the initial
        // dispatch is already refreshing that same binding. Both paths submit
        // the active prompt, so serialize them per agent to prevent one browser
        // prompt from starting on two freshly-created worker agents.
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &dispatch.session_id,
            &dispatch.agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
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
            if let Err(error) = state.owned.mark_active_prompt_delivery(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            ) {
                let _ = state
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return;
            }
            let agent = match state.owned.agent_store.get_agent(&dispatch.agent_id) {
                Ok(agent) => agent,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let (prompt, required_skills) = match state
                .prepare_remote_prompt_skill_context(&agent, &dispatch.prompt)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    let _ = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await;
                    return;
                }
            };
            let (required_mcps, remote_extension_manifest) =
                match state.remote_prompt_mcp_capabilities_for_agent(&agent) {
                    Ok(capabilities) => capabilities,
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
                required_skills,
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

fn remote_prompt_recovery_delay(attempt: u32) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(500_u64.saturating_mul(multiplier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

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
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
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

    #[tokio::test]
    async fn remote_prompt_projection_drain_respects_durable_delivery_phase() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-projection-drain",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-worker-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "remote prompt\n",
            crate::session::PromptStatus::Queued,
        );
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start locally");
        let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
            panic!("remote prompt should start locally");
        };
        assert_eq!(
            app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
                .expect("queue count should load"),
            0
        );

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available"),
                agent.id(),
            )
            .expect("remote prompt should remain active");
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
        assert!(
            runtime
                .remote_prompt_projection_drain_target(session.id(), agent.id())
                .is_none(),
            "an accepted prompt must not drain the prior worker run"
        );

        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("provider-run-worker-1".to_string()),
                None,
            )
            .expect("delivered phase should persist");
        assert!(
            runtime
                .remote_prompt_projection_drain_target(session.id(), agent.id())
                .is_some(),
            "a delivered prompt should drain its worker run"
        );
    }

    #[tokio::test]
    async fn delivered_remote_prompt_restores_worker_run_before_restart_drain() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-remote-restart",
                "worktree-remote-restart",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-restart",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-remote-restart",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let prompt_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("remote prompt should start")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some("provider-run-worker-1".to_string()),
            None,
        )
        .expect("delivery metadata should persist");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        assert!(runtime
            .recover_remote_prompt_after_kernel_restart(
                session.id(),
                agent.id(),
                Some(crate::session::DurablePromptDeliveryPhase::Delivered),
                Some("provider-run-worker-1"),
            )
            .await
            .expect("remote recovery should start"));
        let restored = runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available");
        assert_eq!(
            restored
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.as_deref()),
            Some("provider-run-worker-1")
        );
    }

    #[tokio::test]
    async fn remote_prompt_dispatch_success_refreshes_session_projection() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-dispatch-projection",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to remote execution");

        let projection_store = app.session_state_projection_store();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let prompt = crate::session::PromptQueueItem::new(
            "pending:remote-dispatch-projection",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");
        let dispatch = submission
            .remote_dispatch
            .expect("started remote prompt should dispatch");

        runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("provider-run-worker-1".to_string()))
            .await
            .expect("remote prompt dispatch should settle");

        let projected = projection_store
            .get(session.id())
            .expect("session projection should remain available");
        let projected_agent = projected
            .agents()
            .iter()
            .find(|candidate| candidate.id() == agent.id())
            .expect("remote agent should remain projected");
        assert_eq!(
            projected_agent
                .remote_execution()
                .and_then(|remote| remote.active_worker_provider_run_id.as_deref()),
            Some("provider-run-worker-1"),
            "dispatch settlement must refresh the warm session projection"
        );
    }

    #[tokio::test]
    async fn remote_prompt_dispatch_failure_projects_agent_error() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-dispatch-error",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to remote execution");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let prompt = crate::session::PromptQueueItem::new(
            "pending:remote-dispatch-error",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");
        let dispatch = submission
            .remote_dispatch
            .expect("started remote prompt should dispatch");

        let result = runtime
            .finish_remote_prompt_dispatch(
                dispatch,
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "submit remote prompt",
                    message: "provider rejected the prompt".to_string(),
                }),
            )
            .await;

        assert!(result.is_err(), "dispatch failure must be preserved");
        let failed_agent = runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("failed agent should remain available");
        assert_eq!(failed_agent.state(), crate::agent::AgentState::Error);
        assert!(!failed_agent.is_processing());
        {
            let mut sessions = runtime.owned.session_store.write();
            runtime
                .owned
                .agent_store
                .focus_agent(session.id(), agent.id(), &mut sessions)
                .expect("failed agent should remain focusable");
        }
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(agent.id())
                .expect("focused failed agent should remain available")
                .state(),
            crate::agent::AgentState::Error,
            "focusing or restoring a failed pane must not erase its error badge",
        );
        let snapshot = runtime
            .owned
            .session_snapshot(session.id())
            .expect("failed session should remain projectable");
        assert_eq!(
            runtime
                .agent_activity_for_session(&snapshot)
                .get(agent.id())
                .expect("failed agent activity should remain projected")
                .status,
            crate::runtime::projection::AgentRuntimeStatus::Error,
        );
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available"),
                agent.id(),
            )
            .is_none());
    }

    #[test]
    fn remote_prompt_projection_drain_claims_coalesce_restart_before_release() {
        let claims = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let mut first =
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-1", "agent-1")
                .expect("first drain should claim the agent");

        let other_agent =
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-2", "agent-2")
                .expect("a different agent must remain independently dispatchable");

        assert!(
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-1", "agent-1",)
                .is_none(),
            "a duplicate drain must not start while the first owner is alive"
        );
        assert!(
            first.release_or_restart(),
            "the active owner must consume a start request that arrived before release"
        );
        assert!(
            !first.release_or_restart(),
            "the owner must release once no newer start request remains"
        );

        assert!(
            RemotePromptAgentClaim::try_acquire(claims, "session-1", "agent-1",).is_some(),
            "an atomically released claim must allow reconnect recovery to start a new drain"
        );
        drop(other_agent);
    }

    #[test]
    fn remote_prompt_projection_transport_retry_delay_is_bounded() {
        assert_eq!(
            remote_prompt_transport_retry_delay(1),
            std::time::Duration::from_millis(250)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(2),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(4),
            std::time::Duration::from_millis(2_000)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(100),
            std::time::Duration::from_millis(2_000)
        );
    }
}
