//! One decision is one card: the interaction a sub-agent raises stays one object, and the
//! kernel only moves its holder (the parent agent that may answer it, or the user). The
//! holder is kernel state for routing and audit; clients never read it. The user sees the card
//! in the top parent's panel from the start and may answer it whoever holds it.
use super::runtime_interaction_state::AgentDecisionAnswer;
use super::*;
use crate::agent::authority_limit::{ancestors, descendants};
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation};
use crate::provider::AgentPermissionLevel;

pub(crate) const CHILD_DECISION_EVENT: &str = "child_decision";
/// The answerer was not the holder when the answer reached the interaction lock.
pub(crate) const HOLDER_CHANGED: &str = "decision holder changed";
/// A one-line question is at most this many characters in the holder's inbox event.
const QUESTION_LIMIT: usize = 160;
/// The card's own text (the command it allows) is at most this many characters in the event.
const DETAIL_LIMIT: usize = 1000;

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
    } else if super::vault_use::is_vault_payment_decision(interaction.id()) {
        Some(UserOnly::Payment)
    } else if pending.passkey_prompt.is_some()
        || interaction
            .choices()
            .iter()
            .any(crate::session::RuntimeInteractionChoice::requires_passkey)
    {
        Some(UserOnly::Passkey)
    } else if interaction.provider_login_is_human_only()
        || interaction.handoff().is_some()
        || super::is_handoff_interaction_id(interaction.id())
    {
        Some(UserOnly::SignIn)
    } else if pending.user_domain_interaction.is_some() {
        Some(UserOnly::UserDomain)
    } else if interaction.level() == crate::session::RuntimeInteractionLevel::Critical {
        Some(UserOnly::Critical)
    } else if super::agent_task_owner_interactions::is_blocked_task_decision(interaction.id()) {
        // Resume or Cancel of a blocked task is a decision about an agent, so its parent may
        // hold it. Allowing the person's browser again stays the person's.
        if interaction.choice("allow_browser").is_some() {
            Some(UserOnly::UserDomain)
        } else {
            None
        }
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
    pub(super) fn ancestor_can_hold(
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
        self.durable_state_store.agent_inbox_occurrence(
            session_id,
            holder,
            &child_decision_occurrence(interaction_id, holder, tenure),
        )
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
            self.hand_decision_to(session_id, &id, &holder)?;
            let mut handed_to_user = false;
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
                    handed_to_user = true;
                }
                DecisionHolder::Agent(_) => announce.push(id.clone()),
            }
            drop(sessions);
            if handed_to_user {
                self.alert_owner_of_handed_up_task(&id);
            }
            self.withdraw_child_decision_event(session_id, agent, old_tenure, &id);
        }
        Ok((moved, announce))
    }

    /// A blocked task's card nobody above could hold reaches the person now, so they are alerted to it.
    fn alert_owner_of_handed_up_task(&self, interaction_id: &str) {
        let Some((task_id, _)) = interaction_id
            .strip_prefix(super::agent_task_owner_interactions::BLOCKED_TASK_DECISION_PREFIX)
            .and_then(|rest| rest.rsplit_once('-'))
        else {
            return;
        };
        let Ok(Some(task)) = self.durable_state_store.agent_task(task_id) else {
            return;
        };
        if let Err(error) = self.emit_task_owner_notification(&task, interaction_id) {
            tracing::warn!(%error, interaction_id, "handed-up task alert failed");
        }
    }

    /// The authority above or of `agent` changed (its permission or mode): every open decision of
    /// its subtree is checked again under the interaction lock, exactly as for a move in the tree.
    pub(super) fn recheck_subtree_decisions(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Result<(usize, Vec<String>), DaemonError> {
        let _mutation =
            self.pending_interactions
                .mutation
                .lock()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime interaction",
                    message: "Interaction store is unavailable".into(),
                })?;
        self.move_subtree_decisions(session_id, agent)
    }

    /// `agent` was just placed elsewhere in the tree; the caller holds the interaction lock. Every
    /// open decision its subtree raised follows it: every card is shown in the new top panel, and
    /// one held by an agent that is no longer above the asker, or can no longer hold
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
        let (mut placed, mut announce, mut withdraw) = (Vec::new(), Vec::new(), Vec::new());
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
            let rehanded = match pending.holder.clone() {
                DecisionHolder::Agent(holder) if may_still_hold(&holder) => false,
                DecisionHolder::Agent(holder) => {
                    withdraw.push((holder, pending.holder_tenure, id.clone()));
                    let holder = self.decision_holder_at_raise(&session, &pending, &interaction);
                    self.hand_decision_to(session_id, &id, &holder)?;
                    pending.hand_to(holder);
                    if let DecisionHolder::Agent(_) = pending.holder {
                        announce.push(id.clone());
                    }
                    true
                }
                DecisionHolder::User => false,
            };
            // Every card follows its subtree to the new top panel, whoever holds it.
            let shown = self.decision_shown_in(session_id, &interaction);
            if shown.as_deref() != interaction.shown_in_agent_id() || rehanded {
                session.show_interaction_in(&id, shown.clone());
                placed.push((interaction.shown_in(shown), pending));
            }
        }
        // A recheck that moved no card leaves the session as it is.
        if !placed.is_empty() {
            sessions.restore_session(session);
        }
        drop(sessions);
        for (holder, tenure, id) in &withdraw {
            self.withdraw_child_decision_event(session_id, holder, *tenure, id);
        }
        // The follower hears of each card the user holds where it is now listed.
        for (interaction, pending) in &placed {
            if pending.holder == DecisionHolder::User {
                self.publish_outside_interaction(
                    session_id,
                    interaction,
                    InteractionPhase::Pending(pending),
                )?;
            }
        }
        // A card handed to a new holder agent was placed again, so it is counted once.
        Ok((placed.len(), announce))
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

