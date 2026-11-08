//! One decision is one card: the interaction a sub-agent raises stays one object, and the
//! kernel only moves its holder (the parent agent that may answer it, or the user). The
//! holder is kernel state for routing and audit; clients never read it.
use super::*;
use crate::agent::authority_limit::{ancestors, descendants};
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation};
use crate::provider::AgentPermissionLevel;

pub(crate) const CHILD_DECISION_EVENT: &str = "child_decision";
/// The answerer was not the holder when the answer reached the interaction lock.
pub(crate) const HOLDER_CHANGED: &str = "decision holder changed";
/// A one-line question is at most this many characters in the holder's inbox event.
const QUESTION_LIMIT: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecisionHolder {
    User,
    Agent(String),
}

/// Decisions that never leave the user. Default deny: a decision no case names is the user's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserOnly {
    /// Secret values, Vault entries and credential prompts.
    Secret,
    Sudo,
    Passkey,
    Payment,
    /// Provider login, pairing, relay and Cloud identity, human verification.
    SignIn,
    /// Kernel access grants and their extensions.
    AccessGrant,
    /// Browser decisions of the user's own domain.
    UserDomain,
    /// Anything the kernel marked critical.
    Critical,
    /// Any other kernel operation: acquiring or starting a machine, installs, unclassified.
    KernelOperation,
}

/// Why `interaction` can only be answered by the user, if it can.
pub(crate) fn user_only(
    pending: &PendingInteraction,
    interaction: &crate::session::RuntimeInteraction,
) -> Option<UserOnly> {
    let operation = interaction.kernel_operation_id().unwrap_or_default();
    let secret = pending.terminal_credential_owner.is_some()
        || interaction.custom_choice().is_some_and(|custom| {
            custom.input_kind() == crate::session::RuntimeInteractionInputKind::Secret
        })
        || interaction
            .choices()
            .iter()
            .any(|choice| choice.vault_unlock().is_some());
    if secret {
        Some(UserOnly::Secret)
    } else if operation.starts_with("sudo:") || interaction.sudo_request().is_some() {
        Some(UserOnly::Sudo)
    } else if operation.starts_with("access-") {
        Some(UserOnly::AccessGrant)
    } else if pending.passkey_prompt.is_some()
        || interaction
            .choices()
            .iter()
            .any(crate::session::RuntimeInteractionChoice::requires_passkey)
    {
        Some(UserOnly::Passkey)
    } else if interaction.id().starts_with("vault-card-") {
        Some(UserOnly::Payment)
    } else if interaction.provider_login_is_human_only()
        || interaction.handoff().is_some()
        || super::is_handoff_interaction_id(interaction.id())
    {
        Some(UserOnly::SignIn)
    } else if pending.user_domain_interaction.is_some() {
        Some(UserOnly::UserDomain)
    } else if interaction.level() == crate::session::RuntimeInteractionLevel::Critical {
        Some(UserOnly::Critical)
    } else if interaction.agent_id().is_none()
        || pending.kernel_operation_owner.is_some()
        || pending.kernel_wide_interaction.is_some()
    {
        Some(UserOnly::KernelOperation)
    } else {
        None
    }
}

/// The agent a decision is about: the asker, or the agent a kernel-owned decision concerns.
fn subject_agent(interaction: &crate::session::RuntimeInteraction) -> Option<&str> {
    interaction
        .agent_id()
        .or_else(|| interaction.related_agent_id())
}

