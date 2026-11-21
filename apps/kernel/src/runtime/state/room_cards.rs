//! Owner room reach: public agent state, pending cards, and non-secret answers.
use super::*;
use crate::agent::AgentInstance;
use crate::runtime::room_tool_admission::denied;
use crate::session::{RuntimeInteraction, RuntimeSession};

/// Agent answers are checked under the same lock that settles the interaction.
#[derive(Clone, Copy)]
pub(super) enum AgentInteractionAnswer<'a> {
    Holder(&'a str),
    Owner(&'a str),
}

impl KernelRuntimeOwnedState {
    pub(super) fn authorize_owner_room(
        &self,
        source: &RuntimeSession,
        caller: &AgentInstance,
        target: &RuntimeSession,
    ) -> Result<(), DaemonError> {
        if self.drives_as_room_guest(source, caller) {
            return Err(denied(
                "a guest's agent cannot inspect or answer another room's cards",
            ));
        }
        let owner =
            |id: &str| self.room_member_user_id_in(crate::session::DEFAULT_LOCAL_USER_ID, id);
        if caller.archived()
            || target.is_hidden()
            || target.status() == crate::session::SessionStatus::Ended
            || owner(source.owner_user_id()) != owner(target.owner_user_id())
        {
            return Err(denied("room is unavailable to this owner"));
        }
        Ok(())
    }

    pub(super) fn authorize_owner_card(
        &self,
        rooms: &crate::session::SessionService,
        source: &RuntimeSession,
        caller: &AgentInstance,
        target: &RuntimeSession,
        pending: &PendingInteraction,
        interaction: &RuntimeInteraction,
    ) -> Result<(), DaemonError> {
        self.authorize_owner_room(source, caller, target)?;
        if super::agent_deadline_cards::is_deadline_card(interaction.id()) {
            return Err(denied("This card is owner-only. Authorized agents can extend the limit with agent deadline."));
        }
        use super::decision_holder::UserOnly;
        match super::decision_holder::user_only(pending, interaction) {
            None | Some(UserOnly::AccessGrant | UserOnly::KernelOperation)
                if pending.passkey_prompt.is_none()
                    && !interaction
                        .choices()
                        .iter()
                        .any(crate::session::RuntimeInteractionChoice::requires_passkey) => {}
            _ => {
                return Err(denied(
                    "this card is owner-only; secrets, passkeys, payments and owner verification must be answered by the person",
                ));
            }
        }
        if !self.ancestor_can_hold(rooms, caller) {
            return Err(denied(
                "your current authority requires the owner's answer to this card",
            ));
        }
        let mine = self.decision_execution_config(rooms, source, caller);
        if let Some(subject) = interaction
            .agent_id()
            .or_else(|| interaction.related_agent_id())
        {
            let agents = self.agent_store.get_session_agents(target.id());
            let subject = agents
                .iter()
                .find(|agent| agent.id() == subject)
                .ok_or_else(|| denied("the asking agent is unavailable"))?;
            let theirs = self.decision_execution_config(rooms, target, subject);
            if !mine.permission_level.permits(theirs.permission_level)
                || !mine.mode.permits(theirs.mode)
            {
                return Err(denied(
                    "the asking agent has more authority than you; the owner must answer",
                ));
            }
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(crate) fn owner_room_cards(
        &self,
        caller_id: &str,
        room: &str,
        after: Option<&str>,
    ) -> Result<serde_json::Value, DaemonError> {
        // Leave space under the provider result limit for JSON escaping and envelope fields.
        const PAGE_BYTES: usize = 16 * 1024;
        const PAGE_CARDS: usize = 12;
        let caller = self.owned.agent_store.get_agent(caller_id)?;
        let source = self.owned.session_store.get_session(caller.session_id())?;
        let target = self.owned.session_store.get_session(room)?;
        self.owned.authorize_owner_room(&source, &caller, &target)?;
        let rooms = self.owned.session_store.read();
        let pending = self.owned.pending_interactions.write();
        let mut open = target
            .active_interactions()
            .iter()
            .filter(|card| after.is_none_or(|after| card.id() > after))
            .filter_map(|card| {
                let p = pending
                    .get(card.id())
                    .filter(|p| p.session_id == target.id())?;
                Some((card, p))
            })
            .collect::<Vec<_>>();
        open.sort_unstable_by_key(|(card, _)| card.id());

        let mut cards = Vec::new();
        let mut bytes = 0;
        let mut last = None;
        let mut next_after = None;
        for (card, p) in open {
            let refusal = self
                .owned
                .authorize_owner_card(&rooms, &source, &caller, &target, p, card)
                .err();
            let mut row = serde_json::json!({
                "id": card.id(),
                "agent_id": card.agent_id().or_else(|| card.related_agent_id()),
                "kind": card.kind(),
                "level": card.level(),
                "title": card.title(),
                "message": card.message(),
                "choices": card.choices().iter().map(|choice| serde_json::json!({"id":choice.id(), "label":choice.label()})).collect::<Vec<_>>(),
                "custom_choice": card.custom_choice().filter(|c| c.input_kind() == crate::session::RuntimeInteractionInputKind::Text),
                "answerable": refusal.is_none(),
                "reason": refusal.as_ref().map(ToString::to_string),
            });
            let mut size = row.to_string().len() + 1;
            if size > PAGE_BYTES {
                row = serde_json::json!({
                    "id": card.id(),
                    "answerable": false,
                    "reason": refusal.map_or_else(
                        || "This card is too large to read here. Open it in the Room before answering.".to_string(),
                        |error| error.to_string(),
                    ),
                });
                size = row.to_string().len() + 1;
            }
            if cards.len() == PAGE_CARDS || bytes + size > PAGE_BYTES {
                next_after = last;
                break;
            }
            last = Some(card.id());
            bytes += size;
            cards.push(row);
        }
        Ok(serde_json::json!({"room_id": target.id(), "cards": cards, "next_after": next_after}))
    }

    pub(crate) fn owner_room_agents(
        &self,
        caller_id: &str,
        room: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let caller = self.owned.agent_store.get_agent(caller_id)?;
        let source = self.owned.session_store.get_session(caller.session_id())?;
        let target = self.owned.session_snapshot(room)?;
        self.owned.authorize_owner_room(&source, &caller, &target)?;
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(target.id()), None)?;
        let agents = target.agents().iter().filter(|agent| agent.parent_agent_id().is_none() && agent.visible_in_freeform()).map(|agent| {
            let task = tasks.iter().filter(|task| task.agent_id == agent.id()).max_by_key(|task| task.last_progress_at_ms)
                .map(|task| serde_json::json!({"task_id":task.task_id,"prompt_id":task.prompt_id,"state":task.state}));
            serde_json::json!({
                "id":agent.id(),"agent_ref":agent.agent_ref(),"alias":agent.alias(),
                "state":agent.state(),"is_processing":agent.is_processing(),
                "active_prompt_id":target.active_prompt_for_agent(agent.id()).map(|prompt|prompt.id()),
                "task":task,
            })
        }).collect::<Vec<_>>();
        Ok(serde_json::json!({"room_id":target.id(),"agents":agents}))
    }

    pub(crate) fn answer_owner_room_card(
        &self,
        caller_id: &str,
        room: &str,
        interaction_id: &str,
        answer: &str,
    ) -> Result<ChildDecisionStatus, DaemonError> {
        let caller = self.owned.agent_store.get_agent(caller_id)?;
        let source = self.owned.session_store.get_session(caller.session_id())?;
        let target = self.owned.session_store.get_session(room)?;
        self.owned.authorize_owner_room(&source, &caller, &target)?;
        let Some(interaction) = target
            .active_interactions()
            .iter()
            .find(|i| i.id() == interaction_id)
        else {
            return Ok(ChildDecisionStatus::UnknownInteraction);
        };
        let (choice, reply) = if interaction.choice(answer).is_some() {
            (answer.to_owned(), None)
        } else if let Some(custom) = interaction.custom_choice() {
            (custom.id().to_owned(), Some(answer))
        } else {
            return Ok(ChildDecisionStatus::InvalidAnswer);
        };
        let owner = self.owner_identity_in(room, caller.owner_user_id());
        let answerer = crate::runtime::command::KernelCaller {
            caller_id: caller.id().into(),
            caller_kind: crate::runtime::command::KernelCallerKind::Metaagent,
            user_id: Some(caller.owner_user_id().into()),
            metaagent_id: Some(caller.id().into()),
            ..Default::default()
        };
        self.owned.resolve_runtime_interaction_routed(
            room,
            interaction_id,
            &choice,
            reply,
            Some(&owner),
            false,
            None,
            false,
            false,
            Some(&answerer),
            Some(AgentInteractionAnswer::Owner(caller_id)),
        )?;
        self.audit_interaction_answered(
            room,
            interaction_id,
            &choice,
            None,
            Some(
                super::runtime_interaction_state::AgentDecisionAnswer::RoomAgent {
                    agent: caller_id,
                    room: caller.session_id(),
                },
            ),
        );
        self.schedule_question_answer_delivery(interaction_id)?;
        Ok(ChildDecisionStatus::Answered)
    }
}
