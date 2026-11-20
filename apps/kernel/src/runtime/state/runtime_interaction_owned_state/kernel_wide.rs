//! MP-08 / MP-10 / MP-11: access decisions on the shared kernel interaction
//! board, independent of sessions. Answers, passkey checks, limits, expiry and
//! revocation use the ordinary interaction machinery.
use super::*;

use crate::session::{RuntimeInteraction, RuntimeSession, SessionStatus};

/// The refusal that `interaction_waits` recognises by its leading words.
fn decision_limit_message(holders: &[String]) -> String {
    if holders.is_empty() {
        return format!(
            "{}. Other owners have open decisions. Retry when a slot clears.",
            super::KERNEL_DECISION_LIMIT
        );
    }
    const LISTED_OPEN_DECISIONS: usize = 10;
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
    pub(in crate::runtime::state) fn pending_kernel_decisions(
        &self,
    ) -> Vec<(String, PendingInteraction)> {
        self.pending_interactions
            .write()
            .iter()
            .filter(|(_, p)| {
                p.belongs_to(&self.session_store) && p.kernel_operation_owner.is_some()
            })
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect()
    }

    /// The interaction backing a pending kernel operation. An ended or deleted Room
    /// cannot deliver an answer, even when the original caller is still waiting.
    pub(super) fn kernel_operation_card(
        &self,
        id: &str,
        pending: &PendingInteraction,
        session_of: &dyn Fn(&str) -> Option<RuntimeSession>,
    ) -> Option<RuntimeInteraction> {
        pending
            .kernel_wide_interaction
            .clone()
            .or_else(|| pending.user_domain_interaction.clone())
            .or_else(|| {
                let session = session_of(&pending.session_id)?;
                if session.status() == SessionStatus::Ended {
                    return None;
                }
                session
                    .active_interactions()
                    .iter()
                    .find(|card| card.id() == id)
                    .cloned()
            })
    }

    /// The counted subset of the pending store, shared by admission and owner views.
    pub(in crate::runtime::state) fn open_kernel_decision(
        &self,
        id: &str,
        pending: &PendingInteraction,
        now: std::time::Instant,
        session_of: &dyn Fn(&str) -> Option<RuntimeSession>,
    ) -> Option<RuntimeInteraction> {
        if !pending.belongs_to(&self.session_store)
            || pending.kernel_operation_owner.is_none()
            || !pending.decision_still_waits(now)
        {
            return None;
        }
        self.kernel_operation_card(id, pending, session_of)
    }

    pub(in crate::runtime::state) fn kernel_decision_owner_matches(
        &self,
        owner: &str,
        recorded_owner: &str,
    ) -> bool {
        let local = crate::session::DEFAULT_LOCAL_USER_ID;
        self.room_member_user_id_in(local, owner)
            == self.room_member_user_id_in(local, recorded_owner)
    }

    pub(super) fn ensure_kernel_decision_capacity(
        &self,
        owner: &str,
        interaction: &RuntimeInteraction,
        session_of: &dyn Fn(&str) -> Option<RuntimeSession>,
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
        let candidates = self.pending_kernel_decisions();
        let now = std::time::Instant::now();
        let decisions = candidates
            .iter()
            .filter_map(|(id, pending)| {
                self.open_kernel_decision(id, pending, now, session_of)
                    .map(|card| (pending, card))
            })
            .collect::<Vec<_>>();
        let own_decisions = decisions
            .iter()
            .filter(|(p, _)| {
                p.kernel_operation_owner
                    .as_deref()
                    .is_some_and(|recorded| self.kernel_decision_owner_matches(owner, recorded))
            })
            .collect::<Vec<_>>();
        // Keep the hard kernel bound and reserve its last slot for owner elevation.
        let owner_count = own_decisions
            .iter()
            .filter(|(p, _)| {
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
        if decisions.len() >= 32 || (!owner_elevation && decisions.len() >= 31) || owner_count >= 8
        {
            let holders = own_decisions
                .iter()
                .map(|(_, card)| format!("{} ({})", card.id(), card.title().unwrap_or("decision")))
                .collect::<Vec<_>>();
            return Err(interaction_error(&decision_limit_message(&holders)));
        }
        Ok(())
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
                codex_approval_transfer: None,
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
                codex_approval_transfer: None,
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
