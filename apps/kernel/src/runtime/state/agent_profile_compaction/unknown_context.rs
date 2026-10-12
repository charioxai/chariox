//! MP-08 / MP-10 / MP-11: cold/relaunched Claude sessions use protected history.
use super::*;

impl KernelRuntimeState {
    pub(super) fn recover_unknown_context_downshift(
        &self,
        session_id: &str,
        agent: &crate::agent::AgentInstance,
        run: Option<&RuntimeProviderRun>,
        target_provider: &str,
        target_account: &str,
        target_model: &str,
    ) -> Result<bool, crate::error::DaemonError> {
        let source_provider = run.map_or(agent.provider(), RuntimeProviderRun::provider);
        let source_account = run.map_or(
            agent.provider_account_profile(),
            RuntimeProviderRun::account_profile,
        );
        let Some(source_model) = run.map(RuntimeProviderRun::model).or(agent.model()) else {
            return Ok(false);
        };
        let claude =
            |provider: &str| crate::provider::canonical_provider_family(provider) == Some("claude");
        let unknown = run.is_none_or(|run| {
            run.usage().context_tokens.is_none()
                || !matches!(
                    run.state(),
                    ProviderRunState::Running | ProviderRunState::Parked
                )
        });
        let disabled = run.is_some_and(crate::provider::claude_1m_context_disabled);
        let window = |provider: &str, model: &str| {
            crate::provider::effective_model_context_window_tokens(provider, model, disabled)
        };
        if !unknown
            || !claude(source_provider)
            || !claude(target_provider)
            || source_account != target_account
            || window(target_provider, target_model) >= window(source_provider, source_model)
        {
            return Ok(false);
        }
        let Some(native_session) = run
            .and_then(|run| {
                run.provider_session_id()
                    .or_else(|| run.resume_state().provider_session_id(run.adapter_key()))
            })
            .or_else(|| {
                agent
                    .provider_resume_state()
                    .provider_session_id(source_provider)
            })
        else {
            return Ok(false);
        };
        // A cold source has no live run to compact. Persist the same historical
        // boundary, then clear only the matching identity; the fresh target's
        // ordinary dispatcher derives its handoff from protected source history.
        self.owned
            .operational_history_store
            .record_agent_compaction_boundary(
                session_id,
                agent.id(),
                source_provider,
                source_model,
                source_account,
                Some(native_session),
            )?;
        let outcome = self
            .owned
            .agent_store
            .clear_provider_resume_state_durably_if_matches(
                &self.owned.durable_state_store,
                agent.id(),
                source_provider,
                native_session,
                run.map_or("cold-claude-downshift", RuntimeProviderRun::id),
                "unknown_window_downshift_context",
            )?;
        if matches!(
            outcome,
            crate::agent::ProviderResumeClearOutcome::Superseded { .. }
        ) {
            return Err(crate::error::DaemonError::LocalTransport {
                operation: "compact before window downshift",
                message: "the native session changed during downshift; select the model again"
                    .into(),
            });
        }
        self.owned.record_notice(session_id, run.map(RuntimeProviderRun::id),
            self.owned.attachment_store.list_session_attachment_ids(session_id),
            "Claude's current context size is unknown. The smaller model will start a fresh native session with the protected conversation handoff.");
        Ok(true)
    }
}
