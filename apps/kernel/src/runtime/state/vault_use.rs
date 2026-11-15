//! MP-08 / MP-09 / MP-10 / MP-11: ordinary agent Vault operations.
//! Authentication identifies the actor; the stored use identifies the destination.
use super::*;
use crate::config::{PaymentCardField, UserCredentialConfig, VaultUseTarget, VaultValueKind};
use crate::runtime::kernel_access::error;

/// The longest an approved card payment lasts; the asking task ending ends it sooner.
const VAULT_PAYMENT_APPROVAL_WINDOW: Duration = Duration::from_secs(15 * 60);

/// How long a card payment decision waits for its owner while the asking task lives.
const VAULT_PAYMENT_DECISION_SECONDS: u64 = 24 * 60 * 60;

/// Every card payment decision's id starts with this.
const VAULT_PAYMENT_DECISION_PREFIX: &str = "vault-card-";

/// Whether a decision is a card payment's: only the owner answers it, and Allow always needs the
/// typed passkey, under any remember window, unlocked Vault or sudo.
pub(crate) fn is_vault_payment_decision(id: &str) -> bool {
    id.starts_with(VAULT_PAYMENT_DECISION_PREFIX)
}

/// Who asks for a Vault use and, for a payment card, which field and payment.
#[derive(Clone, Copy, Default)]
pub(super) struct VaultUseAsk<'a> {
    pub(super) agent_id: Option<&'a str>,
    pub(super) session_id: Option<&'a str>,
    pub(super) card_field: Option<PaymentCardField>,
    pub(super) claim: Option<&'a crate::local::VaultPaymentClaim>,
}

/// One card payment the owner was asked about. Kernel memory only: never serialized or shown to an agent.
#[derive(Clone, PartialEq)]
pub(super) struct VaultPayment {
    actor: String,
    credential_id: String,
    /// The saved card value and binding the owner is asked about.
    revision: crate::secret::VaultCardRevision,
    /// The site the kernel observed when the agent asked.
    target: VaultUseTarget,
    claim: Option<crate::local::VaultPaymentClaim>,
}

/// One approved card payment, for the task that asked.
pub(super) struct VaultPaymentGrant {
    approval_id: String,
    payment: VaultPayment,
    task_id: String,
    pub(super) expires_at: std::time::Instant,
}