/// What the card says beyond its title, so a holder can tell which command it is asked to allow.
/// Secret-shaped values are redacted before the text is cut, so a cut never leaves half a token.
/// A card with no title already has its text as the question. The flag is true when the text was
/// cut: the part that was cut can be the part that matters, so a holder must be able to tell.
fn card_detail(interaction: &crate::session::RuntimeInteraction) -> Option<(String, bool)> {
    let title = interaction.title()?;
    let message = interaction.message();
    if message.trim().is_empty() || message == title {
        return None;
    }
    let redacted = crate::secret_redaction::redact_secrets(message);
    let mut detail: String = redacted.chars().take(DETAIL_LIMIT).collect();
    let truncated = redacted.chars().count() > DETAIL_LIMIT;
    if truncated {
        detail.push('…');
    }
    Some((detail, truncated))
}

impl KernelRuntimeOwnedState {
    /// Records the holder agent's urgent `child_decision` event for its current tenure and returns
    /// that holder. A decision the user holds tells no agent.
    pub(super) fn record_child_decision_event(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let Some(pending) = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .cloned()
        else {
            return Ok(None);
        };
        let DecisionHolder::Agent(holder) = pending.holder else {
            return Ok(None);
        };
        let session = self.session_store.get_session(session_id)?;
        let Some(interaction) = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
        else {
            return Ok(None);
        };
        let child = subject_agent(interaction).unwrap_or_default();
        let alias = self
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
        let mut payload = serde_json::json!({
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
        });
        if let Some((detail, truncated)) = card_detail(interaction) {
            payload["detail"] = detail.into();
            if truncated {
                payload["detail_truncated"] = true.into();
            }
        }
        let mut event = ledger::occurrence(
            session_id,
            &holder,
            child,
            &child_decision_occurrence(interaction_id, &holder, pending.holder_tenure),
            CHILD_DECISION_EVENT,
            payload,
        );
        event.urgent = true;
        self.durable_state_store
            .agent_lifecycle(Operation::Occur(event))?;
        Ok(Some(holder))
    }

