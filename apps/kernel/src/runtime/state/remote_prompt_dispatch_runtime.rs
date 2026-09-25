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

#[derive(Debug, PartialEq, Eq)]
enum RemotePromptRunBindingRecovery {
    Recovered,
    Rejected,
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
        let provider_run_id = remote_execution
            .active_worker_provider_run_id
            .clone()
            .or_else(|| {
                (active_prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Delivered))
                .then(|| {
                    active_prompt
                        .durable_delivery_provider_run_id()
                        .filter(|provider_run_id| !provider_run_id.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
            })?;
        Some((remote_execution, provider_run_id))
    }

    fn recover_remote_prompt_run_binding_from_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_binding: &crate::agent::RemoteAgentBinding,
        provider_run_id: &str,
        event: &crate::transport::relay_peer::RelayPeerEvent,
    ) -> Result<RemotePromptRunBindingRecovery, DaemonError> {
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id,
            home_agent_id,
            provider_run_id: projected_provider_run_id,
            completions,
            ..
        } = event;
        if home_session_id != session_id
            || home_agent_id != agent_id
            || projected_provider_run_id != provider_run_id
        {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        };
        if active_prompt.durable_delivery_phase()
            != Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            || active_prompt.durable_delivery_provider_run_id() != Some(provider_run_id)
            || (!completions.is_empty()
                && !completions.iter().any(|completion| {
                    completion.home_prompt_id.as_deref() == Some(active_prompt.id())
                }))
        {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let Some(current_binding) = agent.remote_execution() else {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        };
        if !same_remote_prompt_worker_binding(current_binding, expected_binding) {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        match current_binding.active_worker_provider_run_id.as_deref() {
            Some(current_provider_run_id) if current_provider_run_id == provider_run_id => {
                return Ok(RemotePromptRunBindingRecovery::Recovered);
            }
            Some(_) => return Ok(RemotePromptRunBindingRecovery::Rejected),
            None => {}
        }
        self.owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                agent_id,
                Some(provider_run_id.to_string()),
            )?;
        let _ = self.owned.session_snapshot(session_id)?;
        Ok(RemotePromptRunBindingRecovery::Recovered)
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
        let recover_missing_run_binding = remote_execution.active_worker_provider_run_id.is_none();
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
            Err(error)
                if remote_prompt_projection_error_should_refresh_binding(
                    recover_missing_run_binding,
                    &error,
                ) =>
            {
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
                    if recover_missing_run_binding {
                        match self.recover_remote_prompt_run_binding_from_projection(
                            session_id,
                            agent_id,
                            &remote_execution,
                            &provider_run_id,
                            &event,
                        )? {
                            RemotePromptRunBindingRecovery::Recovered => {}
                            RemotePromptRunBindingRecovery::Rejected => return Ok(false),
                        }
                    }
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
        let refresh_agent_id = agent_id.to_string();
        let stale_leased_agent_id = stale_binding.leased_agent_id.clone();
        let refresh_stale_provider_run_id = stale_provider_run_id.to_string();
        let rebound = self
            .with_app_side_effect_blocking(move |app| {
                let agent = app.agents().get_agent(&refresh_agent_id)?;
                let Some(current_binding) = agent.remote_execution() else {
                    return Ok(None);
                };
                if current_binding.leased_agent_id != stale_leased_agent_id
                    || current_binding.active_worker_provider_run_id.as_deref()
                        != Some(refresh_stale_provider_run_id.as_str())
                {
                    return Ok(None);
                }
                app.refresh_remote_agent_binding(&refresh_agent_id)
                    .map(Some)
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
            let (prompt, _) = match self
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
            hidden_system_context: active_prompt.hidden_system_context().to_string(),
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
        use super::remote_prompt_owned_state::RemotePromptDispatchSettlement;
        let mut append_retry = 0_u8;
        let settlement = loop {
            match self.owned.settle_remote_dispatch_if_current(
                &dispatch,
                result.as_ref().ok().map(String::as_str),
            ) {
                Err(error)
                    if result.is_ok()
                        && matches!(&error, DaemonError::SessionHistoryFailed { .. })
                        && append_retry < 2 =>
                {
                    append_retry += 1;
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "worker accepted prompt but durable acknowledgement failed; retrying acknowledgement only",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": append_retry,
                            "error": error.to_string(),
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        50 * u64::from(append_retry),
                    ))
                    .await;
                }
                outcome => break outcome?,
            }
        };
        let settled_prompt = match settlement {
            RemotePromptDispatchSettlement::Settled(prompt) => prompt,
            RemotePromptDispatchSettlement::Superseded => return Ok(()),
            RemotePromptDispatchSettlement::BindingChanged(prompt) => {
                let workflow_id = prompt.workflow_run_id().and_then(|run_id| {
                    self.owned
                        .session_store
                        .get_session(&dispatch.session_id)
                        .ok()
                        .and_then(|session| {
                            session
                                .workflow_run(run_id)
                                .map(|run| run.workflow_id().to_string())
                        })
                });
                let message = format!(
                    "Remote delivery of prompt `{}` is uncertain because its worker binding changed before acknowledgement. The prompt remains pending; it was not replayed or cancelled. Check the previous worker's turn before deciding whether to retry.",
                    dispatch.prompt_id,
                );
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    &message,
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "prompt_id": dispatch.prompt_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "delivery_uncertain": true,
                        "prompt_replayed": false,
                    }),
                );
                let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                let merge_key = Some(format!("remote-dispatch-uncertain:{}", dispatch.prompt_id));
                self.owned.fan_out_remote_dispatch_error(
                    &dispatch,
                    &provider_run_id,
                    merge_key.clone(),
                    &message,
                );
                self.owned.append_operational_history_entry_with_context(
                    &SessionHistoryEntry::provider_output(
                        &dispatch.session_id,
                        &provider_run_id,
                        Some(&dispatch.agent_id),
                        crate::terminal::TerminalOutputKind::ProviderError,
                        merge_key,
                        message,
                    )
                    .with_prompt_origin(dispatch.prompt_origin)
                    .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                    crate::history::HistoryEventTurnContext {
                        session_id: Some(dispatch.session_id.clone()),
                        agent_id: Some(dispatch.agent_id.clone()),
                        provider_run_id: Some(provider_run_id),
                        prompt_id: Some(dispatch.prompt_id.clone()),
                        turn_id: Some(dispatch.prompt_id.clone()),
                        workflow_id,
                        workflow_run_id: prompt.workflow_run_id().map(str::to_string),
                        workflow_node_id: prompt.workflow_node_run_id().map(str::to_string),
                        ..Default::default()
                    },
                );
                return Ok(());
            }
        };
        self.owned.provider_process_projection.invalidate();
        let echo_to_all_attachments = settled_prompt.durable_initially_queued() == Some(true);
        let should_start_projection_drain = {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    let _ = owned.session_snapshot(&dispatch.session_id)?;
                    if echo_to_all_attachments {
                        let projected_provider_run_id =
                            crate::provider::projected_leased_provider_run_id(
                                &dispatch.leased_agent_id,
                                &remote_provider_run_id,
                            );
                        owned.echo_promoted_queued_prompt_to_attachments(
                            &dispatch.session_id,
                            &projected_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    } else {
                        owned.echo_prompt_to_other_attachments(
                            &dispatch.session_id,
                            &remote_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    }
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                        None,
                    );
                    Ok(true)
                }
                Err(error) => {
                    let message =
                        format!("Remote prompt dispatch failed after acknowledgement: {error}");
                    let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                    let merge_key = Some(format!("remote-dispatch-error:{}", dispatch.prompt_id));
                    owned.fan_out_remote_dispatch_error(
                        &dispatch,
                        &provider_run_id,
                        merge_key.clone(),
                        &message,
                    );
                    let workflow_id = settled_prompt.workflow_run_id().and_then(|run_id| {
                        owned
                            .session_store
                            .get_session(&dispatch.session_id)
                            .ok()
                            .and_then(|session| {
                                session
                                    .workflow_run(run_id)
                                    .map(|run| run.workflow_id().to_string())
                            })
                    });
                    owned.append_operational_history_entry_with_context(
                        &SessionHistoryEntry::provider_output(
                            &dispatch.session_id,
                            &provider_run_id,
                            Some(&dispatch.agent_id),
                            crate::terminal::TerminalOutputKind::ProviderError,
                            merge_key,
                            message,
                        )
                        .with_prompt_origin(dispatch.prompt_origin)
                        .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                        crate::history::HistoryEventTurnContext {
                            workflow_id,
                            session_id: Some(dispatch.session_id.clone()),
                            agent_id: Some(dispatch.agent_id.clone()),
                            provider_run_id: Some(provider_run_id.clone()),
                            prompt_id: Some(dispatch.prompt_id.clone()),
                            turn_id: Some(dispatch.prompt_id.clone()),
                            workflow_run_id: settled_prompt.workflow_run_id().map(str::to_string),
                            workflow_node_id: settled_prompt
                                .workflow_node_run_id()
                                .map(str::to_string),
                            ..Default::default()
                        },
                    );
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                        Some(error.to_string()),
                    );
                    owned.record_cancelled_prompt_settlement(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &settled_prompt,
                        settled_prompt.durable_delivery_provider_run_id(),
                    );
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
            let (prompt, _) = match state
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
            match result {
                Ok(remote_provider_run_id) => {
                    if let Err(error) = state
                        .finish_remote_prompt_dispatch(dispatch.clone(), Ok(remote_provider_run_id))
                        .await
                    {
                        let message = format!(
                            "Worker accepted prompt `{}`, but the home kernel could not record its acknowledgement: {error}. Delivery is uncertain; do not replay this prompt without checking the worker.",
                            dispatch.prompt_id,
                        );
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            &message,
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "worker_kernel_id": dispatch.worker_kernel_id,
                                "leased_agent_id": dispatch.leased_agent_id,
                                "delivery_uncertain": true,
                            }),
                        );
                        state.owned.fan_out_remote_dispatch_error(
                            &dispatch,
                            &format!("remote-dispatch:{}", dispatch.prompt_id),
                            Some(format!(
                                "remote-dispatch-ack-persist-failed:{}",
                                dispatch.prompt_id
                            )),
                            &message,
                        );
                    }
                }
                Err(error) => {
                    if let Err(settlement_error) = state
                        .finish_remote_prompt_dispatch(dispatch, Err(error))
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote prompt failure could not be settled",
                            serde_json::json!({"error": settlement_error.to_string()}),
                        );
                    }
                }
            }
        });
    }
}

