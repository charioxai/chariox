//! Agent profile, alias, and substitute administration.
//!
//! Execution-mode and permission overrides live in `agent_config_owned_state`; capability grants
//! live in `capability_owned_state`.

use super::*;

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_PROFILE_VALIDATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = Default::default();
}

impl owned::OwnedRemoteAgentProfileUpdate {
    pub(super) fn validate_worker_acknowledgement(
        &self,
        home_agent_id: &str,
        leased_agent: &crate::execution_lease::LeasedAgent,
    ) -> Result<(), DaemonError> {
        if leased_agent.id != self.leased_agent_id
            || leased_agent.lease_id != self.execution_lease_id
            || leased_agent.home_agent_id != home_agent_id
            || leased_agent.provider != self.provider
            || leased_agent.account_profile != self.account_profile
            || leased_agent.model != self.model
            || leased_agent.effort != self.effort
        {
            return Err(DaemonError::LocalTransport {
                operation: "update remote leased agent profile",
                message: "the worker acknowledgement does not match the requested agent, lease, or provider profile; the home profile was not changed".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct AgentProfileSelections {
    pub(super) provider: Option<String>,
    pub(super) account_profile: Option<String>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<Option<String>>,
}

pub(super) struct ValidatedAgentProfile {
    pub(super) agent: crate::agent::AgentInstance,
    pub(super) provider: String,
    pub(super) account_profile: String,
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
}

impl KernelRuntimeOwnedState {
    /// MP-08 / MP-10 / MP-11: selections and resolved run identities compare in
    /// the same namespace before deciding whether a native session survives.
    pub(super) fn agent_profile_source_account(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> String {
        if let Some(run) = self
            .provider_store
            .get_run_for_agent(agent.session_id(), agent.id())
            .or_else(|| {
                self.provider_store
                    .get_latest_run_for_agent(agent.session_id(), agent.id())
            })
        {
            let native_session = agent
                .provider_resume_state()
                .provider_session_id(run.adapter_key());
            let owns_native_session = native_session.is_some()
                && run
                    .provider_session_id()
                    .or_else(|| run.resume_state().provider_session_id(run.adapter_key()))
                    == native_session;
            if (crate::account_profile::provider_account_selection_for_run(&run)
                == agent.provider_account_profile()
                || owns_native_session)
                && crate::provider::canonical_provider_family(run.provider())
                    .unwrap_or(run.provider())
                    == crate::provider::canonical_provider_family(agent.provider())
                        .unwrap_or(agent.provider())
            {
                return run.account_profile().to_string();
            }
        }
        // An unbound selector may have been reassigned since the native session
        // ran. Compare it as stored: concrete ids retain their original behavior,
        // while aliases reset instead of guessing the current account owns it.
        agent.provider_account_profile().to_string()
    }

    pub(super) fn validate_agent_profile_update(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        selections: AgentProfileSelections,
    ) -> Result<ValidatedAgentProfile, DaemonError> {
        let AgentProfileSelections {
            provider,
            account_profile,
            model,
            effort,
        } = selections;
        let provider = provider
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let model = model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let effort = effort.map(|value| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent profile")?;
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "update agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        self.ensure_agent_config_not_provider_native_tui(
            session_id,
            agent_id,
            "update agent profile",
        )?;
        let target_provider = provider.as_deref().unwrap_or(agent.provider()).to_string();
        let family_changed = crate::provider::canonical_provider_family(&target_provider)
            .unwrap_or(&target_provider)
            != crate::provider::canonical_provider_family(agent.provider())
                .unwrap_or(agent.provider());
        // Omitted selections belong to the current provider family. A provider-only
        // control selects the destination default rather than reusing a foreign ID.
        let target_model = model
            .as_deref()
            .or(if family_changed { None } else { agent.model() })
            .map(str::to_string);
        let requested_account_profile = account_profile.as_deref().unwrap_or(if family_changed {
            "default"
        } else {
            agent.provider_account_profile()
        });
        let target_account_profile = if crate::provider::canonical_provider_family(&target_provider)
            .is_some_and(|provider| matches!(provider, "codex" | "claude" | "opencode"))
        {
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config_projection.snapshot(),
                    agent.owner_user_id(),
                );
            self.provider_account_profiles
                .get(
                    &account_owner_user_id,
                    &target_provider,
                    requested_account_profile,
                )?
                .profile_id
        } else {
            requested_account_profile.to_string()
        };
        let target_effort = match effort.as_ref() {
            Some(value) => value.as_deref(),
            None if family_changed => None,
            None => agent.effort(),
        };
        Ok(ValidatedAgentProfile {
            effort: target_effort.map(str::to_string),
            agent,
            provider: target_provider,
            account_profile: target_account_profile,
            model: target_model,
        })
    }

    pub(super) fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        selections: AgentProfileSelections,
    ) -> Result<owned::OwnedAgentProfileUpdate, DaemonError> {
        let account_requested = selections.account_profile.is_some();
        let validated =
            self.validate_agent_profile_update(session_id, agent_id, caller_user_id, selections)?;
        let agent = validated.agent;
        #[cfg(test)]
        AFTER_PROFILE_VALIDATION.with(|gate| {
            if let Some(gate) = gate.borrow_mut().take() {
                gate();
            }
        });
        let target_provider = validated.provider;
        let target_account_profile = validated.account_profile;
        let target_model = validated.model;
        let target_effort = validated.effort.as_deref();
        let source_account = self.agent_profile_source_account(&agent);
        let provider_model_or_account_changed = target_provider != agent.provider()
            || target_model.as_deref() != agent.model()
            || target_account_profile != source_account;
        let selection_changed =
            account_requested && target_account_profile != agent.provider_account_profile();
        if !provider_model_or_account_changed
            && target_effort == agent.effort()
            && !selection_changed
        {
            return Ok(owned::OwnedAgentProfileUpdate {
                agent,
                terminated_run_ids: Vec::new(),
                remote_update: None,
            });
        }
        let remote_update =
            agent
                .remote_execution()
                .map(|binding| owned::OwnedRemoteAgentProfileUpdate {
                    worker_kernel_id: binding.worker_kernel_id.clone(),
                    execution_lease_id: binding.execution_lease_id.clone(),
                    leased_agent_id: binding.leased_agent_id.clone(),
                    relay_url: binding.relay_url.clone(),
                    relay_token: binding.relay_token.clone(),
                    provider: target_provider.clone(),
                    account_profile: target_account_profile.clone(),
                    model: target_model.clone(),
                    effort: target_effort.map(str::to_string),
                });
        let mut terminated_run_ids = Vec::new();
        if remote_update.is_none()
            && (provider_model_or_account_changed || target_effort != agent.effort())
        {
            if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
                match run.state() {
                    crate::provider::ProviderRunState::Starting
                    | crate::provider::ProviderRunState::Running
                    | crate::provider::ProviderRunState::Parked => {
                        let outcome = self
                            .provider_store
                            .terminate_run_provider_only(session_id, run.id())?;
                        self.clear_active_provider_run_session_pointer(
                            session_id,
                            outcome.run().id(),
                        )?;
                        let ended = outcome.into_run();
                        terminated_run_ids.push(ended.id().to_string());
                        self.provider_run_projection.update(ended);
                    }
                    crate::provider::ProviderRunState::Ended => {
                        self.provider_store.clear_runtime(run.id());
                    }
                }
            }
        }
        let agent = if remote_update.is_some() {
            agent
        } else {
            let agent = {
                // MP-08 / MP-10 / MP-11: source retirement fences native writes.
                // Retain identities confirmed since validation, under the same
                // lock as the commit. Selection-only pins leave the run live,
                // so their read and write must also be atomic with confirmation.
                let mut agents = self.agent_store.write();
                let current = agents.get_agent(agent_id)?;
                let resume_state = current.provider_resume_state().after_profile_change(
                    current.provider(),
                    &source_account,
                    &target_provider,
                    &target_account_profile,
                );
                agents.set_agent_runtime_profile_with_account_profile(
                    agent_id,
                    &target_provider,
                    target_model,
                    target_effort.map(str::to_string),
                    Some(target_account_profile),
                    resume_state,
                )?
            };
            let _ = self.session_snapshot(session_id)?;
            agent
        };
        Ok(owned::OwnedAgentProfileUpdate {
            agent,
            terminated_run_ids,
            remote_update,
        })
    }

    /// MP-08 / MP-10 / MP-11: an idle native run may confirm another identity
    /// after a selection-only commit. Persist the current agent while excluding
    /// native writes, rather than append an older profile result over them.
    pub(super) fn persist_current_local_agent_profile(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agents = self.agent_store.read();
        let agent = agents.get_agent(agent_id)?;
        self.durable_state_store.append_event(
            "agent.updated",
            Some(agent.id().to_string()),
            serde_json::json!({ "agent": &agent, "capability_name": null }),
        )?;
        Ok(agent)
    }

    pub(super) fn commit_remote_agent_profile_update(
        &self,
        session_id: &str,
        agent_id: &str,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "commit remote agent profile",
                message: format!(
                    "agent `{agent_id}` has an active turn; update the profile after it finishes"
                ),
            });
        }
        if crate::provider::canonical_provider_family(&provider).is_some() {
            let account_owner_user_id =
                crate::account_profile::provider_account_authority_owner_user_id(
                    &self.config_projection.snapshot(),
                    agent.owner_user_id(),
                );
            let confirmed = self.provider_account_profiles.get(
                &account_owner_user_id,
                &provider,
                &account_profile,
            )?;
            if confirmed.profile_id != account_profile {
                return Err(DaemonError::LocalTransport {
                    operation: "commit remote agent profile",
                    message: "selected account identity changed while the worker confirmed the profile; select the account again".into(),
                });
            }
        }
        // The worker has acknowledged the replacement profile. Completed turns
        // can still have a running home projection; retire it so projection
        // fallback and lagging snapshots cannot advertise the old profile.
        if let Some(binding) = agent.remote_execution() {
            self.provider_run_projection.retire_leased_agent_runs(
                session_id,
                agent_id,
                &binding.leased_agent_id,
            );
            if let Some(run_id) = session.active_provider_run_id() {
                if crate::provider::worker_provider_run_id_from_projected_leased_id(
                    &binding.leased_agent_id,
                    run_id,
                )
                .is_some()
                {
                    self.clear_active_provider_run_session_pointer(session_id, run_id)?;
                }
            }
        }
        let source_account = self.agent_profile_source_account(&agent);
        if let Some(binding) = agent.remote_execution().filter(|_| {
            agent.provider() != provider
                || source_account != account_profile
                || agent.model() != model.as_deref()
        }) {
            self.operational_history_store
                .rearm_recreated_remote_lease_context(session_id, agent_id, binding)?;
        }
        let resume_state = agent.provider_resume_state().after_profile_change(
            agent.provider(),
            &source_account,
            &provider,
            &account_profile,
        );
        self.agent_store
            .set_agent_runtime_profile_with_account_profile(
                agent_id,
                &provider,
                model,
                effort,
                Some(account_profile),
                resume_state,
            )?;
        self.agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let agent = self.agent_store.get_agent(agent_id)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(agent)
    }