/// What the agent learns when its fill waits on the owner: nothing was filled, and where the answer comes.
fn vault_payment_asked(id: &str, registration: &str) -> DaemonError {
    error(format!(
        "nothing was filled: the owner is asked to approve this payment (interaction {id}, \
         registration {registration}). Keep working; when the approval arrives in your inbox, \
         fill the card again for the same payment"
    ))
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
        VaultUseTarget::AnySite => "any site".into(),
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
    /// lets the same agent fill every field of that card for that payment (observed site and
    /// claimed merchant, amount and currency) until its turn ends or 15 minutes pass.
    /// Call before reading a value, then recheck the binding under the use barrier.
    pub(super) async fn authorize_vault_use(
        &self,
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
            || (card.is_some() && ask.claim.is_none())
            || (binding.value_kind != VaultValueKind::PaymentCard && ask.claim.is_some())
        {
            return Err(error(if card.is_some() {
                "fill a saved card one checkout field at a time: name it with card_field and describe the payment with payment {merchant, amount, currency}"
            } else {
                "card_field and payment are only for payment cards"
            }));
        }
        if let Some(claim) = ask.claim {
            claim.validate()?;
        }
        if binding.value_kind != VaultValueKind::PaymentCard {
            return Ok(crate::secret::VaultUseApproval::new(
                credential.id.clone(),
                binding.clone(),
                None,
                None,
            ));
        }
        let revision = {
            let _lifecycle = self.vault_lifecycle_read_guard().await;
            self.vault_card_revision(credential)?
        };
        let approval = crate::secret::VaultUseApproval::new(
            credential.id.clone(),
            binding.clone(),
            ask.card_field,
            Some(revision),
        );
        let payment = VaultPayment {
            actor: actor.to_owned(),
            credential_id: credential.id.clone(),
            revision,
            target: target.clone(),
            claim: ask.claim.cloned(),
        };
        let task = ask
            .session_id
            .zip(ask.agent_id)
            .and_then(|(session, agent)| self.live_vault_task(session, agent));
        if self.vault_payment_approved(&payment, task.as_deref()) {
            return Ok(approval);
        }
        let (Some(session_id), Some(agent_id)) = (ask.session_id, ask.agent_id) else {
            return Err(error(
                "a card payment is approved in the asking agent's Room; fill it from an agent turn",
            ));
        };
        // The same payment asked again while its card waits shows no second card.
        let waiting = self
            .vault_payment_asks()
            .iter()
            .find(|(_, asked)| **asked == payment)
            .map(|(id, _)| id.clone());
        if let Some(id) = waiting {
            return Err(vault_payment_asked(&id, &format!("completion-{id}")));
        }
        // The owner answers on a card in the asking agent's pane, as for any question that does
        // not block the turn: the agent keeps working, and the answer arrives in its inbox.
        let id = format!(
            "{VAULT_PAYMENT_DECISION_PREFIX}{:016x}",
            rand::random::<u64>()
        );
        let destination = vault_use_destination(target);
        let label = credential.description.as_deref().unwrap_or(&credential.id);
        let message = match ask.claim {
            Some(claim) => format!(
                "Allow {label} to be used at {destination}? The agent says: {} {} to {}.",
                claim.amount, claim.currency, claim.merchant
            ),
            None => format!("Allow {label} to be used at {destination}?"),
        };
        let interaction = crate::session::RuntimeInteraction::new(
            id.clone(),
            agent_id,
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Critical,
            Some("Use payment card".into()),
            message,
            vec![
                crate::session::RuntimeInteractionChoice::new("allow", "Allow once", "allow", None)
                    .requiring_passkey(),
                crate::session::RuntimeInteractionChoice::new(
                    "cancel",
                    "Cancel",
                    "cancel",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(VAULT_PAYMENT_DECISION_SECONDS),
            None,
        )
        .with_payment(crate::local::VaultPaymentApproval {
            origin: destination,
            entry: label.to_owned(),
            card_last4: card.map(|card| card.last4.clone()),
            card_brand: card.and_then(|card| card.brand.clone()),
            agent_claim: ask.claim.cloned(),
            requested_by_agent_id: Some(agent_id.to_owned()),
            session_id: Some(session_id.to_owned()),
        });
        self.vault_payment_asks().insert(id.clone(), payment);
        let asked = match self.arm_interaction_wake(session_id, agent_id, interaction) {
            Ok(asked) => asked,
            Err(refused) => {
                self.vault_payment_asks().remove(&id);
                return Err(refused);
            }
        };
        let registration = asked.payload["registration_id"]
            .as_str()
            .unwrap_or_default();
        Err(vault_payment_asked(&id, registration))
    }

    /// The saved card's revision, if the registered entry still shows what `shown` shows.
    /// Call under the Vault lifecycle guard.
    fn vault_card_revision(
        &self,
        shown: &UserCredentialConfig,
    ) -> Result<crate::secret::VaultCardRevision, DaemonError> {
        let service = self.home_runtime_secret_service()?;
        let (current, revision) = service.vault_card_revision(&shown.id)?;
        let summary = |entry: &UserCredentialConfig| {
            (
                entry.description.clone(),
                entry.source.clone(),
                entry
                    .metadata
                    .as_ref()
                    .map(|m| (m.use_binding.clone(), m.payment_card.clone())),
            )
        };
        if summary(current) != summary(shown) {
            return Err(error(
                "this payment card changed; nothing was filled. Fill again to ask the owner",
            ));
        }
        Ok(revision)
    }

    fn vault_payment_grants(&self) -> std::sync::MutexGuard<'_, Vec<VaultPaymentGrant>> {
        self.owned.vault_payment_grants()
    }

    fn vault_payment_asks(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, VaultPayment>> {
        self.owned.vault_payment_asks()
    }

    /// The agent's running turn, if it has one that has not ended.
    fn live_vault_turn(&self, session_id: &str, agent_id: &str) -> Option<String> {
        let session = self.owned.session_store.get_session(session_id).ok()?;
        self.owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| {
                !matches!(
                    prompt.status(),
                    crate::session::PromptStatus::Cancelling
                        | crate::session::PromptStatus::Cancelled
                        | crate::session::PromptStatus::Completed
                )
            })
            .map(|prompt| prompt.id().to_owned())
    }

    /// Whether the owner approved this payment for this agent's current task: the same saved card
    /// value, observed site and claimed merchant, amount and currency, not yet expired.
    fn vault_payment_approved(&self, payment: &VaultPayment, task: Option<&str>) -> bool {
        let Some(task) = task else {
            return false;
        };
        self.vault_payment_grants().iter().any(|grant| {
            let granted = &grant.payment;
            granted.actor == payment.actor
                && granted.credential_id == payment.credential_id
                && granted.revision == payment.revision
                && granted.target == payment.target
                && granted.claim == payment.claim
                && grant.task_id == task
        })
    }

    /// The task of the agent's running turn.
    fn live_vault_task(&self, session_id: &str, agent_id: &str) -> Option<String> {
        let prompt = self.live_vault_turn(session_id, agent_id)?;
        self.owned
            .durable_state_store
            .agent_tasks(Some(session_id), Some(agent_id))
            .ok()?
            .into_iter()
            .find(|task| task.prompt_id == prompt)
            .map(|task| task.task_id)
    }

    /// Private browser authority is re-read on each action, never supplied by a tool caller.
    pub(super) fn vault_browser_payment_approvals(&self, agent_id: &str) -> Vec<serde_json::Value> {
        let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
            return Vec::new();
        };
        let Some(task) = self.live_vault_task(agent.session_id(), agent_id) else {
            return Vec::new();
        };
        let now = std::time::Instant::now();
        let wall = crate::session::unix_epoch_ms();
        self.vault_payment_grants()
            .iter()
            .filter(|grant| grant.payment.actor == agent_id && grant.task_id == task && grant.expires_at > now)
            .filter_map(|grant| {
                let VaultUseTarget::Site { origin } = &grant.payment.target else {
                    return None;
                };
                Some(serde_json::json!({
                    "id": grant.approval_id,
                    "credential_id": grant.payment.credential_id,
                    "origin": origin,
                    "claim": grant.payment.claim,
                    "expires_at_ms": wall.saturating_add(grant.expires_at.saturating_duration_since(now).as_millis() as u64)
                }))
            })
            .collect()
    }

    /// A replaced or deleted card ends the payments approved for it.
    pub(super) fn forget_vault_payment_grants(&self, credential_id: &str) {
        self.vault_payment_grants()
            .retain(|grant| grant.payment.credential_id != credential_id);
    }
}

