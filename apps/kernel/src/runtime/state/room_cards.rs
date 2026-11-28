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
        self.refuse_self_approval(interaction, caller)?;
        if super::agent_deadline_status::is_deadline_status(interaction.id()) {
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
        if !self.ancestor_can_hold(rooms, caller, false) {
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
    /// Read just enough to identify the owner's kernel decisions. The terminal still owns
    /// presentation and answer authority, including for secrets and passkeys.
    pub(crate) fn owner_kernel_decisions(
        &self,
        source: &RuntimeSession,
        caller: &AgentInstance,
    ) -> Result<serde_json::Value, DaemonError> {
        if self.owned.drives_as_room_guest(source, caller) {
            return Err(denied(
                "a guest's agent cannot inspect the owner's kernel decisions",
            ));
        }
        let now = std::time::Instant::now();
        let now_ms = crate::session::unix_epoch_ms();
        let mut cards = self
            .owned
            .pending_kernel_decisions()
            .into_iter()
            .filter(|(_, pending)| {
                pending
                    .kernel_operation_owner
                    .as_deref()
                    .is_some_and(|owner| {
                        self.owned
                            .kernel_decision_owner_matches(source.owner_user_id(), owner)
                    })
            })
            .filter_map(|(id, pending)| {
                let card = self
                    .owned
                    .open_kernel_decision(&id, &pending, now, &|room| {
                        self.owned.session_store.get_session(room).ok()
                    })?;
                let kind = if let Some(prompt) = &pending.passkey_prompt {
                    serde_json::json!(prompt.kind)
                } else if id.starts_with("machine-acquisition-") {
                    serde_json::json!("machine_acquisition")
                } else {
                    serde_json::json!(card.kind())
                };
                let requester = card
                    .agent_id()
                    .or_else(|| card.related_agent_id())
                    .map(|agent_id| serde_json::json!({"agent_id": agent_id}))
                    .or_else(|| {
                        card.requester().map(|process| {
                        serde_json::json!({"pid": process.pid, "harness": process.provider_harness})
                    })
                    });
                let requested_at = card.requested_at_ms();
                let row = serde_json::json!({
                    "id": id,
                    "kind": kind,
                    "age_ms": now_ms.saturating_sub(requested_at),
                    "requester": requester,
                });
                Some((requested_at, row))
            })
            .collect::<Vec<_>>();
        cards.sort_by_key(|(requested_at, _)| *requested_at);
        Ok(serde_json::json!({
            "cards": cards.into_iter().map(|(_, card)| card).collect::<Vec<_>>()
        }))
    }

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

    /// Why a card is not taking the person's input, for the agent that raised it (or any agent of
    /// the owner's rooms): the Tab, generation and document it is bound to, what the browser shows
    /// now, who holds the Tab's input, and the last input taken and refused. Names and times only.
    pub(crate) fn owner_room_card(
        &self,
        caller_id: &str,
        card_id: &str,
        room: Option<&str>,
    ) -> Result<serde_json::Value, DaemonError> {
        let caller = self.owned.agent_store.get_agent(caller_id)?;
        let source = self.owned.session_store.get_session(caller.session_id())?;
        let target = match room {
            Some(room) => self.owned.session_snapshot(room)?,
            None => source.clone(),
        };
        self.owned.authorize_owner_room(&source, &caller, &target)?;
        let interaction = target
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == card_id)
            .ok_or_else(|| {
                denied("no pending card with that id in the room; list them with `room cards`")
            })?;
        let browser = interaction.view().map(|view| {
            let bound = &view.browser_region;
            let user = self.provider_account_authority_owner_user_id(caller.owner_user_id());
            let tab = self
                .owned
                .kernel_browser_host
                .card_tab_state(&user, &bound.tab_id);
            let (taken, refused) = self.browser_input_log.read(&bound.tab_id);
            let now_ms = crate::session::unix_epoch_ms();
            let outcome = |outcome: Option<super::browser_input_log::InputOutcome>| {
                outcome.map(|outcome| {
                    let mut row = serde_json::to_value(&outcome).unwrap_or_default();
                    row["age_ms"] = now_ms.saturating_sub(outcome.at_ms).into();
                    row
                })
            };
            let findings =
                card_findings(bound, tab.as_ref().ok(), taken.as_ref(), refused.as_ref());
            serde_json::json!({
                "bound": {
                    "tab_id": bound.tab_id,
                    "ref": bound.reference,
                    "generation": bound.generation,
                    "document_id": bound.document_id,
                    "origin": bound.origin,
                },
                "tab": match &tab {
                    Ok(state) => state.clone(),
                    Err(reason) => serde_json::json!({"unavailable": reason}),
                },
                "input": {"last_taken": outcome(taken), "last_refused": outcome(refused)},
                "findings": findings,
            })
        });
        Ok(serde_json::json!({
            "id": interaction.id(),
            "room_id": target.id(),
            "agent_id": interaction.agent_id().or_else(|| interaction.related_agent_id()),
            "kind": interaction.kind(),
            "browser": browser,
        }))
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

/// What the numbers say, in words: each finding names what is wrong and what it means for the
/// card, so the agent need not compare generations and documents itself.
fn card_findings(
    bound: &crate::session::RuntimeInteractionBrowserRegion,
    tab: Option<&serde_json::Value>,
    taken: Option<&super::browser_input_log::InputOutcome>,
    refused: Option<&super::browser_input_log::InputOutcome>,
) -> Vec<serde_json::Value> {
    let finding = |code: &str, text: String| serde_json::json!({"code": code, "text": text});
    let mut findings = Vec::new();
    match tab {
        None => findings.push(finding(
            "browser_unreadable",
            "the kernel could not read this Tab's state; the browser may be starting or gone".into(),
        )),
        Some(state) if state["tab_known"] == false => findings.push(finding(
            "tab_gone",
            format!(
                "the browser no longer has Tab {}: the page this card shows is gone, so no input can reach it; raise the card again on a current Tab",
                bound.tab_id
            ),
        )),
        Some(state) => {
            if state["generation"].as_u64().is_some_and(|now| now != bound.generation) {
                findings.push(finding(
                    "generation_changed",
                    format!(
                        "the card is bound to browser generation {} and the browser is at {}: it restarted since the card was raised, so input bound to the old generation is refused; raise the card again",
                        bound.generation, state["generation"]
                    ),
                ));
            }
            if state["document_id"].as_str().is_some_and(|now| now != bound.document_id) {
                findings.push(finding(
                    "document_changed",
                    format!(
                        "the card is bound to document {} and the Tab now shows {}: the page navigated or reloaded after the card was raised, so input bound to the old document is refused; raise the card again",
                        bound.document_id, state["document_id"]
                    ),
                ));
            }
            for holder in state["input_held_by"].as_array().into_iter().flatten() {
                findings.push(finding(
                    "input_held",
                    format!(
                        "this Tab's input is held by {} ({}){}",
                        holder["label"].as_str().unwrap_or("an actor"),
                        holder["kind"].as_str().unwrap_or("unknown kind"),
                        if holder["holds"]
                            .as_array()
                            .is_some_and(|holds| holds.iter().any(|held| held == "desktop"))
                        {
                            ", through a takeover of the desktop"
                        } else {
                            ""
                        }
                    ),
                ));
            }
        }
    }
    if let Some(refused) = refused {
        findings.push(finding(
            "last_input_refused",
            format!(
                "the last refused {} input said: {}",
                refused.kind,
                refused.reason.as_deref().unwrap_or("no reason recorded")
            ),
        ));
    }
    if taken.is_none() && refused.is_none() {
        findings.push(finding(
            "no_input_seen",
            "no input has reached the kernel for this Tab since it started: if the person clicked, the client did not send it".into(),
        ));
    }
    findings
}

#[cfg(test)]
mod card_findings_tests {
    use super::*;
    use serde_json::json;

    fn bound() -> crate::session::RuntimeInteractionBrowserRegion {
        serde_json::from_value(json!({
            "tab_id":"t","ref":"r","generation":9,"document_id":"DOC-A","origin":"https://x",
            "rect":{"x":0,"y":0,"width":1,"height":1},"viewport_width":1,"viewport_height":1
        }))
        .unwrap()
    }
    fn codes(findings: &[serde_json::Value]) -> Vec<&str> {
        findings.iter().filter_map(|f| f["code"].as_str()).collect()
    }

    #[test]
    fn a_card_bound_to_an_old_generation_or_document_says_which() {
        let now =
            json!({"tab_known":true,"generation":10,"document_id":"DOC-B","input_held_by":[]});
        let found = card_findings(&bound(), Some(&now), None, None);
        assert_eq!(
            codes(&found),
            ["generation_changed", "document_changed", "no_input_seen"]
        );
        assert!(found[0]["text"].as_str().unwrap().contains("restarted"));
        assert!(found[1]["text"].as_str().unwrap().contains("navigated"));
    }

    #[test]
    fn a_current_card_whose_input_is_held_names_the_holder() {
        let now = json!({"tab_known":true,"generation":9,"document_id":"DOC-A",
            "input_held_by":[{"actor_id":"agent:a","kind":"agent","label":"Agent"}]});
        let found = card_findings(&bound(), Some(&now), None, None);
        assert_eq!(codes(&found), ["input_held", "no_input_seen"]);
        assert!(found[0]["text"].as_str().unwrap().contains("Agent (agent)"));
    }

    #[test]
    fn an_unreadable_browser_is_said_not_guessed() {
        let found = card_findings(&bound(), None, None, None);
        assert_eq!(codes(&found), ["browser_unreadable", "no_input_seen"]);
    }
}
