//! MP-08 / MP-10 / MP-11: access decisions on the shared kernel interaction
//! board, independent of sessions. Answers, passkey checks, limits, expiry and
//! revocation use the ordinary interaction machinery.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_kernel_decision_capacity(&self, owner: &str) -> Result<(), DaemonError> {
        let pending = self.pending_interactions.write();
        let decisions = pending
            .values()
            .filter(|p| p.belongs_to(&self.session_store))
            .filter(|p| p.kernel_operation_owner.is_some())
            .collect::<Vec<_>>();
        if decisions.len() >= 32
            || decisions
                .iter()
                .filter(|p| p.kernel_operation_owner.as_deref() == Some(owner))
                .count()
                >= 8
        {
            return Err(interaction_error(super::KERNEL_DECISION_LIMIT));
        }
        Ok(())
    }

    pub(super) fn register_kernel_wide_access_interaction(
        &self,
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
        let scope = crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE;
        let prompt = super::super::passkey_prompts::passkey_prompt_for_scope(
            scope,
            None,
            &interaction,
            crate::session::unix_epoch_ms(),
        )?
        .ok_or_else(|| interaction_error("Access requires a passkey popup"))?;
        self.ensure_kernel_decision_capacity(owner)?;
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
                responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
                kernel_wide_interaction: Some(interaction),
            },
        );
        drop(pending);
        self.passkey_prompts.record_change();
        Ok(())
    }
}
