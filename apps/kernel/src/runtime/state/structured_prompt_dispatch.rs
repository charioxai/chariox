//! Structured prompt submission and the detached handoff preparation continuation.
use super::context_handoff::PendingAgentContextHandoff;
use super::local_prompt_dispatch_runtime::join_hidden_context;
use super::*;

impl KernelRuntimeState {
    pub(super) fn spawn_handoff_brief_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_run: crate::provider::RuntimeProviderRun,
        handoff: PendingAgentContextHandoff,
        hidden_system_context: String,
    ) {
        self.owned.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id),
            "Preparing handoff brief…".to_string(),
        );
        let state = self.clone();
        tokio::spawn(async move {
            if !state.owned.handoff_brief_dispatch_is_current(&dispatch) {
                return;
            }
            let handoff = state
                .with_current_handoff_brief(
                    &state.owned,
                    handoff,
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &provider_run,
                    || state.owned.handoff_brief_dispatch_is_current(&dispatch),
                )
                .await;
            // Order the final ownership/cancellation check and Submit with Abort.
            // No lane is held while the brief utility runs.
            let _permit = state
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if !state.owned.handoff_brief_dispatch_is_current(&dispatch) {
                return;
            }
            match state.submit_structured_prompt_with_handoff(
                &dispatch,
                &provider_run,
                Some(handoff),
                &hidden_system_context,
            ) {
                Ok(true) => state
                    .owned
                    .update_metaagent_event_prompt_delivery_for_prompt(
                    &dispatch.prompt_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                    None,
                ),
                Ok(false) => {}
                Err(error) => {
                    let _ = state.fail_prompt_dispatch(dispatch, error).await;
                }
            }
        });
    }

    pub(super) fn submit_structured_prompt_with_handoff(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
        handoff: Option<PendingAgentContextHandoff>,
        hidden_system_context: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;
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
        let prompt_with_handoff = handoff
            .map(|handoff| {
                super::context_handoff::inject_context_handoff(&dispatch.prompt, &handoff)
            })
            .unwrap_or_else(|| dispatch.prompt.clone());
        let granted_skill_context = owned.granted_skill_hidden_context(
            &dispatch.session_id,
            &dispatch.agent_id,
            &prompt_with_handoff,
        )?;
        let hidden_system_context =
            join_hidden_context(hidden_system_context, &granted_skill_context);
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
            &prompt_with_handoff,
            &hidden_system_context,
            &dispatch.attachments,
            mode,
            dispatch.steering,
        );
        result.map(|()| true)
    }
}

impl KernelRuntimeOwnedState {
    fn handoff_brief_dispatch_is_current(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> bool {
        self.ensure_prompt_dispatch_matches_active_prompt(dispatch)
            .unwrap_or(false)
            && self
                .provider_store
                .get_run_for_agent(&dispatch.session_id, &dispatch.agent_id)
                .is_some_and(|run| {
                    run.id() == dispatch.provider_run_id
                        && run.state() == crate::provider::ProviderRunState::Running
                })
    }
}
