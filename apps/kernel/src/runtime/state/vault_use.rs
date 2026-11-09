//! MP-08 / MP-09 / MP-10 / MP-11: ordinary agent Vault operations.
//! Authentication identifies the actor; the stored use identifies the destination.
use super::*;
use crate::config::{
    PaymentCardField, UserCredentialConfig, VaultUseBinding, VaultUseTarget, VaultValueKind,
};
use crate::runtime::kernel_access::error;

/// How long the fields of one approved card payment may be filled.
const VAULT_PAYMENT_FILL_WINDOW: Duration = Duration::from_secs(120);

/// Who asks for a Vault use and, for a payment card, which field and payment.
#[derive(Clone, Copy, Default)]
pub(super) struct VaultUseAsk<'a> {
    pub(super) agent_id: Option<&'a str>,
    pub(super) session_id: Option<&'a str>,
    pub(super) card_field: Option<PaymentCardField>,
    pub(super) claim: Option<&'a crate::local::VaultPaymentClaim>,
}

/// One approved card payment. Kernel memory only: never serialized or shown to an agent.
pub(super) struct VaultPaymentGrant {
    actor: String,
    credential_id: String,
    binding: VaultUseBinding,
    filled: Vec<PaymentCardField>,
    pub(super) expires_at: std::time::Instant,
}

fn vault_use_destination(target: &VaultUseTarget) -> String {
    match target {
        VaultUseTarget::Site { origin }
        | VaultUseTarget::CommandStdin { origin }
        | VaultUseTarget::Totp { origin }
        | VaultUseTarget::WebAuthn { origin, .. } => origin.clone(),
        VaultUseTarget::CommandEnvironment { name, origin } => {
            format!("{name} for {origin}")
        }
        VaultUseTarget::File { path } => path.clone(),
    }
}

#[derive(Clone)]
pub(super) struct VaultAgentContext {
    pub(super) run: crate::provider::RuntimeProviderRun,
    pub(super) agent_id: String,
    pub(super) session_id: String,
    pub(super) owner_user_id: String,
    pub(super) prompt_id: Option<String>,
}