    /// Whether `agent`'s last turn ended or parked on its sources: no turn of it is running.
    fn turn_is_settled(&self, session_id: &str, agent: &str) -> bool {
        self.durable_state_store
            .agent_tasks(Some(session_id), Some(agent))
            .ok()
            .and_then(|tasks| tasks.last().map(|task| task.state.clone()))
            .is_some_and(|state| matches!(state, ExecutionState::Waiting | ExecutionState::Done))
    }

    /// Hands the decision to `holder` for a new tenure. The holder is stored first: a pass to the
    /// user leaves no other trace, and a question registered again after a restart keeps it. The
    /// caller holds the interaction lock, so the tenure cannot change in between.
    fn hand_decision_to(
        &self,
        session_id: &str,
        interaction_id: &str,
        holder: &DecisionHolder,
    ) -> Result<(), DaemonError> {
        let Some(tenure) = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .map(|pending| pending.holder_tenure + 1)
        else {
            return Ok(());
        };
        self.durable_state_store.record_decision_holder(
            &crate::durable_state::StoredDecisionHolder {
                room_id: session_id.to_owned(),
                interaction_id: interaction_id.to_owned(),
                holder_agent_id: match holder {
                    DecisionHolder::User => None,
                    DecisionHolder::Agent(agent) => Some(agent.clone()),
                },
                tenure,
            },
        )?;
        if let Some(entry) = self.pending_interactions.write().get_mut(interaction_id) {
            entry.hand_to(holder.clone());
        }
        Ok(())
    }

    /// The holder and tenure a question that does not block its turn had before a restart.
    /// `None` when it was never handed on (or was stored by a kernel that predates the stored
    /// holder), or when the stored agent can no longer hold it: it is then handed on at raise.
    pub(super) fn restored_decision_holder(
        &self,
        session: &crate::session::RuntimeSession,
        pending: &PendingInteraction,
        interaction: &crate::session::RuntimeInteraction,
    ) -> Result<Option<(DecisionHolder, u32)>, DaemonError> {
        if pending.async_task_id.is_none() {
            return Ok(None);
        }
        let Some(stored) = self.durable_state_store.decision_holder(interaction.id())? else {
            return Ok(None);
        };
        if stored.room_id != pending.session_id || stored.tenure == 0 {
            return Ok(None);
        }
        let holder = match stored.holder_agent_id {
            None => DecisionHolder::User,
            Some(holder) => {
                let agents = self.agent_store.get_session_agents(&pending.session_id);
                let may_still_hold = subject_agent(interaction)
                    .and_then(|asker| agents.iter().find(|agent| agent.id() == asker))
                    .is_some_and(|asker| {
                        ancestors(asker, &agents).iter().any(|ancestor| {
                            ancestor.id() == holder
                                && self.ancestor_can_hold(session, ancestor, &agents)
                        })
                    });
                if !may_still_hold {
                    return Ok(None);
                }
                DecisionHolder::Agent(holder)
            }
        };
        Ok(Some((holder, stored.tenure)))
    }

