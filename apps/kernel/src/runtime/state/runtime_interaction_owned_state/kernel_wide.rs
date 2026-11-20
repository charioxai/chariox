//! MP-08 / MP-10 / MP-11: access decisions on the shared kernel interaction
//! board, independent of sessions. Answers, passkey checks, limits, expiry and
//! revocation use the ordinary interaction machinery.
use super::*;

/// A decision counted against the kernel limit, copied out so no lock is held while it is named.
struct OpenDecision {
    id: String,
    session_id: String,
    interaction: Option<crate::session::RuntimeInteraction>,
}

const LISTED_OPEN_DECISIONS: usize = 10;

/// The refusal that `interaction_waits` recognises by its leading words.
fn decision_limit_message(holders: &[String]) -> String {
    let mut listed = holders
        .iter()
        .take(LISTED_OPEN_DECISIONS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if holders.len() > LISTED_OPEN_DECISIONS {
        listed.push_str(&format!(
            ", and {} more",
            holders.len() - LISTED_OPEN_DECISIONS
        ));
    }
    format!(
        "{}. Open decisions: {listed}. Answer or dismiss them in your Chariox terminal, then retry.",
        super::KERNEL_DECISION_LIMIT
    )
}

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_kernel_decision_capacity(
        &self,
        owner: &str,
        interaction: &crate::session::RuntimeInteraction,
        session_of: &dyn Fn(&str) -> Option<crate::session::RuntimeSession>,
    ) -> Result<(), DaemonError> {
        let owner_elevation = interaction.id().starts_with("sudo:")
            && interaction.requester().is_none()
            && interaction
                .sudo_request()
                .is_none_or(|request| request.grant_id.is_none())
            && interaction
                .choices()
                .iter()
                .any(crate::session::RuntimeInteractionChoice::requires_passkey);
        let pending = self.pending_interactions.write();
        let decisions = pending
            .iter()
            .filter(|(_, p)| p.belongs_to(&self.session_store))
            .filter(|(_, p)| p.kernel_operation_owner.is_some())
            .collect::<Vec<_>>();
        // Keep the hard kernel bound, reserving its last slot for owner elevation.
        // Agent decisions cannot spend the owner's separate elevation allowance.
        let owner_count = decisions
            .iter()
            .filter(|(_, p)| p.kernel_operation_owner.as_deref() == Some(owner))
            .filter(|(_, p)| {
                !owner_elevation
                    || p.passkey_prompt.as_ref().is_some_and(|prompt| {
                        prompt.kind == crate::local::PasskeyPromptKind::Sudo
                            && prompt.requester.is_none()
                            && prompt
                                .sudo_request
                                .as_ref()
                                .is_none_or(|request| request.grant_id.is_none())
                    })
            })
            .count();
        let owner_full = owner_count >= 8;
        if decisions.len() >= 32 || (!owner_elevation && decisions.len() >= 31) || owner_full {
            // The refusal names what holds the room, so the caller can clear it. The lookups
            // below take the session lock, which registration holds before this one.
            let holders = decisions
                .iter()
                .filter(|(_, p)| !owner_full || p.kernel_operation_owner.as_deref() == Some(owner))
                .map(|(id, p)| OpenDecision {
                    id: (*id).clone(),
                    session_id: p.session_id.clone(),
                    interaction: p
                        .kernel_wide_interaction
                        .clone()
                        .or_else(|| p.user_domain_interaction.clone()),
                })
                .collect::<Vec<_>>();
            drop(decisions);
            drop(pending);
            let holders = holders
                .iter()
                .map(|holder| self.describe_open_decision(holder, session_of))
                .collect::<Vec<_>>();
            return Err(interaction_error(&decision_limit_message(&holders)));
        }
        Ok(())
    }

    /// One open decision as "id (agent: title)" for the cap refusal.
    fn describe_open_decision(
        &self,
        open: &OpenDecision,
        session_of: &dyn Fn(&str) -> Option<crate::session::RuntimeSession>,
    ) -> String {
        let interaction = open.interaction.clone().or_else(|| {
            session_of(&open.session_id)?
                .active_interactions()
                .iter()
                .find(|active| active.id() == open.id)
                .cloned()
        });
        let title = interaction
            .as_ref()
            .and_then(|interaction| interaction.title())
            .unwrap_or("decision");
        let agent = interaction
            .as_ref()
            .and_then(|interaction| interaction.related_agent_id())
            .and_then(|agent_id| self.agent_store.get_agent(agent_id).ok())
            .map(|agent| agent.alias().unwrap_or(agent.agent_ref()).to_owned());
        match agent {
            Some(agent) => format!("{} ({agent}: {title})", open.id),
            None => format!("{} ({title})", open.id),
        }
    }

    /// An outside agent's Vault value request: a kernel-wide decision every owner terminal
    /// shows as a popup. It has the secret choice and no passkey; the answer is the value.
    pub(super) fn register_kernel_wide_secret_interaction(
        &self,
        kind: super::CardKind,
        interaction: crate::session::RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
        owner: Option<&str>,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
        credential_owner: Option<&str>,
    ) -> Result<(), DaemonError> {
        let owner = owner
            .filter(|owner| *owner == self.config_projection.local_owner_user_id())
            .ok_or_else(|| interaction_error("Only the local kernel owner can receive a secret"))?;
        if forwarding.is_some()
            || credential_owner.is_some()
            || interaction.agent_id().is_some()
            || interaction.kind() != crate::session::RuntimeInteractionKind::Choice
            || interaction.default_on_timeout().is_some()
            || !interaction.custom_choice().is_some_and(|choice| {
                choice.input_kind() == crate::session::RuntimeInteractionInputKind::Secret
            })
            || !interaction
                .timeout_sec()
                .is_some_and(|seconds| (1..=3600).contains(&seconds))
        {
            return Err(interaction_error("Invalid kernel-wide secret request"));
        }
        super::card_registry::admit_scoped_card(kind, &interaction, &responder)?;
        let scope = crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE;
        let prompt = super::super::passkey_prompts::vault_secret_prompt(
            scope,
            &interaction,
            crate::session::unix_epoch_ms(),
        );
        self.ensure_kernel_decision_capacity(owner, &interaction, &|id| {
            self.session_store.get_session(id).ok()
        })?;
        let mut pending = self.pending_interactions.write();
        if pending.contains_key(interaction.id()) {
            return Err(interaction_error(super::INTERACTION_ALREADY_PENDING));
        }
        self.passkey_prompts
            .forget_answered(scope, interaction.id());
        pending.insert(
            interaction.id().into(),
            super::super::PendingInteraction {
                checking_answer: Default::default(),
                goal_approval: None,
                session_id: scope.into(),
                session_store_identity: self.session_store.weak_identity(),
                agent_lifetime: None,
                kernel_operation_owner: Some(owner.into()),
                terminal_credential_owner: None,
                kernel_operation_deadline: Some(
                    std::time::Instant::now()
                        + std::time::Duration::from_secs(
                            interaction.timeout_sec().expect("validated timeout"),
                        ),
                ),
                passkey_prompt: Some(prompt),
                holder: super::super::DecisionHolder::User,
                holder_tenure: 0,
                responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
                kernel_wide_interaction: Some(interaction),
                user_domain_interaction: None,
                async_task_id: None,
            },
        );
        drop(pending);
        self.passkey_prompts.record_change();
        Ok(())
    }

    pub(super) fn register_kernel_wide_access_interaction(
        &self,
        kind: super::CardKind,
        interaction: crate::session::RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
        owner: Option<&str>,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
        credential_owner: Option<&str>,
    ) -> Result<(), DaemonError> {
        let owner = owner
            .filter(|owner| *owner == self.config_projection.local_owner_user_id())
            .ok_or_else(|| interaction_error("Only the local kernel owner can grant access"))?;
        if forwarding.is_some()
            || credential_owner.is_some()
            || interaction.agent_id().is_some()
            || interaction.kind() != crate::session::RuntimeInteractionKind::Permission
            || interaction.custom_choice().is_some()
            || interaction.default_on_timeout().is_some()
            || !interaction
                .timeout_sec()
                .is_some_and(|seconds| (1..=86_400).contains(&seconds))
            || !interaction.kernel_operation_id().is_some_and(|id| {
                id.starts_with("access-grant:") || id.starts_with("access-extension:")
            })
        {
            return Err(interaction_error("Invalid kernel-wide access decision"));
        }
        super::card_registry::admit_scoped_card(kind, &interaction, &responder)?;
        let scope = crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE;
        let prompt = super::super::passkey_prompts::passkey_prompt_for_scope(
            scope,
            None,
            &interaction,
            crate::session::unix_epoch_ms(),
        )?
        .ok_or_else(|| interaction_error("Access requires a passkey popup"))?;
        self.ensure_kernel_decision_capacity(owner, &interaction, &|id| {
            self.session_store.get_session(id).ok()
        })?;
        let mut pending = self.pending_interactions.write();
        if pending.contains_key(interaction.id())
            || pending.values().any(|p| {
                p.belongs_to(&self.session_store)
                    && p.kernel_wide_interaction
                        .as_ref()
                        .is_some_and(|existing| existing.subject() == interaction.subject())
            })
        {
            return Err(interaction_error(super::INTERACTION_ALREADY_PENDING));
        }
        self.passkey_prompts
            .forget_answered(scope, interaction.id());
        pending.insert(
            interaction.id().into(),
            super::super::PendingInteraction {
                checking_answer: Default::default(),
                goal_approval: None,
                async_task_id: None,
                session_id: scope.into(),
                session_store_identity: self.session_store.weak_identity(),
                agent_lifetime: None,
                kernel_operation_owner: Some(owner.into()),
                terminal_credential_owner: None,
                kernel_operation_deadline: Some(
                    std::time::Instant::now()
                        + std::time::Duration::from_secs(
                            interaction.timeout_sec().expect("validated timeout"),
                        ),
                ),
                passkey_prompt: Some(prompt),
                holder: super::super::DecisionHolder::User,
                holder_tenure: 0,
                responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
                kernel_wide_interaction: Some(interaction),
                user_domain_interaction: None,
            },
        );
        drop(pending);
        self.passkey_prompts.record_change();
        Ok(())
    }
}
