//! Rerun a turn whose provider failed on the agent's next substitute.

use super::provider_prompt_failure_runtime::{FailedProviderAttempt, SubstituteRerun};
use super::*;

impl KernelRuntimeState {
    /// Every provider failure of a turn reruns that same turn on the first
    /// configured substitute this turn has not tried yet, in list order, until
    /// none is left. The list is read at each failure, so edits made during the
    /// turn apply and no substitute runs twice. The substitute serves only this
    /// turn. A user cancel is not a provider failure; remote, external and
    /// provider-native turns are not rerun.
    pub(super) async fn rerun_failed_turn_on_substitute(
        &self,
        attempt: &FailedProviderAttempt<'_>,
        active_prompt: &crate::session::PromptQueueItem,
    ) -> Result<SubstituteRerun, DaemonError> {
        let failed_run = attempt.provider_run;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling
            || active_prompt.is_external()
            || !failed_run.client_interface().is_chariox()
            || failed_run
                .turn_substitute()
                .is_some_and(|turn| turn.prompt_id != active_prompt.id())
        {
            return Ok(SubstituteRerun::NotApplicable);
        }
        let agent = self.owned.agent_store.get_agent(attempt.agent_id)?;
        if agent.remote_execution().is_some() {
            return Ok(SubstituteRerun::NotApplicable);
        }
        let session_id = failed_run.session_id();
        let mut failed_label = provider_profile_label(failed_run.provider(), failed_run.model());
        // The reason builder redacts credentials itself and keeps the text
        // readable; `safe_message` is narrowed to the evidence character set.
        let mut reason = crate::provider::provider_turn_failure_reason(
            failed_run.adapter_key(),
            attempt.message,
            attempt.termination,
        );
        let mut tried = failed_run
            .turn_substitute()
            .map(|turn| turn.tried.clone())
            .unwrap_or_default();
        let Some(mut index) = self.owned.next_untried_substitute_index(&agent, &tried)? else {
            if failed_run.turn_substitute().is_some() {
                self.record_turn_substitute_notice(
                    session_id,
                    failed_run.id(),
                    no_substitute_left_notice(&failed_label, &reason),
                );
            }
            return Ok(SubstituteRerun::NotApplicable);
        };
        // Unbind the turn before retiring its run, so that run's exit cannot
        // settle it while the substitute starts.
        self.owned.mark_active_prompt_delivery(
            session_id,
            attempt.agent_id,
            active_prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Accepted,
            None,
            None,
        )?;
        self.record_failed_provider_attempt(attempt).await?;
        loop {
            let substitute = &agent.substitutes()[index];
            tried.push(substitute.clone());
            let label = provider_profile_label(&substitute.provider, &substitute.model);
            self.record_turn_substitute_notice(
                session_id,
                failed_run.id(),
                format!(
                    "This turn runs on {label} because {failed_label} failed: {}.",
                    reason.trim_end_matches('.')
                ),
            );
            let launched = self
                .with_app_side_effect(|app| {
                    app.launch_turn_substitute_run(
                        session_id,
                        attempt.agent_id,
                        active_prompt,
                        tried.clone(),
                    )
                })
                .await;
            let error = match launched {
                Ok(provider_run_id) => {
                    // The failed attempt's turn tracking ends here; the turn
                    // continues on the substitute run.
                    let _ = self.owned.clear_prompt_activity(failed_run.id());
                    if active_prompt.workflow_run_id().is_none() {
                        // A conversation's substitute starts from its history.
                        if let Ok(substitute_run) =
                            self.owned.provider_store.get_run(&provider_run_id)
                        {
                            self.owned.prepare_turn_substitute_context_handoff(
                                failed_run,
                                Some(substitute_run.id()),
                                substitute_run.provider(),
                                substitute_run.account_profile(),
                                Some(substitute_run.model()),
                            );
                        }
                    }
                    let dispatch =
                        self.turn_rerun_dispatch(session_id, &provider_run_id, active_prompt)?;
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                    }
                    return Ok(SubstituteRerun::Started);
                }
                Err(error) => error,
            };
            failed_label = label;
            reason = format!(
                "could not start: {}",
                crate::provider::sanitize_provider_diagnostic(&error.to_string())
            );
            match self.owned.next_untried_substitute_index(&agent, &tried)? {
                Some(next) => index = next,
                None => {
                    self.record_turn_substitute_notice(
                        session_id,
                        failed_run.id(),
                        no_substitute_left_notice(&failed_label, &reason),
                    );
                    return Ok(SubstituteRerun::Exhausted);
                }
            }
        }
    }

    fn record_turn_substitute_notice(
        &self,
        session_id: &str,
        provider_run_id: &str,
        notice: String,
    ) {
        self.owned.record_notice(
            session_id,
            Some(provider_run_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(session_id),
            notice,
        );
    }

    fn turn_rerun_dispatch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<crate::app::KernelPromptDispatch, DaemonError> {
        Ok(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: prompt.target_agent_id().to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: None,
            source_attachment_id: self
                .owned
                .promoted_prompt_source_attachment_id(session_id, prompt.source_attachment_id())?,
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: false,
        })
    }
}

