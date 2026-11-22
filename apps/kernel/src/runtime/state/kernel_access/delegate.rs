//! An external agent answers the person's cards on their behalf under the access the person
//! already gave it. Secret entry, Vault unlock, passkeys and payments stay the person's own
//! (`PendingInteraction::person_only`).
use super::*;
use crate::runtime::command::KernelCaller;
use crate::runtime::state::PendingInteraction;

/// An answer an external agent gives for the person: its local grant, the same agent forwarded
/// through the person's other kernel, or an agent driving this Room through reach.
fn answers_for_person(answerer: &KernelCaller) -> bool {
    answerer.connection_class == Some(KernelConnectionClass::ExternalAgent)
        || (answerer.connection_class == Some(KernelConnectionClass::KernelAgent)
            && answerer.caller_id.starts_with(super::reach::REACH_PREFIX))
}

impl KernelRuntimeState {
    /// The name of the agent answering for the person in this state, if one is.
    pub(in crate::runtime::state) fn answering_agent(&self) -> Option<String> {
        let answerer = self
            .interaction_answerer
            .as_ref()
            .filter(|answerer| answers_for_person(answerer))?;
        let name = match self.access_grant_summary(&answerer.caller_id) {
            Some(grant) => {
                crate::runtime::kernel_access::requester::harness_name(grant.holder_harness)
                    .map_or_else(
                        || crate::runtime::kernel_gateway::harness_name(&grant),
                        str::to_owned,
                    )
            }
            None => self
                .gateway_origin(&answerer.caller_id)
                .map_or_else(|| "An external agent".to_owned(), |origin| origin.harness),
        };
        Some(name)
    }

    /// The person's open cards, oldest first, each with why only the person answers it, if so.
    pub(crate) fn list_owner_interactions(&self, owner: &str) -> Vec<OwnerInteraction> {
        let now = std::time::Instant::now();
        let pending = self
            .owned
            .pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| pending.belongs_to(&self.owned.session_store))
            .map(|(id, pending)| (id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        let mut cards = pending
            .into_iter()
            .filter_map(|(id, pending)| {
                let interaction = if let Some(recorded) = &pending.kernel_operation_owner {
                    if !self.owned.kernel_decision_owner_matches(owner, recorded) {
                        return None;
                    }
                    self.owned
                        .open_kernel_decision(&id, &pending, now, &|session| {
                            self.owned.session_store.get_session(session).ok()
                        })?
                } else {
                    self.owner_card(owner, &id, &pending)?
                };
                let reply_choice = interaction.custom_choice().filter(|custom| {
                    custom.input_kind() == crate::session::RuntimeInteractionInputKind::Text
                });
                Some((
                    interaction.requested_at_ms(),
                    OwnerInteraction {
                        session_id: (!pending.session_id.is_empty())
                            .then(|| pending.session_id.clone()),
                        interaction_id: id,
                        title: interaction.title().map(str::to_owned),
                        message: interaction.message().to_owned(),
                        choices: interaction
                            .choices()
                            .iter()
                            .map(|choice| OwnerInteractionChoice {
                                id: choice.id().to_owned(),
                                label: choice.label().to_owned(),
                                description: choice.description().map(str::to_owned),
                            })
                            .collect(),
                        reply_choice_id: reply_choice.map(|choice| choice.id().to_owned()),
                        reply_choice_description: reply_choice
                            .and_then(|choice| choice.description())
                            .map(str::to_owned),
                        person_only: pending.person_only(&interaction).map(str::to_owned),
                    },
                ))
            })
            .collect::<Vec<_>>();
        cards.sort_by_key(|(at, _)| *at);
        cards.into_iter().map(|(_, card)| card).collect()
    }

    /// The card `id`, when it is open and belongs to `owner`: a card of one of their Rooms,
    /// whichever agent asked, or a kernel-wide card of theirs.
    fn owner_card(
        &self,
        owner: &str,
        id: &str,
        pending: &PendingInteraction,
    ) -> Option<RuntimeInteraction> {
        let session = self
            .owned
            .session_store
            .get_session(&pending.session_id)
            .ok();
        let card_owner = pending
            .kernel_operation_owner
            .as_deref()
            .or(pending.terminal_credential_owner.as_deref())
            .or(session.as_ref().map(|session| session.owner_user_id()))?;
        if !self.access_owned_by(owner, card_owner) {
            return None;
        }
        pending
            .kernel_wide_interaction
            .clone()
            .or_else(|| pending.user_domain_interaction.clone())
            .or_else(|| {
                session?
                    .active_interactions()
                    .iter()
                    .find(|interaction| interaction.id() == id)
                    .cloned()
            })
    }

    /// The card an agent answers for the person: refused, with the reason, when it is not the
    /// person's or the person keeps it.
    pub(in crate::runtime::state) fn delegated_card(
        &self,
        owner: Option<&str>,
        interaction_id: &str,
    ) -> Result<RuntimeInteraction, DaemonError> {
        let pending = self
            .owned
            .pending_interactions
            .write()
            .get(interaction_id)
            .cloned()
            .ok_or_else(crate::runtime::state::passkey_prompts::interaction_closed)?;
        let interaction = owner
            .and_then(|owner| self.owner_card(owner, interaction_id, &pending))
            .ok_or_else(|| error("this card is not the person's you act for"))?;
        if let Some(reason) = pending.person_only(&interaction) {
            return Err(error(format!(
                "the person must answer this card in a Chariox client: {reason}"
            )));
        }
        Ok(interaction)
    }

    /// The card's Room shows every client who answered it for the person.
    pub(in crate::runtime::state) fn record_delegated_answer(
        &self,
        session_id: &str,
        agent: &str,
        interaction: &RuntimeInteraction,
        choice_id: &str,
    ) {
        let card = interaction.title().unwrap_or("a card");
        let choice = interaction
            .choice(choice_id)
            .map_or(choice_id, |choice| choice.label());
        self.owned.record_notice(
            session_id,
            None,
            self.owned
                .attachment_store
                .list_session_attachment_ids(session_id),
            format!("{agent} answered \"{card}\" on the owner's behalf: {choice}."),
        );
    }
}

impl crate::runtime::state::KernelRuntimeOwnedState {
    /// Whether `answerer` is an external agent answering for the person.
    pub(in crate::runtime::state) fn answers_for_person(
        &self,
        answerer: Option<&KernelCaller>,
    ) -> bool {
        answerer.is_some_and(answers_for_person)
    }
}