fn same_remote_prompt_worker_binding(
    current: &crate::agent::RemoteAgentBinding,
    expected: &crate::agent::RemoteAgentBinding,
) -> bool {
    current.worker_kernel_id == expected.worker_kernel_id
        && current.worker_machine_id == expected.worker_machine_id
        && current.execution_lease_id == expected.execution_lease_id
        && current.leased_agent_id == expected.leased_agent_id
        && current.relay_url == expected.relay_url
        && current.relay_token == expected.relay_token
        && current.relay_peer_protocol_version == expected.relay_peer_protocol_version
}

fn remote_prompt_projection_error_should_refresh_binding(
    recovering_missing_run_binding: bool,
    error: &DaemonError,
) -> bool {
    !recovering_missing_run_binding && remote_prompt_error_should_refresh_binding(error)
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

    #[tokio::test]
    async fn recovered_queued_remote_prompt_echo_uses_durable_queue_origin() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new("workspace-1", "worktree-1")).unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(session.id(), "queued-recovery-source", crate::attachment::ClientCapabilityLevel::FullTerminal)).unwrap();
        app.agents.bind_remote_execution(agent.id(), crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-1".into(), worker_machine_id: "machine-1".into(),
            execution_lease_id: "lease-1".into(), leased_agent_id: "leased-agent-1".into(),
            active_worker_provider_run_id: None, relay_url: None, relay_token: None,
            relay_peer_protocol_version: Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION),
        }).unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime.owned.submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
            session_id: session.id().to_string(),
            prompt: crate::session::PromptQueueItem::new("pending", attachment.id(), agent.id(), "queued recovery prompt", crate::session::PromptStatus::Queued),
            force_queue: true, refresh_projection: true,
        }).unwrap().unwrap();
        let promoted = runtime.owned.advance_next_queued_remote_prompt_dispatch(session.id(), agent.id()).unwrap().unwrap();
        let original_dispatch = promoted.remote_dispatch.unwrap();
        let session = runtime.owned.session_store.get_session(session.id()).unwrap();
        let active = runtime.owned.prompt_state_owner.active_prompt_for_agent(&session, agent.id()).unwrap();
        assert_eq!(active.durable_initially_queued(), Some(true));
        assert!(active.durable_operation_id().is_none(), "ordinary queue origins must not require a command operation ID");
        let private = crate::session::DurablePromptPrivateState::from_prompt(session.id(), &active).unwrap();
        let private: crate::session::DurablePromptPrivateState = serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let mut restored: crate::session::PromptQueueItem = serde_json::from_value(serde_json::to_value(&active).unwrap()).unwrap();
        assert_eq!(restored.durable_initially_queued(), None);
        restored.restore_durable_private_state(&private);
        assert!(runtime.owned.prompt_state_owner.replace_active_prompt_if_matches(&session, agent.id(), &active, restored));
        // Discard the original dispatch intent, as happens on restart. Recovery
        // must reconstruct its echo policy from durable prompt ownership alone.
        drop(original_dispatch);
        let recovered = runtime.remote_prompt_recovery_dispatch(&runtime.owned.agent_store.get_agent(agent.id()).unwrap()).unwrap().unwrap();
        assert_eq!(recovered.prompt_id, active.id());
        assert!(runtime.owned.terminal_stream.drain_output_records(session.id(), attachment.id()).iter().all(|record| record.kind != crate::terminal::TerminalOutputKind::PromptEcho));
        runtime.finish_remote_prompt_dispatch(recovered, Ok("worker-run-recovered".into())).await.unwrap();
        let echoes = runtime.owned.terminal_stream.drain_output_records(session.id(), attachment.id()).into_iter().filter(|record| record.kind == crate::terminal::TerminalOutputKind::PromptEcho).collect::<Vec<_>>();
        assert_eq!(echoes.len(), 1, "recovered queued prompt must echo once to its submitting attachment");
        assert_eq!(echoes[0].prompt_id.as_deref(), Some(active.id()));
        assert_eq!(echoes[0].provider_run_id, crate::provider::projected_leased_provider_run_id("leased-agent-1", "worker-run-recovered"));
    }

    fn completed_worker_projection(
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        home_prompt_id: &str,
    ) -> crate::transport::relay_peer::RelayPeerEvent {
        crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: session_id.to_string(),
            home_agent_id: agent_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            provider_run: None,
            prompts: Vec::new(),
            output_chunks: Vec::new(),
            notices: Vec::new(),
            completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-completed".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(home_prompt_id.to_string()),
                provider_termination: None,
            }],
        }
    }

    fn actual_worker_output_then_completion(
        home_session_id: &str,
        home_agent_id: &str,
        home_prompt_id: &str,
    ) -> (
        crate::agent::RemoteAgentBinding,
        String,
        crate::transport::relay_peer::RelayPeerEvent,
        crate::transport::relay_peer::RelayPeerEvent,
    ) {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut worker = DaemonApp::bootstrap(config).expect("worker bootstrap should succeed");
        let lease = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .create_execution_lease(
                "home-kernel-live-recovery",
                home_session_id,
                home_agent_id,
                false,
                "owner-live-recovery",
            )
            .expect("worker lease should be created");
        let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("worker leased agent should be created");
        let (provider_run_id, outcome) = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "remote prompt\n",
                Vec::new(),
                None,
                Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: home_session_id.to_string(),
                    home_agent_id: home_agent_id.to_string(),
                    home_prompt_id: home_prompt_id.to_string(),
                    home_turn_id: home_prompt_id.to_string(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: "remote prompt".to_string(),
                }),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("worker prompt should submit");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));
        crate::app::RemoteLeaseRuntime::new(&mut worker)
            .set_leased_agent_provider_for_test(&leased_agent.id, "codex");
        let launch_request = crate::provider::LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            "codex",
            "codex",
            "default",
            "gpt-5.4",
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_client_interface(crate::provider::ProviderClientInterface::Chariox);
        let mut running_provider = crate::provider::RuntimeProviderRun::new(
            &provider_run_id,
            &launch_request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "codex:codex:gpt-5.4".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        running_provider.mark_running();
        worker.providers_mut().insert_run_for_test(running_provider);
        worker.fan_out_output_for_agent(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"LIVE_RECOVERY_OUTPUT_ONCE",
        );
        worker.record_notice_for_agent(
            &leased_agent.backing_session_id,
            Some(&provider_run_id),
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "LIVE_RECOVERY_NOTICE_ONCE",
        );
        let output_event = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker output drain should succeed")
            .expect("worker output should project")
            .1;
        if let Some((_, duplicate_event)) = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker duplicate drain should succeed")
        {
            let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                output_chunks,
                notices,
                ..
            } = duplicate_event;
            assert!(output_chunks.is_empty(), "worker output must not repeat");
            assert!(notices.is_empty(), "worker notice must not repeat");
        }
        worker
            .complete_active_prompt(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
                Some(&provider_run_id),
            )
            .expect("worker prompt should settle");
        let completion_event = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker completion drain should succeed")
            .expect("worker completion should project")
            .1;
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-kernel-live-recovery".to_string(),
            worker_machine_id: "worker-machine-live-recovery".to_string(),
            execution_lease_id: lease.id,
            leased_agent_id: leased_agent.id,
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        (binding, provider_run_id, output_event, completion_event)
    }

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
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
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
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
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
    async fn delivered_remote_prompt_uses_durable_run_for_live_projection_drain() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-live-recovery",
                "worktree-live-recovery",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-live-recovery",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-live-recovery",
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
        let (binding, provider_run_id, output_event, completion_event) =
            actual_worker_output_then_completion(session.id(), agent.id(), &prompt_id);
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            provider_run,
            output_chunks,
            notices,
            completions,
            ..
        } = &output_event;
        assert!(provider_run.is_some(), "worker status should project");
        assert_eq!(output_chunks.len(), 1);
        assert_eq!(notices, &["LIVE_RECOVERY_NOTICE_ONCE"]);
        assert!(completions.is_empty());
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            notices,
            completions,
            ..
        } = &completion_event;
        assert!(
            output_chunks.is_empty(),
            "worker output must be deduplicated"
        );
        assert!(notices.is_empty(), "worker notice must be consumed once");
        assert_eq!(completions.len(), 1);
        assert_eq!(
            completions[0].home_prompt_id.as_deref(),
            Some(prompt_id.as_str())
        );
        app.agents
            .bind_remote_execution(agent.id(), binding.clone())
            .expect("agent should bind to the worker projection source");
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.clone()),
            None,
        )
        .expect("delivery metadata should persist");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let (drain_binding, drain_provider_run_id) = runtime
            .remote_prompt_projection_drain_target(session.id(), agent.id())
            .expect("live drain must recover the durable delivered worker run");

        assert_eq!(drain_binding, binding);
        assert_eq!(drain_provider_run_id, provider_run_id);
        let mismatched_prompt = completed_worker_projection(
            session.id(),
            agent.id(),
            &provider_run_id,
            "other-home-prompt",
        );
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &mismatched_prompt,
                )
                .expect("mismatched prompt projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        let stale_run =
            completed_worker_projection(session.id(), agent.id(), "provider-run-stale", &prompt_id);
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &stale_run,
                )
                .expect("stale run projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &output_event,
                )
                .expect("output-only worker projection should recover the binding"),
            RemotePromptRunBindingRecovery::Recovered,
        );
        runtime
            .project_remote_runtime_projection_event(output_event)
            .await
            .expect("output-only worker projection should reach the home session");
        let first_output = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            first_output
                .iter()
                .filter(|record| record.bytes == b"LIVE_RECOVERY_OUTPUT_ONCE")
                .count(),
            1
        );
        let first_notices = runtime
            .owned
            .terminal_stream
            .drain_notice_records(session.id(), attachment.id());
        assert_eq!(
            first_notices
                .iter()
                .filter(|record| record.message == "LIVE_RECOVERY_NOTICE_ONCE")
                .count(),
            1
        );
        let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
            &binding.leased_agent_id,
            &provider_run_id,
        );
        assert!(runtime
            .owned
            .provider_run_projection
            .get(&projected_provider_run_id)
            .is_some());
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("home session should remain available"),
                agent.id(),
            )
            .is_some());

        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    execution_lease_id: "lease-other".to_string(),
                    leased_agent_id: "leased-agent-other".to_string(),
                    ..binding.clone()
                },
            )
            .expect("test should replace the current lease binding");
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &completion_event,
                )
                .expect("other lease projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    active_worker_provider_run_id: Some(provider_run_id.clone()),
                    ..binding.clone()
                },
            )
            .expect("test should restore the recovered worker binding");

        runtime
            .project_remote_runtime_projection_event(completion_event)
            .await
            .expect("worker completion should settle the home prompt");
        assert!(runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .is_empty());
        assert!(runtime
            .owned
            .terminal_stream
            .drain_notice_records(session.id(), attachment.id())
            .is_empty());
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("home session should remain available"),
                agent.id(),
            )
            .is_none());
    }

    #[tokio::test]
    async fn settlement_projection_order_remote_completion_publishes_bound_idle_snapshot_after_completion_event(
    ) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-settlement-projection-order-remote",
                "worktree-settlement-projection-order-remote",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-settlement-projection-order-remote",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-settlement-projection-order-remote",
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
        let (binding, provider_run_id, output_event, completion_event) =
            actual_worker_output_then_completion(session.id(), agent.id(), &prompt_id);
        let home_binding = crate::agent::RemoteAgentBinding {
            active_worker_provider_run_id: Some(provider_run_id.clone()),
            ..binding.clone()
        };
        app.agents
            .bind_remote_execution(agent.id(), home_binding)
            .expect("home agent should bind to the worker projection source");
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.clone()),
            None,
        )
        .expect("delivery metadata should persist");

        // This is a real run binding for flow-control, not a manually inserted active turn. The
        // run is ended only after the pre-completion snapshot so the public watch cannot invoke a
        // second local PTY settlement while the worker completion is being projected.
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "managed-dev-stub",
            "managed-dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent.id());
        let mut home_run = crate::provider::RuntimeProviderRun::new(
            &provider_run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "remote-worker-settlement-projection-order".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        home_run.mark_running();
        app.providers_mut().insert_run_for_test(home_run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(provider_run_id.clone()))
            .expect("home run should be selected for the pre-completion snapshot");
        app.update_provider_run_projection(home_run.clone());
        crate::transport::flow_control::note_prompt_started(&mut app, &provider_run_id);

        let initial_snapshot =
            crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(
                &mut app,
                session.id(),
                0,
            )
            .expect("public pre-completion snapshot should be available");
        let initial_activity = initial_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be present before remote output");
        assert_eq!(
            initial_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working
        );
        assert!(initial_activity.busy);
        assert_eq!(initial_activity.active_prompt_count, 1);
        assert!(initial_activity.active_turn.is_some());

        let mut ended_home_run = home_run;
        ended_home_run.mark_ended();
        app.providers_mut()
            .insert_run_for_test(ended_home_run.clone());
        app.update_provider_run_projection(ended_home_run);

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .project_remote_runtime_projection_event(output_event)
            .await
            .expect("worker output projection should reach the home session");
        let output_records = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            output_records
                .iter()
                .filter(|record| record.bytes == b"LIVE_RECOVERY_OUTPUT_ONCE")
                .count(),
            1,
            "the remote callback must deliver the worker output before completion"
        );
        let output_snapshot = {
            let mut app = runtime.app.lock().await;
            crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(
                &mut app,
                session.id(),
                0,
            )
            .expect("public output snapshot should be available")
        };
        let output_activity = output_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should remain present after output");
        assert_eq!(
            output_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working,
            "a completion projection must not publish idle before its completion callback"
        );
        assert!(output_activity.busy);
        assert_eq!(output_activity.active_prompt_count, 1);
        assert!(output_activity.active_turn.is_some());
        let projection_before_completion = runtime.owned.session_projection.change_sequence();

        runtime
            .project_remote_runtime_projection_event(completion_event)
            .await
            .expect("worker completion projection should settle the home prompt");
        assert!(
            runtime.owned.session_projection.change_sequence() > projection_before_completion,
            "remote completion must invalidate the public session projection"
        );

        // This is the same public watch boundary used by the transport loop: completion records
        // are drained before the activity snapshot is projected. Requiring both in this result
        // catches an idle snapshot emitted before the completion event, not only eventual idle.
        let watch_result = {
            let mut app = runtime.app.lock().await;
            crate::runtime_transport::watch_subscription_state(
                &mut app,
                session.id(),
                attachment.id(),
                true,
                Some(output_snapshot.clone()),
                0,
            )
        };
        let crate::runtime_transport::WatchResult::Ok {
            records,
            completions,
            workflow_run_updates,
            snapshot,
            ..
        } = watch_result
        else {
            panic!("remote settlement subscription should remain available");
        };
        assert!(records.is_empty(), "remote output was already observed");
        assert!(
            workflow_run_updates.is_empty(),
            "the non-workflow settlement should not select a workflow-run event instead of the public activity/snapshot path"
        );
        assert_eq!(
            completions.len(),
            1,
            "the worker completion callback must expose exactly one public completion"
        );
        assert_eq!(completions[0].provider_run_id, provider_run_id);
        assert_eq!(
            completions[0].agent_id.as_deref(),
            Some(agent.id()),
            "the completion event must retain the remote run's home agent binding"
        );
        let settled_snapshot = snapshot
            .as_ref()
            .clone()
            .expect("remote settlement should carry a changed public snapshot");
        let settled_activity = settled_snapshot
            .agent_activity
            .get(agent.id())
            .expect("settled public snapshot should include the remote agent");
        assert_eq!(
            settled_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Idle
        );
        assert!(!settled_activity.busy);
        assert_eq!(settled_activity.active_prompt_count, 0);
        assert!(settled_activity.active_turn.is_none());
        // The public subscription emits a narrow activity delta when the session permits one.
        // Otherwise its actual transport loop falls back to the snapshot payload above. Check
        // the same selection boundary without manufacturing a fallback event in this test.
        match crate::transport::kernel_protocol::agent_activity_changed_event(
            &settled_snapshot,
            Some(&output_snapshot),
        ) {
            Some(crate::transport::kernel_protocol::KernelEvent::AgentActivityChanged {
                agent_activity,
                ..
            }) => {
                let activity = agent_activity
                    .get(agent.id())
                    .expect("idle event should include the settled remote agent");
                assert_eq!(
                    activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!activity.busy);
                assert_eq!(activity.active_prompt_count, 0);
                assert!(activity.active_turn.is_none());
            }
            Some(event) => panic!("unexpected public remote settlement event: {event:?}"),
            None => {
                assert!(
                    crate::transport::kernel_protocol::provider_run_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no activity delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::session_metadata_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no metadata delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::runtime_interactions_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no interaction delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::workflow_run_updated_events(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_empty()
                        && !crate::transport::kernel_protocol::workflow_run_only_changed(
                            &settled_snapshot,
                            Some(&output_snapshot),
                        ),
                    "the public transport must use the full snapshot when no narrow projection applies"
                );
                let fallback_activity = settled_snapshot
                    .agent_activity
                    .get(agent.id())
                    .expect("full snapshot fallback should include the settled remote agent");
                assert_eq!(
                    fallback_activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!fallback_activity.busy);
                assert_eq!(fallback_activity.active_prompt_count, 0);
                assert!(fallback_activity.active_turn.is_none());
            }
        }
    }

    #[test]
    fn live_run_binding_recovery_never_refreshes_the_lease() {
        let missing_lease = DaemonError::ExecutionLeaseNotFound {
            lease_id: "lease-2".to_string(),
        };

        assert!(!remote_prompt_projection_error_should_refresh_binding(
            true,
            &missing_lease,
        ));
        assert!(remote_prompt_projection_error_should_refresh_binding(
            false,
            &missing_lease,
        ));
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
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
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
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
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

    async fn superseded_remote_dispatch_fixture(
        rebind: bool,
    ) -> (KernelRuntimeState, crate::app::KernelRemotePromptDispatch) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-late-remote-dispatch",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-a".to_string(),
            worker_machine_id: "machine-a".to_string(),
            execution_lease_id: "lease-a".to_string(),
            leased_agent_id: "leased-agent-a".to_string(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        app.agents
            .bind_remote_execution(agent.id(), binding.clone())
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .ensure_managed_activity_tracking("remote-dispatch-settlement-fixture")
            .unwrap();
        let submit = |text: &str| {
            runtime
                .owned
                .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                    session_id: session.id().to_string(),
                    prompt: crate::session::PromptQueueItem::new(
                        "pending-fixture",
                        attachment.id(),
                        agent.id(),
                        text,
                        crate::session::PromptStatus::Queued,
                    ),
                    force_queue: false,
                    refresh_projection: true,
                })
                .unwrap()
                .unwrap()
                .remote_dispatch
                .unwrap()
        };
        let stale_dispatch = submit("prompt A");
        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &stale_dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
        let cancelled = runtime
            .owned
            .cancel_active_prompt_only(session.id(), agent.id())
            .unwrap();
        assert_eq!(cancelled.id(), stale_dispatch.prompt_id);
        let mut successor_binding = binding;
        if rebind {
            successor_binding.worker_kernel_id = "worker-b".to_string();
            successor_binding.worker_machine_id = "machine-b".to_string();
            successor_binding.execution_lease_id = "lease-b".to_string();
            successor_binding.leased_agent_id = "leased-agent-b".to_string();
        }
        successor_binding.active_worker_provider_run_id = Some("worker-run-b".to_string());
        runtime
            .owned
            .agent_store
            .bind_remote_execution(agent.id(), successor_binding)
            .unwrap();
        let successor = submit("prompt B");
        assert_ne!(successor.prompt_id, stale_dispatch.prompt_id);
        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &successor.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("worker-run-b".to_string()),
                None,
            )
            .unwrap();
        runtime.owned.session_snapshot(session.id()).unwrap();
        (runtime, stale_dispatch)
    }

    async fn assert_late_remote_dispatch_preserves_successor(
        result: Result<String, DaemonError>,
        rebind: bool,
    ) {
        let (runtime, stale_dispatch) = superseded_remote_dispatch_fixture(rebind).await;
        let session_id = stale_dispatch.session_id.clone();
        let agent_id = stale_dispatch.agent_id.clone();
        let before_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();

        // Exercise the actual async dispatch settlement entry point after A has
        // lost ownership. Whether the stale result is reported or ignored is
        // secondary; it must not mutate B's state or transcript.
        let _ = runtime
            .finish_remote_prompt_dispatch(stale_dispatch, result)
            .await;

        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let after_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let after_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&after_session, &agent_id);
        let after_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        assert_eq!(
            after_prompt.as_ref(),
            Some(&before_prompt),
            "late A settlement must preserve B's active prompt"
        );
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution(),
            "late A settlement must preserve B's worker binding"
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            after_history, before_history,
            "late A settlement must not append an error to B's transcript"
        );
        assert_eq!(
            after_prompt.unwrap().durable_delivery_provider_run_id(),
            Some("worker-run-b")
        );
    }

    #[tokio::test]
    async fn late_remote_dispatch_success_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(Ok("worker-run-a-late".to_string()), true)
            .await;
    }

    async fn assert_duplicate_remote_settlement_is_inert(result: Result<String, DaemonError>) {
        let (runtime, mut dispatch) = superseded_remote_dispatch_fixture(false).await;
        let session = runtime
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .unwrap();
        assert_eq!(
            before_prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        dispatch.prompt_id = before_prompt.id().to_string();
        dispatch.prompt = before_prompt.prompt().to_string();
        {
            let mut app = runtime.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    &dispatch.session_id,
                    "duplicate-dispatch-observer",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap();
        }
        let before_agent = runtime
            .owned
            .agent_store
            .get_agent(&dispatch.agent_id)
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&dispatch.session_id, Some(&dispatch.agent_id))
            .unwrap();
        let before_terminal_count = runtime.owned.terminal_stream.output_records().len();
        let before_terminal_sequence = runtime.owned.terminal_stream.change_sequence();
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let _ = runtime
            .finish_remote_prompt_dispatch(dispatch, result)
            .await;
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id),
            Some(before_prompt)
        );
        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution()
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            runtime
                .owned
                .operational_history_store
                .load_session_history_entries(&session_id, Some(&agent_id))
                .unwrap(),
            before_history
        );
        assert_eq!(
            runtime.owned.terminal_stream.output_records().len(),
            before_terminal_count
        );
        assert_eq!(
            runtime.owned.terminal_stream.change_sequence(),
            before_terminal_sequence,
            "duplicate settlement must not echo or notify terminal output"
        );
    }

    #[tokio::test]
    async fn duplicate_remote_dispatch_success_after_delivered_is_inert() {
        assert_duplicate_remote_settlement_is_inert(Ok("worker-run-b".to_string())).await;
    }

    async fn pending_remote_settlement_fixture(
    ) -> (KernelRuntimeState, crate::app::KernelRemotePromptDispatch) {
        let (runtime, mut dispatch) = superseded_remote_dispatch_fixture(false).await;
        let session = runtime
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .unwrap();
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .unwrap();
        dispatch.prompt_id = prompt.id().to_string();
        dispatch.prompt = prompt.prompt().to_string();
        runtime
            .owned
            .mark_active_prompt_delivery(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
        (runtime, dispatch)
    }

    #[tokio::test]
    async fn remote_dispatch_durable_append_failure_rolls_back_then_manual_settlement_succeeds() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        let before_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        let before_terminal = runtime.owned.terminal_stream.change_sequence();
        let before_activity = runtime.managed_activity_change_sequence();
        let event_count = || {
            runtime
                .owned
                .durable_state_store
                .load_events_by_kind("session.prompt_state.updated")
                .unwrap()
                .len()
        };
        let before_events = event_count();
        let connection =
            rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_remote_settlement_append BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'session.prompt_state.updated'
             BEGIN SELECT RAISE(FAIL, 'injected remote settlement append failure'); END;",
            )
            .unwrap();
        let result = runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("worker-run-after-retry".to_string()))
            .await;
        connection
            .execute_batch("DROP TRIGGER fail_remote_settlement_append;")
            .unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("injected remote settlement append failure"));
        let after_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&after_session, &agent_id),
            Some(before_prompt.clone())
        );
        assert_eq!(
            after_session.active_prompt_for_agent(&agent_id),
            Some(&before_prompt)
        );
        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution()
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            runtime
                .owned
                .operational_history_store
                .load_session_history_entries(&session_id, Some(&agent_id))
                .unwrap(),
            before_history
        );
        assert_eq!(
            runtime.owned.terminal_stream.change_sequence(),
            before_terminal
        );
        assert_eq!(runtime.managed_activity_change_sequence(), before_activity);
        assert_eq!(event_count(), before_events);

        let retry = runtime
            .remote_prompt_recovery_dispatch(&after_agent)
            .unwrap()
            .unwrap();
        assert_eq!(retry.prompt_id, prompt_id);
        runtime
            .finish_remote_prompt_dispatch(retry, Ok("worker-run-after-retry".to_string()))
            .await
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let delivered = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(delivered.id(), prompt_id);
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            delivered.durable_delivery_provider_run_id(),
            Some("worker-run-after-retry")
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .unwrap()
                .remote_execution()
                .unwrap()
                .active_worker_provider_run_id
                .as_deref(),
            Some("worker-run-after-retry")
        );
        assert_eq!(
            event_count(),
            before_events + 1,
            "retry must publish exactly one prompt-state settlement"
        );
    }

    #[tokio::test]
    async fn acknowledged_remote_dispatch_retries_only_durable_settlement() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_events = runtime
            .owned
            .durable_state_store
            .load_events_by_kind("session.prompt_state.updated")
            .unwrap()
            .len();
        let connection =
            rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_first_remote_ack BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'session.prompt_state.updated'
             BEGIN SELECT RAISE(FAIL, 'injected transient acknowledgement failure'); END;",
            )
            .unwrap();
        let settling = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                runtime
                    .finish_remote_prompt_dispatch(dispatch, Ok("worker-run-accepted".to_string()))
                    .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        connection
            .execute_batch("DROP TRIGGER fail_first_remote_ack;")
            .unwrap();
        settling.await.unwrap().unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let delivered = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(delivered.id(), prompt_id);
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            delivered.durable_delivery_provider_run_id(),
            Some("worker-run-accepted")
        );
        assert_eq!(
            runtime
                .owned
                .durable_state_store
                .load_events_by_kind("session.prompt_state.updated")
                .unwrap()
                .len(),
            before_events + 1,
            "retry must persist exactly one acknowledgement for the already accepted run"
        );
    }

    #[tokio::test]
    async fn remote_dispatch_success_preserves_cancellation_in_flight() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        runtime
            .owned
            .begin_remote_prompt_cancellation(
                &session_id,
                &agent_id,
                &dispatch.source_attachment_id,
            )
            .unwrap();
        runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("worker-run-cancelling".to_string()))
            .await
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(prompt.status(), crate::session::PromptStatus::Cancelling);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            prompt.durable_delivery_provider_run_id(),
            Some("worker-run-cancelling")
        );
        assert_eq!(
            session.active_prompt_for_agent(&agent_id).unwrap().status(),
            crate::session::PromptStatus::Cancelling
        );
    }

    #[tokio::test]
    async fn current_remote_dispatch_binding_change_is_not_silently_settled() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        assert_eq!(before_prompt.id(), prompt_id);
        assert!(before_prompt.delivery_pending());
        let mut replacement = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .clone();
        replacement.worker_kernel_id = "replacement-worker".to_string();
        replacement.leased_agent_id = "replacement-leased-agent".to_string();
        replacement.execution_lease_id = "replacement-lease".to_string();
        replacement.active_worker_provider_run_id = None;
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent_id, replacement.clone())
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        let source_attachment_id = dispatch.source_attachment_id.clone();
        let before_terminal = runtime
            .owned
            .terminal_stream
            .attachment_change_sequence(&session_id, &source_attachment_id);

        assert!(matches!(
            runtime.owned.settle_remote_dispatch_if_current(&dispatch, Some("old-worker-run")).unwrap(),
            super::super::remote_prompt_owned_state::RemotePromptDispatchSettlement::BindingChanged(ref prompt)
                if prompt.id() == prompt_id && prompt.delivery_pending()
        ));
        let result = runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("old-worker-run".to_string()))
            .await;

        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id),
            Some(before_prompt)
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .unwrap()
                .remote_execution(),
            Some(&replacement)
        );
        let after_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        assert_eq!(after_history.len(), before_history.len() + 1);
        let warning = after_history
            .iter()
            .find(|entry| {
                entry.merge_key.as_deref()
                    == Some(format!("remote-dispatch-uncertain:{prompt_id}").as_str())
            })
            .expect("delivery-uncertain diagnostic must be durable");
        assert!(warning.text.contains("remains pending"));
        assert!(warning.text.contains("not replayed or cancelled"));
        assert!(
            runtime
                .owned
                .terminal_stream
                .attachment_change_sequence(&session_id, &source_attachment_id)
                > before_terminal
        );
        assert!(
            runtime
                .remote_prompt_projection_drain_target(&session_id, &agent_id)
                .is_none(),
            "ordinary projection drain cannot reconcile pending delivery after a binding change"
        );
        assert!(result.is_ok());
        assert!(
            runtime
                .owned
                .remote_prompt_recoveries
                .lock()
                .unwrap()
                .is_empty(),
            "uncertain delivery must not replay the original prompt"
        );
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_after_delivered_is_inert() {
        assert_duplicate_remote_settlement_is_inert(Err(DaemonError::LocalTransport {
            operation: "submit remote prompt",
            message: "late transport error after delivery".to_string(),
        }))
        .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(
            Err(DaemonError::LocalTransport {
                operation: "submit remote prompt",
                message: "late rejection of prompt A".to_string(),
            }),
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_success_on_same_binding_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(Ok("worker-run-a-late".to_string()), false)
            .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_on_same_binding_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(
            Err(DaemonError::LocalTransport {
                operation: "submit remote prompt",
                message: "late rejection of prompt A on the same binding".to_string(),
            }),
            false,
        )
        .await;
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