impl KernelRuntimeOwnedState {
    fn vault_payment_grants(&self) -> std::sync::MutexGuard<'_, Vec<VaultPaymentGrant>> {
        let mut grants = self
            .vault_payment_grants
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        grants.retain(|grant| grant.expires_at > now);
        grants
    }

    fn vault_payment_asks(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, VaultPayment>> {
        self.vault_payment_asks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The owner's answer to a card decision: Allow (with the passkey) approves that one payment
    /// for the task that asked; any other end approves nothing.
    pub(super) fn settle_vault_payment(
        &self,
        interaction: &crate::session::RuntimeInteraction,
        resolution: &super::PendingInteractionResolution,
    ) {
        let Some(payment) = self.vault_payment_asks().remove(interaction.id()) else {
            return;
        };
        let Some(task_id) = interaction.asking_task_id() else {
            return;
        };
        if resolution.status == "answered" && resolution.choice_id.as_deref() == Some("allow") {
            self.vault_payment_grants().push(VaultPaymentGrant {
                approval_id: interaction.id().to_owned(),
                payment,
                task_id: task_id.to_owned(),
                expires_at: std::time::Instant::now() + VAULT_PAYMENT_APPROVAL_WINDOW,
            });
        }
    }

    /// Whether this kernel holds what the card decision `interaction_id` approves.
    pub(super) fn vault_payment_asked(&self, interaction_id: &str) -> bool {
        self.vault_payment_asks().contains_key(interaction_id)
    }

    /// A withdrawn card decision approves nothing.
    pub(super) fn forget_vault_payment_ask(&self, interaction_id: &str) {
        self.vault_payment_asks().remove(interaction_id);
    }
}