impl KernelRuntimeOwnedState {
    /// Whether `ancestor` may hold a decision now: it is alive, not stopped, not past its deadline,
    /// not itself waiting for the user, and would approve for itself (a `Required` agent would ask
    /// the user, and a parent passes on only the authority it holds).
    fn ancestor_can_hold(
        &self,
        session: &crate::session::RuntimeSession,
        ancestor: &crate::agent::AgentInstance,
        agents: &[crate::agent::AgentInstance],
    ) -> bool {
        if ancestor.archived()
            || crate::session::effective_agent_execution_config_in_tree(session, ancestor, agents)
                .permission_level
                == AgentPermissionLevel::Required
            || session
                .active_interaction_for_agent(ancestor.id())
                .is_some()
        {
            return false;
        }
        let past_deadline = self
            .durable_state_store
            .agent_deadline(session.id(), ancestor.id())
            .ok()
            .flatten()
            .is_some_and(|deadline| {
                deadline.state == crate::local::AgentDeadlineState::Reached
                    || deadline.deadline_ms <= crate::session::unix_epoch_ms()
            });
        let ended = self
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(ancestor.id()))
            .ok()
            .and_then(|tasks| tasks.last().map(|task| task.state.clone()))
            .is_some_and(|state| {
                matches!(state, ExecutionState::Cancelled | ExecutionState::Blocked)
            });
        !past_deadline && !ended
    }

    /// The nearest ancestor above `from` that may answer `interaction`, or the user.
    pub(super) fn decision_holder_above(
        &self,
        session: &crate::session::RuntimeSession,
        pending: &PendingInteraction,
        interaction: &crate::session::RuntimeInteraction,
        from: &str,
    ) -> DecisionHolder {
        if user_only(pending, interaction).is_some() {
            return DecisionHolder::User;
        }
        let agents = self.agent_store.get_session_agents(session.id());
        let Some(start) = agents.iter().find(|agent| agent.id() == from) else {
            return DecisionHolder::User;
        };
        ancestors(start, &agents)
            .into_iter()
            .find(|ancestor| self.ancestor_can_hold(session, ancestor, &agents))
            .map_or(DecisionHolder::User, |ancestor| {
                DecisionHolder::Agent(ancestor.id().to_owned())
            })
    }

    /// Whether `holder` may still answer: it is an ancestor of the asker that can hold now.
    pub(super) fn holder_may_answer(
        &self,
        session: &crate::session::RuntimeSession,
        pending: &PendingInteraction,
        interaction: &crate::session::RuntimeInteraction,
        holder: &str,
    ) -> bool {
        let agents = self.agent_store.get_session_agents(session.id());
        user_only(pending, interaction).is_none()
            && subject_agent(interaction)
                .and_then(|asker| agents.iter().find(|agent| agent.id() == asker))
                .is_some_and(|asker| {
                    ancestors(asker, &agents).into_iter().any(|ancestor| {
                        ancestor.id() == holder
                            && self.ancestor_can_hold(session, ancestor, &agents)
                    })
                })
    }

    /// The holder of a decision at the moment it is raised.
    pub(super) fn decision_holder_at_raise(
        &self,
        session: &crate::session::RuntimeSession,
        pending: &PendingInteraction,
        interaction: &crate::session::RuntimeInteraction,
    ) -> DecisionHolder {
        subject_agent(interaction).map_or(DecisionHolder::User, |agent| {
            self.decision_holder_above(session, pending, interaction, agent)
        })
    }

    /// Where the user sees a decision of a sub-agent: the top parent's panel. `None` for an
    /// agent that has no parent.
    pub(super) fn decision_shown_in(
        &self,
        session_id: &str,
        interaction: &crate::session::RuntimeInteraction,
    ) -> Option<String> {
        let agents = self.agent_store.get_session_agents(session_id);
        let subject = agents
            .iter()
            .find(|agent| Some(agent.id()) == subject_agent(interaction))?;
        ancestors(subject, &agents)
            .last()
            .map(|top| top.id().to_owned())
    }

    /// The decisions of `session_id` the agent `holder` holds now.
    fn decisions_held_by(&self, session_id: &str, holder: &str) -> Vec<String> {
        self.pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| {
                pending.session_id == session_id
                    && pending.holder == DecisionHolder::Agent(holder.to_owned())
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The decisions of `session_id` some agent, not the user, holds now.
    pub(super) fn decisions_held_by_agents(
        &self,
        session_id: &str,
    ) -> std::collections::HashSet<String> {
        self.pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| {
                pending.session_id == session_id && pending.holder != DecisionHolder::User
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Withdraws the `child_decision` event `holder` was sent for `interaction_id`: an event not
    /// yet delivered expires, a delivered one is closed as handled. Never fails the caller.
    pub(super) fn withdraw_child_decision_event(
        &self,
        session_id: &str,
        holder: &str,
        tenure: u32,
        interaction_id: &str,
    ) {
        let events = match self.child_decision_events(session_id, holder, tenure, interaction_id) {
            Ok(events) => events,
            Err(error) => return log_withdraw_failure(interaction_id, &error),
        };
        for event in events {
            let operation = match event.state.as_str() {
                "pending" => Operation::Expire {
                    room: session_id.into(),
                    agent: holder.into(),
                    sequence: event.sequence,
                },
                "expired" | "handled" | "submitting" | "uncertain" | "blocked" => continue,
                _ => Operation::Ack {
                    room: session_id.into(),
                    agent: holder.into(),
                    sequence: event.sequence,
                    handled: true,
                    now: crate::session::unix_epoch_ms(),
                },
            };
            if let Err(error) = self.durable_state_store.agent_lifecycle(operation) {
                log_withdraw_failure(interaction_id, &error);
            }
        }
    }

    /// The `child_decision` events `holder` was sent for `interaction_id` in its `tenure`.
    fn child_decision_events(
        &self,
        session_id: &str,
        holder: &str,
        tenure: u32,
        interaction_id: &str,
    ) -> Result<Vec<ledger::InboxEvent>, DaemonError> {
        let occurrence = child_decision_occurrence(interaction_id, holder, tenure);
        let mut events = Vec::new();
        self.durable_state_store
            .scan_agent_inbox(session_id, holder, 0, None, |event| {
                if event.occurrence_id == occurrence {
                    events.push(event);
                }
                std::ops::ControlFlow::Continue(())
            })?;
        Ok(events)
    }

    /// The decisions `holder` already had: its event for the current tenure is no longer
    /// waiting to be delivered (a turn that never read a steered event gives it back as pending).
    fn decisions_seen_by(&self, session_id: &str, holder: &str) -> Vec<String> {
        let held = self
            .pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| {
                pending.session_id == session_id
                    && pending.holder == DecisionHolder::Agent(holder.to_owned())
            })
            .map(|(id, pending)| (id.clone(), pending.holder_tenure))
            .collect::<Vec<_>>();
        held.into_iter()
            .filter(|(id, tenure)| {
                // An unreadable inbox must not keep the decision from the user.
                self.child_decision_events(session_id, holder, *tenure, id)
                    .map_or(true, |events| {
                        !events.iter().any(|event| event.state == "pending")
                    })
            })
            .map(|(id, _)| id)
            .collect()
    }

    /// Withdraws the holder's event once the decision is no longer pending.
    pub(super) fn withdraw_settled_child_decision(
        &self,
        session_id: &str,
        interaction_id: &str,
        pending: &PendingInteraction,
    ) {
        if let DecisionHolder::Agent(holder) = &pending.holder {
            self.withdraw_child_decision_event(
                session_id,
                holder,
                pending.holder_tenure,
                interaction_id,
            );
        }
    }

    /// Moves every decision `agent` holds to the next holder above it, at once and without a
    /// model turn. Returns the decisions whose new holder is an agent, to be announced.
    fn move_decisions_up_from(
        &self,
        session_id: &str,
        agent: &str,
        only: Option<&str>,
    ) -> Result<(usize, Vec<String>), DaemonError> {
        let _mutation =
            self.pending_interactions
                .mutation
                .lock()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime interaction",
                    message: "Interaction store is unavailable".into(),
                })?;
        let mut announce = Vec::new();
        let mut moved = 0;
        for id in self.decisions_held_by(session_id, agent) {
            if only.is_some_and(|only| only != id) {
                continue;
            }
            moved += 1;
            let mut sessions = self.session_store.write();
            let mut session = sessions.get_session(session_id)?.clone();
            let Some(pending) = self.pending_interactions.write().get(&id).cloned() else {
                continue;
            };
            let Some(interaction) = session
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id() == id)
                .cloned()
            else {
                continue;
            };
            let holder = self.decision_holder_above(&session, &pending, &interaction, agent);
            let old_tenure = pending.holder_tenure;
            if let Some(entry) = self.pending_interactions.write().get_mut(&id) {
                entry.hand_to(holder.clone());
            }
            match &holder {
                DecisionHolder::User => {
                    let shown = self.decision_shown_in(session_id, &interaction);
                    session.show_interaction_in(&id, shown.clone());
                    // The follower hears of it now: it was not listed while an agent held it.
                    let mut pending = pending;
                    pending.holder = DecisionHolder::User;
                    self.publish_outside_interaction(
                        session_id,
                        &interaction.shown_in(shown),
                        InteractionPhase::Pending(&pending),
                    )?;
                    sessions.restore_session(session);
                }
                DecisionHolder::Agent(_) => announce.push(id.clone()),
            }
            drop(sessions);
            self.withdraw_child_decision_event(session_id, agent, old_tenure, &id);
        }
        Ok((moved, announce))
    }

    /// `agent` was just placed elsewhere in the tree; the caller holds the interaction lock. Every
    /// open decision its subtree raised follows it: a card the user holds is shown in the new top
    /// panel, and one held by an agent that is no longer above the asker, or can no longer hold
    /// under the new parent chain, goes to the nearest ancestor that may hold it, or the user.
    /// Returns the moved count and the decisions to announce.
    pub(super) fn move_subtree_decisions(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Result<(usize, Vec<String>), DaemonError> {
        let agents = self.agent_store.get_session_agents(session_id);
        let subtree = std::iter::once(agent)
            .chain(descendants(agent, &agents).into_iter().map(|a| a.id()))
            .collect::<std::collections::HashSet<_>>();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        let open = session
            .active_interactions()
            .iter()
            .filter(|interaction| subject_agent(interaction).is_some_and(|s| subtree.contains(s)))
            .cloned()
            .collect::<Vec<_>>();
        let (mut publish, mut announce, mut withdraw) = (Vec::new(), Vec::new(), Vec::new());
        for interaction in open {
            let id = interaction.id().to_owned();
            let Some(mut pending) = self.pending_interactions.write().get(&id).cloned() else {
                continue;
            };
            // Above the asker is not enough: a new parent chain can lower the holder's authority.
            let may_still_hold = |holder: &str| {
                subject_agent(&interaction)
                    .and_then(|asker| agents.iter().find(|a| a.id() == asker))
                    .is_some_and(|asker| {
                        ancestors(asker, &agents).iter().any(|a| {
                            a.id() == holder && self.ancestor_can_hold(&session, a, &agents)
                        })
                    })
            };
            match pending.holder.clone() {
                DecisionHolder::Agent(holder) if may_still_hold(&holder) => {}
                DecisionHolder::Agent(holder) => {
                    withdraw.push((holder, pending.holder_tenure, id.clone()));
                    pending.hand_to(self.decision_holder_at_raise(
                        &session,
                        &pending,
                        &interaction,
                    ));
                    if let Some(entry) = self.pending_interactions.write().get_mut(&id) {
                        entry.hand_to(pending.holder.clone());
                    }
                    match pending.holder {
                        DecisionHolder::Agent(_) => announce.push(id),
                        DecisionHolder::User => publish.push((interaction, pending)),
                    }
                }
                DecisionHolder::User => publish.push((interaction, pending)),
            }
        }
        let publish = publish
            .into_iter()
            .filter_map(|(interaction, pending)| {
                let shown = self.decision_shown_in(session_id, &interaction);
                let changed = shown.as_deref() != interaction.shown_in_agent_id()
                    || withdraw.iter().any(|(_, _, id)| id == interaction.id());
                changed.then(|| {
                    session.show_interaction_in(interaction.id(), shown.clone());
                    (interaction.shown_in(shown), pending)
                })
            })
            .collect::<Vec<_>>();
        sessions.restore_session(session);
        drop(sessions);
        for (holder, tenure, id) in &withdraw {
            self.withdraw_child_decision_event(session_id, holder, *tenure, id);
        }
        // The follower hears of each card where it is now listed.
        for (interaction, pending) in &publish {
            self.publish_outside_interaction(
                session_id,
                interaction,
                InteractionPhase::Pending(pending),
            )?;
        }
        Ok((publish.len() + announce.len(), announce))
    }
}