impl KernelRuntimeOwnedState {
    /// Hands a conversational turn answered by a substitute to the agent's
    /// configured profile, which starts the next turn.
    pub(super) fn prepare_turn_substitute_return_handoff(
        &self,
        substitute_run: &crate::provider::RuntimeProviderRun,
    ) {
        let Some(agent) = substitute_run
            .agent_instance_id()
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok())
        else {
            return;
        };
        let account = if crate::provider::canonical_provider_family(agent.provider()).is_some() {
            let owner = crate::account_profile::provider_account_authority_owner_user_id(
                &self.config_projection.snapshot(),
                agent.owner_user_id(),
            );
            self.provider_account_profiles
                .get(&owner, agent.provider(), agent.provider_account_profile())
                .map(|account| account.profile_id)
                .unwrap_or_else(|_| agent.provider_account_profile().to_string())
        } else {
            agent.provider_account_profile().to_string()
        };
        self.prepare_turn_substitute_context_handoff(
            substitute_run,
            None,
            agent.provider(),
            &account,
            agent.model(),
        );
    }

    /// The first configured substitute not in `tried` whose saved account is
    /// still usable for its model.
    pub(super) fn next_untried_substitute_index(
        &self,
        agent: &crate::agent::AgentInstance,
        tried: &[crate::agent::AgentSubstituteProfile],
    ) -> Result<Option<usize>, DaemonError> {
        let config = self.config_projection.snapshot();
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &config,
            agent.owner_user_id(),
        );
        let now_ms = crate::session::unix_epoch_ms();
        for (index, candidate) in agent.substitutes().iter().enumerate() {
            if tried.contains(candidate) {
                continue;
            }
            let skipped = if candidate
                .kernel_id
                .as_deref()
                .is_some_and(|kernel_id| kernel_id != config.daemon_id)
            {
                Some("it runs on another kernel".to_string())
            } else if crate::provider::canonical_provider_family(&candidate.provider).is_some() {
                match self.provider_account_profiles.find(
                    &owner,
                    &candidate.provider,
                    candidate.account_profile.as_deref().unwrap_or("default"),
                )? {
                    None => Some(format!(
                        "its saved {} account is no longer available",
                        candidate.provider
                    )),
                    Some(account) if account.has_confirmed_exhaustion(&candidate.model, now_ms) => {
                        Some(format!(
                            "account `{}` has exhausted capacity for `{}`",
                            account.label, candidate.model
                        ))
                    }
                    Some(_) => None,
                }
            } else {
                None
            };
            let Some(skipped) = skipped else {
                return Ok(Some(index));
            };
            self.record_notice(
                agent.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(agent.session_id()),
                format!(
                    "Skipping substitute {} for agent `{}`: {skipped}.",
                    index + 1,
                    agent.id()
                ),
            );
        }
        Ok(None)
    }
}

fn no_substitute_left_notice(failed_label: &str, reason: &str) -> String {
    format!(
        "No substitute is left for this turn: {failed_label} failed: {}.",
        reason.trim_end_matches('.')
    )
}

/// The model a provider profile runs, or the provider when it uses its default.
fn provider_profile_label(provider: &str, model: &str) -> String {
    let model = model.trim();
    if model.is_empty() || model == "default" {
        provider.to_string()
    } else {
        model.to_string()
    }
}
