//! Per-agent remote-prompt recovery claims and durable replay policy.

use super::remote_prompt_receipt_reconciliation_runtime::remote_prompt_receipt_is_rejected;
use super::remote_prompt_worker_submission_runtime::{
    persist_remote_prompt_reconciliation_pending, remote_prompt_error_is_reconciliation_pending,
    remote_prompt_reconciliation_pending, submit_remote_prompt_to_worker_with_binding_refresh,
};
use super::*;

// Duplicate starts advance the generation so release and restart are one atomic decision.
pub(super) struct RemotePromptAgentClaim {
    key: (String, String),
    claims: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
    seen_generation: u64,
    cancellation_sends: BTreeSet<(String, String)>,
    released: bool,
}

impl RemotePromptAgentClaim {
    pub(super) fn try_acquire(
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
            cancellation_sends: BTreeSet::new(),
            released: false,
        })
    }

    pub(super) fn cancellation_was_sent(&self, prompt_id: &str, provider_run_id: &str) -> bool {
        self.cancellation_sends
            .contains(&(prompt_id.to_string(), provider_run_id.to_string()))
    }

    pub(super) fn mark_cancellation_sent(&mut self, prompt_id: &str, provider_run_id: &str) {
        self.cancellation_sends
            .insert((prompt_id.to_string(), provider_run_id.to_string()));
    }

    pub(super) fn release_or_restart(&mut self) -> bool {
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
    pub(super) fn try_claim_remote_prompt_cancellation_send(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RemotePromptAgentClaim> {
        RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            session_id,
            agent_id,
        )
    }

    pub(super) async fn recover_remote_prompt_after_kernel_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        delivery_phase: Option<crate::session::DurablePromptDeliveryPhase>,
        delivery_provider_run_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(session_id)?;
        let active_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id);
        if agent.remote_execution().is_some()
            && active_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling)
        {
            let prompt = active_prompt.as_ref().expect("checked above");
            match prompt.durable_delivery_phase() {
                Some(crate::session::DurablePromptDeliveryPhase::Accepted)
                    if delivery_phase
                        != Some(crate::session::DurablePromptDeliveryPhase::Dispatching) =>
                {
                    // Accepted is persisted before transport starts. On restart no worker run can
                    // belong to this prompt, so finish its durable cancellation without replay.
                    self.owned
                        .finalize_remote_prompt_cancellation_after_worker_settled(
                            session_id,
                            agent_id,
                            prompt.source_attachment_id(),
                        )?;
                    return Ok(true);
                }
                Some(crate::session::DurablePromptDeliveryPhase::Delivered) => {
                    let Some(dispatch) = self.remote_prompt_recovery_dispatch(&agent)? else {
                        return Ok(true);
                    };
                    if let Err(error) = self
                        .resume_delivered_remote_prompt_cancellation_after_restart(
                            &dispatch,
                            prompt.durable_delivery_provider_run_id(),
                        )
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "restart kept delivered remote cancellation held because its worker receipt was not reconciled",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "worker_kernel_id": dispatch.worker_kernel_id,
                                "leased_agent_id": dispatch.leased_agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    return Ok(true);
                }
                Some(crate::session::DurablePromptDeliveryPhase::Dispatching) => {}
                Some(crate::session::DurablePromptDeliveryPhase::Accepted) => {
                    // Conflicting persisted phase evidence cannot prove the worker was untouched.
                    // Keep cancellation held rather than settling or replaying this prompt.
                    return Ok(true);
                }
                None => {
                    // Older or incomplete state has no proof tying this cancellation to a worker
                    // run. Keep the same prompt held; never redispatch or cancel a lease by guess.
                    return Ok(true);
                }
            }
        }
        if agent.remote_execution().is_some()
            && active_prompt.as_ref().is_some_and(|prompt| {
                prompt.durable_delivery_reconciliation_pending()
                    || prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                    || delivery_phase
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            })
        {
            let Some(dispatch) = self.remote_prompt_recovery_dispatch(&agent)? else {
                return Ok(true);
            };
            match self.reconcile_remote_prompt_worker_receipt(&dispatch).await {
                Ok(provider_run_id) => {
                    if let Err(error) = self
                        .finish_remote_prompt_dispatch(dispatch.clone(), Ok(provider_run_id))
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "restart verified worker receipt but could not settle home prompt; retrying read-only reconciliation",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "error": error.to_string(),
                            }),
                        );
                        self.spawn_remote_prompt_receipt_reconciliation(dispatch);
                    }
                }
                Err(error) => {
                    if remote_prompt_receipt_is_rejected(&error) {
                        if let Err(settle_error) = self
                            .settle_verified_remote_prompt_rejection(&dispatch, error)
                            .await
                        {
                            crate::logging::warn_with_fields(
                                "daemon.remote_prompt_dispatch",
                                "verified worker rejection could not be settled; retrying read-only reconciliation",
                                serde_json::json!({
                                    "session_id": dispatch.session_id,
                                    "agent_id": dispatch.agent_id,
                                    "prompt_id": dispatch.prompt_id,
                                    "error": settle_error.to_string(),
                                }),
                            );
                            self.spawn_remote_prompt_receipt_reconciliation(dispatch);
                        }
                        return Ok(true);
                    }
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "restart kept remote prompt held because its worker receipt was not reconciled",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "error": error.to_string(),
                        }),
                    );
                    self.spawn_remote_prompt_receipt_reconciliation(dispatch);
                }
            }
            return Ok(true);
        }
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
        let Some(mut dispatch) =
            self.remote_prompt_recovery_dispatch_for_phase(&agent, delivery_phase)?
        else {
            return Ok(false);
        };
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;
        self.spawn_remote_prompt_dispatch(dispatch);
        Ok(true)
    }

    pub(super) fn spawn_stale_remote_prompt_recovery(
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
                if remote_prompt_error_is_reconciliation_pending(&error) {
                    if let Ok(agent) = state.owned.agent_store.get_agent(&agent_id) {
                        if let Ok(Some(dispatch)) = state.remote_prompt_recovery_dispatch(&agent) {
                            if remote_prompt_reconciliation_pending(&state, &dispatch)
                                .unwrap_or(false)
                            {
                                state
                                    .retry_remote_prompt_receipt_with_claim(&dispatch)
                                    .await;
                            }
                        }
                    }
                }
            }
            state
                .run_remote_prompt_dispatch_with_claim(claim, None)
                .await;
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
        let session = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling)
        {
            // Cancellation owns the exact old worker run. A stale projection must not refresh
            // its binding and replay the prompt onto a replacement worker.
            return Ok(());
        }
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
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch_for_phase(&rebound, None)?
        else {
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
            if !self.remote_prompt_recovery_is_current(
                session_id,
                agent_id,
                &prompt_id,
                &dispatch.leased_agent_id,
            )? {
                return Ok(());
            }
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
                    if remote_prompt_error_is_reconciliation_pending(&error)
                        || remote_prompt_reconciliation_pending(self, &dispatch).unwrap_or(true)
                    {
                        return Err(error);
                    }
                    attempt = attempt.saturating_add(1);
                    self.log_remote_prompt_recovery_retry(
                        session_id, agent_id, &dispatch, attempt, &error,
                    );
                    tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                }
            }
        }
    }

    pub(super) fn remote_prompt_recovery_dispatch(
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

    fn remote_prompt_recovery_dispatch_for_phase(
        &self,
        agent: &crate::agent::AgentInstance,
        observed_delivery_phase: Option<crate::session::DurablePromptDeliveryPhase>,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let Some(dispatch) = self.remote_prompt_recovery_dispatch(agent)? else {
            return Ok(None);
        };
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
        else {
            return Ok(None);
        };
        if active_prompt.id() != dispatch.prompt_id {
            return Ok(None);
        }
        let must_hold = active_prompt.durable_delivery_reconciliation_pending()
            || active_prompt.durable_delivery_phase()
                == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || observed_delivery_phase
                == Some(crate::session::DurablePromptDeliveryPhase::Dispatching);
        if must_hold {
            let marker_was_durable = active_prompt.durable_delivery_reconciliation_pending()
                || persist_remote_prompt_reconciliation_pending(self, &dispatch, true).is_ok();
            let detail = if marker_was_durable {
                "restart found a remote prompt in Dispatching without a durable submission acknowledgement"
            } else {
                "restart found a remote prompt in Dispatching without a durable submission acknowledgement; the reconciliation marker could not be persisted, so automatic replay remains blocked by the durable Dispatching phase"
            };
            self.report_remote_prompt_reconciliation_pending(&dispatch, detail);
            return Ok(None);
        }
        Ok(Some(dispatch))
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
        let active_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id);
        if let Some(prompt) = active_prompt.as_ref().filter(|prompt| {
            prompt.id() == prompt_id
                && prompt.status() == crate::session::PromptStatus::Cancelling
                && prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        }) {
            self.finalize_remote_prompt_cancellation_and_advance(
                session_id,
                agent_id,
                prompt.source_attachment_id(),
            )?;
            return Ok(false);
        }
        let prompt_is_current = active_prompt.as_ref().is_some_and(|prompt| {
            prompt.id() == prompt_id && prompt.status() != crate::session::PromptStatus::Cancelling
        });
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


    async fn remote_prompt_dispatch_after_claim_restart(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Ok(None);
        }
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch_for_phase(&agent, None)?
        else {
            return Ok(None);
        };
        if dispatch.session_id != session_id {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if prompt.status() == crate::session::PromptStatus::Cancelling {
            if prompt.durable_delivery_phase()
                == Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            {
                self.finalize_remote_prompt_cancellation_and_advance(
                    session_id,
                    agent_id,
                    prompt.source_attachment_id(),
                )?;
            }
            return Ok(None);
        }
        if prompt.id() != dispatch.prompt_id
            || prompt.durable_delivery_reconciliation_pending()
            || matches!(
                prompt.durable_delivery_phase(),
                Some(
                    crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered
                )
            )
        {
            return Ok(None);
        }
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;

        // Context resolution can await app work. Recheck exact prompt ownership before handing
        // the rebuilt dispatch to the network path.
        if !self.remote_prompt_recovery_is_current(
            session_id,
            agent_id,
            &dispatch.prompt_id,
            &dispatch.leased_agent_id,
        )? {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if prompt.id() != dispatch.prompt_id
            || prompt.durable_delivery_reconciliation_pending()
            || matches!(
                prompt.durable_delivery_phase(),
                Some(
                    crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered
                )
            )
        {
            return Ok(None);
        }
        Ok(Some(dispatch))
    }

    pub(super) async fn run_remote_prompt_dispatch_with_claim(
        &self,
        mut claim: RemotePromptAgentClaim,
        mut dispatch: Option<crate::app::KernelRemotePromptDispatch>,
    ) {
        let (session_id, agent_id) = claim.key.clone();
        loop {
            let cancellation_pending = match self
                .resume_remote_prompt_cancellation_with_claim(
                    &session_id,
                    &agent_id,
                    &mut claim,
                )
                .await
            {
                Ok(pending) => pending,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote cancellation remains held under the recovery claim",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    true
                }
            };
            if cancellation_pending {
                // A cancelling prompt must never fall through to submission/replay. The
                // durable phase or exact-run path above owns its next transition.
                let session = self.owned.session_store.get_session(&session_id).ok();
                let needs_receipt = session.as_ref().is_some_and(|session| {
                    self.owned
                        .prompt_state_owner
                        .active_prompt_for_agent(session, &agent_id)
                        .is_some_and(|prompt| {
                            prompt.status() == crate::session::PromptStatus::Cancelling
                                && prompt.durable_delivery_phase()
                                    == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                        })
                });
                if needs_receipt {
                    let mut pending_dispatch = dispatch.take();
                    if pending_dispatch.is_none() {
                        if let Ok(agent) = self.owned.agent_store.get_agent(&agent_id) {
                            pending_dispatch = self
                                .remote_prompt_recovery_dispatch(&agent)
                                .ok()
                                .flatten();
                        }
                    }
                    if let Some(pending_dispatch) = pending_dispatch {
                        self.retry_remote_prompt_receipt_with_claim(&pending_dispatch)
                            .await;
                    }
                }
                dispatch = None;
            }
            if let Some(dispatch) = dispatch.take() {
                if let Some(pending) = self.dispatch_remote_prompt_once(dispatch).await {
                    self.retry_remote_prompt_receipt_with_claim(&pending).await;
                }
            }
            if !claim.release_or_restart() {
                return;
            }
            dispatch = match self
                .remote_prompt_dispatch_after_claim_restart(&session_id, &agent_id)
                .await
            {
                Ok(dispatch) => dispatch,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt claim restart could not prepare the current prompt",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    None
                }
            };
        }
    }


    pub(super) fn spawn_remote_prompt_receipt_reconciliation(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &dispatch.session_id,
            &dispatch.agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            state
                .retry_remote_prompt_receipt_with_claim(&dispatch)
                .await;
            state
                .run_remote_prompt_dispatch_with_claim(claim, None)
                .await;
        });
    }

    async fn retry_remote_prompt_receipt_with_claim(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) {
        let mut attempt = 0_u32;
        loop {
            attempt = attempt.saturating_add(1);
            tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
            match self.reconcile_remote_prompt_worker_receipt(dispatch).await {
                Ok(provider_run_id) => {
                    match self
                        .finish_remote_prompt_dispatch(dispatch.clone(), Ok(provider_run_id))
                        .await
                    {
                        Ok(()) => return,
                        Err(error) => crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "verified worker receipt could not be settled; retrying read-only reconciliation",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "error": error.to_string(),
                            }),
                        ),
                    }
                }
                Err(DaemonError::NoActivePrompt { .. }) => return,
                Err(error) if remote_prompt_receipt_is_rejected(&error) => {
                    match self
                        .settle_verified_remote_prompt_rejection(dispatch, error)
                        .await
                    {
                        Ok(()) => return,
                        Err(settle_error) => crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "verified worker rejection could not be settled; retrying read-only reconciliation",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "error": settle_error.to_string(),
                            }),
                        ),
                    }
                }
                Err(error) => {
                    if attempt == 1 || attempt % 12 == 0 {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "worker receipt remains uncertain; retrying read-only reconciliation",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "attempt": attempt,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            }
        }
    }


}

fn remote_prompt_recovery_delay(attempt: u32) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(500_u64.saturating_mul(multiplier))
}