    pub(super) fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "alias agent")?;
        self.agent_store.alias_agent(agent_id, alias)
    }

    pub(super) fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        self.ensure_agent_owner(agent_id, caller_user_id, "update agent substitutes")?;
        match action {
            crate::local::AgentSubstituteAction::Add {
                provider,
                model,
                variant,
                account_profile,
                kernel_id,
                worktree_id,
            } => {
                let provider = provider.trim().to_string();
                let kernel_id = kernel_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                let worktree_id = worktree_id.and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                });
                if let Some(kernel_id) = kernel_id.as_deref() {
                    let local_kernel_id = self.config_projection.snapshot().daemon_id;
                    if kernel_id != local_kernel_id {
                        return Err(DaemonError::LocalTransport {
                            operation: "add agent substitute",
                            message: format!(
                                "remote substitute kernel `{kernel_id}` is not supported yet"
                            ),
                        });
                    }
                }
                // Authority seam: a substitute must bind a real stable account
                // profile from the kernel account inventory. An omitted alias
                // resolves the provider's current default to its stable
                // profile_id now, so the substitute never follows a later
                // default change and can never launch via the literal default
                // sentinel. Unregistered providers (no inventory family) keep
                // historical pass-through semantics.
                let requested_account = account_profile
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let account_profile = if crate::provider::canonical_provider_family(&provider)
                    .is_some_and(|family| matches!(family, "codex" | "claude" | "opencode"))
                {
                    let requested = requested_account.unwrap_or("default");
                    let account_owner_user_id =
                        crate::account_profile::provider_account_authority_owner_user_id(
                            &self.config_projection.snapshot(),
                            agent.owner_user_id(),
                        );
                    match self.provider_account_profiles.get(
                        &account_owner_user_id,
                        &provider,
                        requested,
                    ) {
                        Ok(profile) => Some(profile.profile_id),
                        Err(_error) if requested_account.is_none() => {
                            return Err(DaemonError::LocalTransport {
                                operation: "add agent substitute",
                                message: format!(
                                    "no usable account profile is registered for `{provider}`; \
                                     register one first or bind an explicit account alias"
                                ),
                            });
                        }
                        Err(_) => {
                            // Never echo stable internal profile IDs (or the
                            // literal default sentinel) back to users. If the
                            // selection matches an account registered under a
                            // different provider, surface its public label as
                            // a hint instead.
                            let cross_provider_hint = self
                                .provider_account_profiles
                                .list(&account_owner_user_id, None)
                                .ok()
                                .and_then(|profiles| {
                                    profiles
                                        .iter()
                                        .find(|profile| profile.profile_id == requested)
                                        .map(|profile| {
                                            format!(
                                                " The account alias “{}” is registered for {}.",
                                                profile.label, profile.provider
                                            )
                                        })
                                })
                                .unwrap_or_default();
                            return Err(DaemonError::LocalTransport {
                                operation: "add agent substitute",
                                message: format!(
                                    "no {provider} account matches that selection{cross_provider_hint}; \
                                     choose an available account alias"
                                ),
                            });
                        }
                    }
                } else {
                    requested_account.map(str::to_string)
                };
                self.agent_store.add_agent_substitute(
                    agent_id,
                    crate::agent::AgentSubstituteProfile::new(provider, model, variant)
                        .with_account_profile(account_profile)
                        .with_kernel_id(kernel_id)
                        .with_worktree_id(worktree_id),
                )
            }
            crate::local::AgentSubstituteAction::Remove { index } => {
                self.agent_store.remove_agent_substitute(agent_id, index)
            }
            crate::local::AgentSubstituteAction::Move {
                from_index,
                to_index,
            } => self
                .agent_store
                .move_agent_substitute(agent_id, from_index, to_index),
            crate::local::AgentSubstituteAction::Clear {} => {
                self.agent_store.clear_agent_substitutes(agent_id)
            }
            crate::local::AgentSubstituteAction::SetTimeout { timeout_ms } => self
                .agent_store
                .set_agent_substitution_timeout(agent_id, timeout_ms),
        }
    }
}