fn log_withdraw_failure(interaction_id: &str, error: &DaemonError) {
    crate::logging::warn_with_fields(
        "runtime.interaction",
        "child decision event could not be withdrawn",
        serde_json::json!({"interaction_id": interaction_id, "error": error.to_string()}),
    );
}

/// One event per holder tenure: the same holder announced again is one event, a holder that
/// gets the decision back after another one held it is a new one.
fn child_decision_occurrence(interaction_id: &str, holder: &str, tenure: u32) -> String {
    format!("child-decision-{interaction_id}-{holder}-{tenure}")
}

impl KernelRuntimeState {
    /// Sends the holder agent its urgent `child_decision` event. A decision the user holds
    /// tells no agent.
    pub(super) async fn announce_child_decision(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(pending) = self
            .owned
            .pending_interactions
            .write()
            .get(interaction_id)
            .cloned()
        else {
            return Ok(());
        };
        let DecisionHolder::Agent(holder) = &pending.holder else {
            return Ok(());
        };
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(interaction) = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
        else {
            return Ok(());
        };
        let child = subject_agent(interaction).unwrap_or_default();
        let alias = self
            .owned
            .agent_store
            .get_agent(child)
            .ok()
            .and_then(|agent| agent.alias().map(str::to_owned));
        let question = interaction
            .title()
            .unwrap_or_else(|| interaction.message())
            .chars()
            .take(QUESTION_LIMIT)
            .collect::<String>();
        let mut event = ledger::occurrence(
            session_id,
            holder,
            child,
            &child_decision_occurrence(interaction_id, holder, pending.holder_tenure),
            CHILD_DECISION_EVENT,
            serde_json::json!({
                "interaction_id": interaction_id,
                "child_agent_id": child,
                "child_alias": alias,
                "question": question,
                "choices": interaction.choices().iter().map(|choice| serde_json::json!({
                    "id": choice.id(),
                    "label": choice.label(),
                })).collect::<Vec<_>>(),
                "text_allowed": interaction.custom_choice().is_some(),
                "needs_answer": true,
            }),
        );
        event.urgent = true;
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(event))?;
        let holder = holder.clone();
        Box::pin(self.deliver_agent_inbox(session_id, &holder)).await
    }

    /// `agent` ended, errored or was cancelled: the decisions it held move up at once.
    pub(super) async fn move_decisions_up(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        self.move_decisions_up_matching(session_id, agent, None)
            .await
            .map(|_| ())
    }

    /// `agent` yielded: it runs again only when one of its sources fires, so a decision it already
    /// had would wait on it for nobody. Those move up; one it was not given yet still wakes it.
    pub(super) async fn move_seen_decisions_up(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        for id in self.owned.decisions_seen_by(session_id, agent) {
            self.move_decisions_up_matching(session_id, agent, Some(&id))
                .await?;
        }
        Ok(())
    }

    async fn move_decisions_up_matching(
        &self,
        session_id: &str,
        agent: &str,
        only: Option<&str>,
    ) -> Result<usize, DaemonError> {
        let (moved, announce) = self.owned.move_decisions_up_from(session_id, agent, only)?;
        if moved > 0 {
            self.owned.session_snapshot(session_id)?;
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(session_id);
        }
        for id in announce {
            self.announce_child_decision(session_id, &id).await?;
        }
        Ok(moved)
    }

    /// `agent pass`: the holder hands the decision to the next holder above it.
    pub(crate) async fn pass_child_decision(
        &self,
        session_id: &str,
        holder: &str,
        interaction_id: &str,
    ) -> Result<ChildDecisionStatus, DaemonError> {
        let status = self.child_decision_gate(session_id, holder, interaction_id);
        if status != ChildDecisionStatus::Answered {
            return Ok(status);
        }
        Ok(
            match self
                .move_decisions_up_matching(session_id, holder, Some(interaction_id))
                .await?
            {
                0 => ChildDecisionStatus::NotHolder,
                _ => ChildDecisionStatus::Passed,
            },
        )
    }

    /// `agent answer`: only the current holder answers; the answer is a choice id or free text.
    /// A holder refused because it may no longer answer (a permission change above it, its own
    /// question to the user, its deadline) does not keep the decision: it moves on at once.
    pub(crate) async fn answer_child_decision(
        &self,
        session_id: &str,
        holder: &str,
        interaction_id: &str,
        answer: &str,
    ) -> Result<ChildDecisionStatus, DaemonError> {
        let status = self.answer_held_child_decision(session_id, holder, interaction_id, answer)?;
        if status == ChildDecisionStatus::NotHolder {
            self.move_decisions_up_matching(session_id, holder, Some(interaction_id))
                .await?;
        }
        Ok(status)
    }

    fn answer_held_child_decision(
        &self,
        session_id: &str,
        holder: &str,
        interaction_id: &str,
        answer: &str,
    ) -> Result<ChildDecisionStatus, DaemonError> {
        let status = self.child_decision_gate(session_id, holder, interaction_id);
        if status != ChildDecisionStatus::Answered {
            return Ok(status);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(interaction) = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
        else {
            return Ok(ChildDecisionStatus::SettledByUser);
        };
        let (choice_id, reply) = if interaction.choice(answer).is_some() {
            (answer.to_owned(), None)
        } else if let Some(custom) = interaction.custom_choice() {
            (custom.id().to_owned(), Some(answer))
        } else {
            return Ok(ChildDecisionStatus::InvalidAnswer);
        };
        match self.owned.resolve_runtime_interaction_routed(
            session_id,
            interaction_id,
            &choice_id,
            reply,
            None,
            false,
            None,
            false,
            false,
            None,
            Some(holder),
        ) {
            Ok(()) => Ok(ChildDecisionStatus::Answered),
            Err(error) => match error.to_string() {
                text if text.contains(HOLDER_CHANGED) => Ok(ChildDecisionStatus::NotHolder),
                text if text.contains("not pending") || text.contains("no longer pending") => {
                    Ok(ChildDecisionStatus::SettledByUser)
                }
                _ => Err(error),
            },
        }
    }

    /// Whether `holder` holds the decision now: `Answered` stands for "may go on".
    fn child_decision_gate(
        &self,
        session_id: &str,
        holder: &str,
        interaction_id: &str,
    ) -> ChildDecisionStatus {
        match self
            .owned
            .pending_interactions
            .write()
            .get(interaction_id)
            .filter(|pending| pending.session_id == session_id)
        {
            None => ChildDecisionStatus::SettledByUser,
            Some(pending) if pending.holder != DecisionHolder::Agent(holder.to_owned()) => {
                ChildDecisionStatus::NotHolder
            }
            Some(_) => ChildDecisionStatus::Answered,
        }
    }
}

/// The typed result of `agent answer` and `agent pass`; the kernel adds no error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChildDecisionStatus {
    Answered,
    Passed,
    /// The caller does not hold this decision (any more).
    NotHolder,
    /// The decision was answered, withdrawn or timed out before this answer arrived.
    SettledByUser,
    /// The answer is neither a choice of the decision nor free text it accepts.
    InvalidAnswer,
}
