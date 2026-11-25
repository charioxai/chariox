//! Structured prompt submission and the background handoff brief fold.
use super::context_handoff::PendingAgentContextHandoff;
use super::*;

/// Whether a steering dispatch waits for the provider's receipt. A caller that reads "accepted" as "the provider holds it"
/// (trigger, leased-agent and active-turn steering) waits. Only notification delivery ignores it: it persists its own submit
/// intent and settles through the finished-submit reapers, and it runs inside the global App event pass.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SteerReceipt {
    Ignored,
    Awaited,
}

impl KernelRuntimeState {
    /// Brings the agent's handoff brief up to date in the background: the
    /// turn already went out with the stored brief and the newest history, and
    /// the next handoff carries the folded brief.
    pub(super) fn spawn_handoff_brief_fold(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: crate::provider::RuntimeProviderRun,
        handoff: PendingAgentContextHandoff,
    ) {
        let handoffs = &self.owned.pending_agent_context_handoffs;
        if !handoffs.begin_brief_fold(&dispatch.session_id, &dispatch.agent_id) {
            return;
        }
        let state = self.clone();
        let (session_id, agent_id, prompt_id) = (
            dispatch.session_id.clone(),
            dispatch.agent_id.clone(),
            dispatch.prompt_id.clone(),
        );
        tokio::spawn(async move {
            state
                .with_current_handoff_brief(
                    &state.owned,
                    handoff,
                    &session_id,
                    &agent_id,
                    &prompt_id,
                    &provider_run,
                    || true,
                )
                .await;
            state
                .owned
                .pending_agent_context_handoffs
                .end_brief_fold(&session_id, &agent_id);
        });
    }

    pub(super) async fn submit_structured_prompt_with_handoff(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
        handoff: Option<PendingAgentContextHandoff>,
        hidden_system_context: &str,
        receipt_policy: SteerReceipt,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;
        let _room_guard = owned
            .room_secret_observations
            .barrier(&dispatch.session_id)?
            .read_owned()
            .await;
        if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
            return Ok(false);
        }
        let current = owned
            .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
        if current.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: dispatch.provider_run_id.clone(),
                state: current.state(),
                operation: "submit prompt",
            });
        }
        let handoff = handoff
            .map(|handoff| {
                owned.revalidate_context_handoff(
                    handoff,
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                )
            })
            .transpose()?;
        let explicit_handoff = handoff.as_ref().filter(|handoff| !handoff.derived);
        let prompt = match explicit_handoff {
            Some(handoff) => super::context_handoff::inject_protected_context_handoff(
                &dispatch.prompt,
                handoff,
                |context| {
                    owned
                        .room_secret_observations
                        .scrub(&dispatch.session_id, context)
                },
            )?,
            None => dispatch.prompt.clone(),
        };
        let render_handoff = |room| {
            let text = handoff
                .as_ref()
                .map(|handoff| handoff.render_switch(room))
                .unwrap_or_default();
            owned
                .room_secret_observations
                .scrub(&dispatch.session_id, text)
        };
        let hidden_system_context = hidden_system_context.to_string();
        // Preparation can outlive a Room policy change. Protect every retained
        // input, including failed-request and skill context, at final admission.
        let (hidden_system_context, attachments) =
            owned.room_secret_observations.scrub_prompt_inputs(
                &dispatch.session_id,
                (hidden_system_context, dispatch.attachments.clone()),
            )?;
        let (source_client_id, _source_user_id) =
            owned.active_prompt_source_attribution(&dispatch.session_id, &dispatch.agent_id)?;
        let mode = crate::prompt_assembly::provider_turn_mode_for_prompt(
            &dispatch.agent_id,
            owned
                .agent_store
                .get_agent(&dispatch.agent_id)?
                .is_metaagent(),
            source_client_id.as_deref(),
            &hidden_system_context,
        );
        if !owned.ensure_prompt_dispatch_matches_active_prompt(dispatch)? {
            return Ok(false);
        }
        let private_assessment_evidence = owned.private_evidence_for_dispatch(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
            &dispatch.provider_run_id,
            dispatch.prompt_origin,
            dispatch.steering,
        );
        owned.bind_agent_event_submission(dispatch)?;
        let result = owned.provider_store.enqueue_structured_prompt_submit(
            dispatch.session_id.clone(),
            dispatch.provider_run_id.clone(),
            dispatch.agent_id.clone(),
            dispatch.prompt_id.clone(),
            dispatch
                .target_active_prompt_id
                .as_deref()
                .unwrap_or(&dispatch.prompt_id),
            provider_run,
            &prompt,
            &hidden_system_context,
            &attachments,
            mode.with_prompt_origin(dispatch.prompt_origin),
            private_assessment_evidence,
            dispatch.steering,
            handoff
                .as_ref()
                .filter(|handoff| handoff.derived)
                .map(|_| &render_handoff as &crate::provider::switch_turn::RenderHandoff<'_>),
        );
        let receipt = result?;
        // MP-08/MP-09/MP-10: a trigger's queued steer is recoverable until the
        // structured provider accepts it, even after mailbox admission. Only a
        // definitive rejection permits replay.
        if dispatch.steering && receipt_policy == SteerReceipt::Awaited {
            drop(_room_guard);
            match receipt.await {
                Ok(crate::provider::ProviderPromptSubmitReceipt::Accepted) => {}
                Ok(crate::provider::ProviderPromptSubmitReceipt::Rejected) => {
                    return Err(DaemonError::ProviderPromptSteerRejected {
                        source: Box::new(DaemonError::LocalTransport {
                            operation: "steer queued prompt",
                            message: "provider rejected the queued prompt steer; the message remains queued"
                                .into(),
                        }),
                    });
                }
                Ok(crate::provider::ProviderPromptSubmitReceipt::OpenCodeConflict) => {
                    return Err(DaemonError::OpenCodeSteerConflict);
                }
                Ok(crate::provider::ProviderPromptSubmitReceipt::Uncertain) | Err(_) => {
                    return Err(DaemonError::ProviderPromptSteerUnconfirmed {
                        provider_run_id: dispatch.provider_run_id.clone(),
                    });
                }
            }
        }
        Ok(true)
    }
}