    /// A question that does not block its turn is registered, when asked and again after a
    /// restart, with its holder at raise, which may be parked or ended. It goes to the nearest
    /// ancestor of the asker that may hold it and whose turn is not parked or ended, told with
    /// the event of its new tenure (the same event when a restart hands it to the same holder),
    /// else to the user's card in the top panel. A decision the user holds, or one already
    /// handed on, stays as it is.
    pub(super) fn hand_on_decision_at_raise(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        let _mutation =
            self.pending_interactions
                .mutation
                .lock()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime interaction",
                    message: "Interaction store is unavailable".into(),
                })?;
        let Some(pending) = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .cloned()
        else {
            return Ok(());
        };
        if pending.holder == DecisionHolder::User || pending.holder_tenure > 0 {
            return Ok(());
        }
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        let Some(interaction) = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
            .cloned()
        else {
            return Ok(());
        };
        let agents = self.agent_store.get_session_agents(session_id);
        let holder = subject_agent(&interaction)
            .and_then(|asker| agents.iter().find(|agent| agent.id() == asker))
            .and_then(|asker| {
                ancestors(asker, &agents).into_iter().find(|ancestor| {
                    self.ancestor_can_hold(&session, ancestor, &agents)
                        && !self.turn_is_settled(session_id, ancestor.id())
                })
            })
            .map_or(DecisionHolder::User, |ancestor| {
                DecisionHolder::Agent(ancestor.id().to_owned())
            });
        self.hand_decision_to(session_id, interaction_id, &holder)?;
        if holder != DecisionHolder::User {
            drop(sessions);
            return self
                .record_child_decision_event(session_id, interaction_id)
                .map(|_| ());
        }
        let shown = self.decision_shown_in(session_id, &interaction);
        session.show_interaction_in(interaction_id, shown.clone());
        let mut pending = pending;
        pending.holder = DecisionHolder::User;
        self.publish_outside_interaction(
            session_id,
            &interaction.shown_in(shown),
            InteractionPhase::Pending(&pending),
        )?;
        sessions.restore_session(session);
        drop(sessions);
        self.session_snapshot(session_id)?;
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        Ok(())
    }
}

impl KernelRuntimeState {
    /// Sends the holder agent its urgent `child_decision` event. A decision the user holds
    /// tells no agent.
    pub(super) async fn announce_child_decision(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(holder) = self
            .owned
            .record_child_decision_event(session_id, interaction_id)?
        else {
            return Ok(());
        };
        Box::pin(self.deliver_agent_inbox(session_id, &holder)).await
    }

    /// Shows the cards that moved to the user and tells each new holder agent.
    pub(super) async fn finish_moved_decisions(
        &self,
        session_id: &str,
        moved: usize,
        announce: Vec<String>,
    ) -> Result<(), DaemonError> {
        if moved > 0 {
            self.owned.session_snapshot(session_id)?;
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(session_id);
        }
        for id in announce {
            self.announce_child_decision(session_id, &id).await?;
        }
        Ok(())
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
        let child = subject_agent(interaction).unwrap_or_default().to_owned();
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
            Some(AgentInteractionAnswer::Holder(holder)),
        ) {
            Ok(()) => {
                self.audit_interaction_answered(
                    session_id,
                    interaction_id,
                    &choice_id,
                    None,
                    Some(AgentDecisionAnswer::Parent {
                        holder,
                        child: &child,
                    }),
                );
                Ok(ChildDecisionStatus::Answered)
            }
            Err(error) => match error.to_string() {
                text if text.contains(HOLDER_CHANGED) => Ok(ChildDecisionStatus::NotHolder),
                text if text.contains("not pending") || text.contains("no longer pending") => {
                    Ok(ChildDecisionStatus::SettledByUser)
                }
                _ => Err(error),
            },
        }
    }

    /// A decision no longer pending was settled if `holder` was ever sent it, else it is not
    /// one `holder` knows. An unreadable inbox reads as settled, the old answer.
    fn settled_or_unknown(
        &self,
        session_id: &str,
        holder: &str,
        interaction_id: &str,
    ) -> ChildDecisionStatus {
        let offered = self
            .owned
            .durable_state_store
            .agent_work_inbox(session_id, holder, 0, None)
            .map_or(true, |events| {
                events.iter().any(|event| {
                    event.kind == CHILD_DECISION_EVENT
                        && event.payload["interaction_id"] == interaction_id
                })
            });
        if offered {
            ChildDecisionStatus::SettledByUser
        } else {
            ChildDecisionStatus::UnknownInteraction
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
            None => self.settled_or_unknown(session_id, holder, interaction_id),
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
    /// The caller was never offered a decision with this id.
    UnknownInteraction,
    /// The answer is neither a choice of the decision nor free text it accepts.
    InvalidAnswer,
}