impl KernelRuntimeState {
    pub(super) fn vault_agent_context(
        &self,
        token: &str,
    ) -> Result<VaultAgentContext, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(error(
                "Vault operations require one authenticated provider run",
            ));
        };
        self.vault_agent_context_for_run(run)
    }

    pub(super) fn vault_agent_context_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<VaultAgentContext, DaemonError> {
        let agent_id = run
            .agent_instance_id()
            .ok_or_else(|| error("Vault operations require an agent"))?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(run.session_id())?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id);
        if prompt.as_ref().is_some_and(|p| {
            matches!(
                p.status(),
                crate::session::PromptStatus::Cancelling
                    | crate::session::PromptStatus::Cancelled
                    | crate::session::PromptStatus::Completed
            )
        }) {
            return Err(error("the Vault operation's submitting prompt ended"));
        }
        if run.state() == crate::provider::ProviderRunState::Ended
            || agent.session_id() != session.id()
            || agent.owner_user_id() != session.owner_user_id()
            || self.provider_account_authority_owner_user_id(session.owner_user_id())
                != crate::session::DEFAULT_LOCAL_USER_ID
        {
            return Err(error(
                "the provider run no longer belongs to the owner's agent",
            ));
        }
        Ok(VaultAgentContext {
            run: run.clone(),
            agent_id: agent_id.into(),
            session_id: session.id().into(),
            owner_user_id: session.owner_user_id().into(),
            prompt_id: self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .map(|prompt| prompt.id().to_owned()),
        })
    }

    /// Recheck after an asynchronous unlock, command or lifecycle barrier.
    pub(super) fn recheck_vault_agent(
        &self,
        context: &VaultAgentContext,
    ) -> Result<(), DaemonError> {
        self.authorize_current_external_command()?;
        let current = match context.run.runtime_mcp_auth_token() {
            Some(token) => self.vault_agent_context(token)?,
            None => self.vault_agent_context_for_run(
                &self.owned.provider_store.get_run(context.run.id())?,
            )?,
        };
        if current.run.id() != context.run.id()
            || current.agent_id != context.agent_id
            || current.session_id != context.session_id
            || current.owner_user_id != context.owner_user_id
            || current.prompt_id != context.prompt_id
        {
            return Err(error(
                "the Vault operation's provider run changed; retry from the current run",
            ));
        }
        Ok(())
    }

    /// Invalid requests never ask for an unlock. Rejections can be encrypted in
    /// Activity only while the Vault is already open; no value is read on this path.
    pub(super) fn record_vault_refusal(
        &self,
        id: &str,
        target: &VaultUseTarget,
        actor: &str,
    ) -> Result<(), DaemonError> {
        if self.require_unlocked_vault().is_ok() {
            self.home_runtime_secret_service()?.record_vault_activity(
                id,
                &format!(
                    "Use refused for {} by {actor}",
                    vault_use_destination(target)
                ),
            )?;
        }
        Ok(())
    }

    pub(super) fn preflight_workspace_vault_use(
        &self,
        actor: &str,
        id: &str,
        target: &VaultUseTarget,
    ) -> Result<(), DaemonError> {
        if let Err(error) = self
            .home_runtime_secret_service()?
            .validate_workspace_secret_target(id, target)
        {
            self.record_vault_refusal(id, target, actor)?;
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn read_workspace_vault_value(
        &self,
        context: &VaultAgentContext,
        id: &str,
        target: &VaultUseTarget,
    ) -> Result<zeroize::Zeroizing<String>, DaemonError> {
        self.preflight_workspace_vault_use(&context.agent_id, id, target)?;
        let credential = self.home_runtime_secret_service()?.credential(id)?.clone();
        let approval = self
            .authorize_vault_use(
                &context.owner_user_id,
                &context.agent_id,
                &credential,
                target,
                VaultUseAsk {
                    agent_id: Some(&context.agent_id),
                    session_id: Some(&context.session_id),
                    ..Default::default()
                },
            )
            .await?;
        self.recheck_vault_agent(context)?;
        let service = self
            .home_runtime_secret_service()?
            .with_vault_use_approval(approval);
        let attempt = service.begin_vault_use(
            id,
            &format!(
                "Use attempted at {} by {}",
                vault_use_destination(target),
                context.agent_id
            ),
        )?;
        let value = zeroize::Zeroizing::new(service.workspace_secret_input_for_target(id, target)?);
        attempt.finish(&format!(
            "Used at {} by {}",
            vault_use_destination(target),
            context.agent_id
        ))?;
        Ok(value)
    }

    /// No consent for ordinary values. A card gets one owner decision per payment: Allow once
    /// lets the same agent fill each field of that card at that destination once, briefly.
    /// Call before reading a value, then recheck the binding under the use barrier.
    pub(super) async fn authorize_vault_use(
        &self,
        owner: &str,
        actor: &str,
        credential: &UserCredentialConfig,
        target: &VaultUseTarget,
        ask: VaultUseAsk<'_>,
    ) -> Result<crate::secret::VaultUseApproval, DaemonError> {
        let binding = credential
            .metadata
            .as_ref()
            .and_then(|m| m.use_binding.as_ref())
            .ok_or_else(|| {
                error(
                    "this Vault entry has no declared use; create it for one specific destination",
                )
            })?;
        if let Err(error) = binding.require(target) {
            self.record_vault_refusal(&credential.id, target, actor)?;
            return Err(error);
        }
        let card = credential
            .metadata
            .as_ref()
            .and_then(|m| m.payment_card.as_ref());
        if card.is_some() != ask.card_field.is_some()
            || (binding.value_kind != VaultValueKind::PaymentCard && ask.claim.is_some())
        {
            return Err(error(if card.is_some() {
                "fill a saved card one checkout field at a time: name it with card_field"
            } else {
                "card_field and payment are only for payment cards"
            }));
        }
        if let Some(claim) = ask.claim {
            claim.validate()?;
        }
        let approval = crate::secret::VaultUseApproval::new(
            credential.id.clone(),
            binding.clone(),
            ask.card_field,
        );
        if binding.value_kind != VaultValueKind::PaymentCard
            || self.take_vault_payment_grant(actor, credential, binding, ask.card_field)
        {
            return Ok(approval);
        }
        let id = format!("vault-card-{:016x}", rand::random::<u64>());
        let destination = vault_use_destination(target);
        let label = credential.description.as_deref().unwrap_or(&credential.id);
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            "Use payment card",
            format!("Allow {label} to be used at {destination}?"),
            vec![
                crate::session::RuntimeInteractionChoice::new("allow", "Allow once", "allow", None)
                    .requiring_passkey(),
                crate::session::RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
            ],
        )
        .with_timeout_sec(300)
        .with_payment(crate::local::VaultPaymentApproval {
            origin: destination,
            entry: label.to_owned(),
            card_last4: card.map(|card| card.last4.clone()),
            card_brand: card.and_then(|card| card.brand.clone()),
            claim: ask.claim.cloned(),
            requested_by_agent_id: ask.agent_id.map(str::to_owned),
            session_id: ask.session_id.map(str::to_owned),
        });
        let scope = "";
        let receiver = self
            .create_kernel_operation_interaction(scope, owner, interaction)
            .await?;
        let resolution = match tokio::time::timeout(Duration::from_secs(300), receiver).await {
            Ok(Ok(resolution)) => resolution,
            _ => {
                let _ = self.timeout_runtime_interaction(scope, &id).await;
                return Err(error("payment card approval ended; try again"));
            }
        };
        if resolution.choice_id.as_deref() != Some("allow") {
            return Err(error("payment card use was not approved"));
        }
        if let Some(field) = ask.card_field {
            let mut grants = self.vault_payment_grants();
            grants.push(VaultPaymentGrant {
                actor: actor.to_owned(),
                credential_id: credential.id.clone(),
                binding: binding.clone(),
                filled: vec![field],
                expires_at: std::time::Instant::now() + VAULT_PAYMENT_FILL_WINDOW,
            });
        }
        Ok(approval)
    }

    fn vault_payment_grants(&self) -> std::sync::MutexGuard<'_, Vec<VaultPaymentGrant>> {
        let mut grants = self
            .owned
            .vault_payment_grants
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        grants.retain(|grant| grant.expires_at > now);
        grants
    }

    /// Uses a field of a live approved payment by the same agent, at most once.
    fn take_vault_payment_grant(
        &self,
        actor: &str,
        credential: &UserCredentialConfig,
        binding: &VaultUseBinding,
        field: Option<PaymentCardField>,
    ) -> bool {
        let Some(field) = field else {
            return false;
        };
        self.vault_payment_grants()
            .iter_mut()
            .find(|grant| {
                grant.actor == actor
                    && grant.credential_id == credential.id
                    && &grant.binding == binding
                    && !grant.filled.contains(&field)
            })
            .map(|grant| grant.filled.push(field))
            .is_some()
    }

    /// A replaced or deleted card ends the payments approved for it.
    pub(super) fn forget_vault_payment_grants(&self, credential_id: &str) {
        self.vault_payment_grants()
            .retain(|grant| grant.credential_id != credential_id);
    }
}
