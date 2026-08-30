use std::collections::{BTreeMap, BTreeSet};

use super::action::{
    ActionAdmission, EnvironmentAction, EnvironmentActionRequest, EnvironmentActionState,
    EnvironmentActionTerminal, InputTarget,
};
use super::model::{
    EnvironmentActor, EnvironmentActorKind, EnvironmentError, EnvironmentLifecycle,
};
use super::ownership::{InputOwnership, PendingInputTakeover, TakeoverOutcome};
use super::tabs::TabRegistry;

pub(crate) struct ActionFinishEffect {
    pub(crate) state: EnvironmentActionState,
    pub(crate) ownership_changed: bool,
    pub(crate) started_action_ids: Vec<String>,
}

pub(crate) struct ActionTakeoverEffect {
    pub(crate) outcome: TakeoverOutcome,
    pub(crate) ownership_changed: bool,
    pub(crate) cancelled_action_ids: Vec<String>,
    pub(crate) started_action_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentActionLedger {
    actions: BTreeMap<String, EnvironmentAction>,
    requests: BTreeMap<String, EnvironmentActionRequest>,
    idempotency_actions: BTreeMap<String, String>,
    order: Vec<String>,
    reservations: BTreeMap<InputTarget, String>,
    input_owners: BTreeMap<InputTarget, String>,
    pending_takeovers: BTreeMap<InputTarget, String>,
    next_sequence: u64,
    terminal_capacity: usize,
    queue_capacity: usize,
}

impl EnvironmentActionLedger {
    pub(crate) fn new(terminal_capacity: usize, queue_capacity: usize) -> Self {
        Self {
            actions: BTreeMap::new(),
            requests: BTreeMap::new(),
            idempotency_actions: BTreeMap::new(),
            order: Vec::new(),
            reservations: BTreeMap::new(),
            input_owners: BTreeMap::new(),
            pending_takeovers: BTreeMap::new(),
            next_sequence: 1,
            terminal_capacity,
            queue_capacity,
        }
    }

    pub(crate) fn actions(&self) -> Vec<EnvironmentAction> {
        self.order
            .iter()
            .filter_map(|action_id| self.actions.get(action_id).cloned())
            .collect()
    }

    pub(crate) fn ownership(&self) -> Vec<InputOwnership> {
        self.input_owners
            .iter()
            .map(|(target, actor_id)| InputOwnership {
                target: target.clone(),
                actor_id: actor_id.clone(),
            })
            .collect()
    }

    pub(crate) fn pending_takeovers(&self) -> Vec<PendingInputTakeover> {
        self.pending_takeovers
            .iter()
            .map(|(target, human_actor_id)| PendingInputTakeover {
                target: target.clone(),
                human_actor_id: human_actor_id.clone(),
                blocking_action_ids: self.reservations.get(target).into_iter().cloned().collect(),
            })
            .collect()
    }

    pub(crate) fn owner(&self, target: &InputTarget) -> Option<&str> {
        self.input_owners.get(target).map(String::as_str)
    }

    pub(crate) fn submit(
        &mut self,
        request: EnvironmentActionRequest,
        lifecycle: EnvironmentLifecycle,
        runtime_generation: u64,
        actors: &BTreeMap<String, EnvironmentActor>,
        tabs: &TabRegistry,
    ) -> Result<ActionAdmission, EnvironmentError> {
        if lifecycle != EnvironmentLifecycle::Ready {
            return Err(EnvironmentError::EnvironmentNotReady { lifecycle });
        }
        if !actors.contains_key(&request.actor_id) {
            return Err(EnvironmentError::UnknownActor {
                actor_id: request.actor_id,
            });
        }
        if let Some(idempotency_key) = request.idempotency_key.as_ref() {
            if let Some(action_id) = self.idempotency_actions.get(idempotency_key) {
                let original = self
                    .requests
                    .get(action_id)
                    .expect("idempotency index must reference an action request");
                if !original.matches_idempotent_operation(&request) {
                    return Err(EnvironmentError::IdempotencyConflict {
                        idempotency_key: idempotency_key.clone(),
                    });
                }
                let action = self
                    .actions
                    .get(action_id)
                    .expect("idempotency index must reference an action");
                return Ok(ActionAdmission::Existing {
                    action_id: action_id.clone(),
                    state: action.state,
                });
            }
        }
        if request.runtime_generation != runtime_generation {
            return Err(EnvironmentError::StaleRuntimeGeneration {
                expected: runtime_generation,
                actual: request.runtime_generation,
            });
        }
        for (tab_id, document_revision) in &request.tab_preconditions {
            tabs.validate_reference(
                request.runtime_generation,
                runtime_generation,
                tab_id,
                *document_revision,
            )?;
        }

        let accepted_request = request.clone();
        let targets: Vec<_> = request
            .targets
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for target in &targets {
            validate_target(tabs, target)?;
        }
        let queued = if request.mutates {
            for target in &targets {
                if let Some(human_actor_id) = self
                    .pending_takeovers
                    .get(target)
                    .or_else(|| self.input_owners.get(target))
                {
                    if human_actor_id != &request.actor_id {
                        return Ok(ActionAdmission::RejectedTakeover {
                            target: target.clone(),
                            human_actor_id: human_actor_id.clone(),
                        });
                    }
                }
            }
            targets.iter().any(|target| {
                self.reservations.contains_key(target)
                    || self.actions.values().any(|action| {
                        action.state == EnvironmentActionState::Queued
                            && action.targets.contains(target)
                    })
            })
        } else {
            false
        };
        if queued
            && self
                .actions
                .values()
                .filter(|action| action.state == EnvironmentActionState::Queued)
                .count()
                >= self.queue_capacity
        {
            return Ok(ActionAdmission::RejectedSaturated {
                capacity: self.queue_capacity,
            });
        }

        let sequence = self.next_sequence;
        let action_id = format!("action-{sequence}");
        self.next_sequence += 1;
        let action = EnvironmentAction {
            action_id: action_id.clone(),
            sequence,
            idempotency_key: request.idempotency_key.clone(),
            actor_id: request.actor_id,
            runtime_generation: request.runtime_generation,
            mode: request.mode,
            kind: request.kind,
            targets: targets.clone(),
            state: if queued {
                EnvironmentActionState::Queued
            } else {
                EnvironmentActionState::Running
            },
        };
        if request.mutates && !queued {
            for target in targets {
                self.reservations.insert(target, action_id.clone());
            }
        }
        self.actions.insert(action_id.clone(), action);
        if let Some(idempotency_key) = request.idempotency_key {
            self.requests.insert(action_id.clone(), accepted_request);
            self.idempotency_actions
                .insert(idempotency_key, action_id.clone());
        }
        self.order.push(action_id.clone());
        if queued {
            Ok(ActionAdmission::Queued {
                action_id,
                queue_sequence: sequence,
            })
        } else {
            Ok(ActionAdmission::Accepted { action_id })
        }
    }

    pub(crate) fn finish(
        &mut self,
        action_id: &str,
        terminal: EnvironmentActionTerminal,
    ) -> Result<ActionFinishEffect, EnvironmentError> {
        let state = terminal.into();
        let action =
            self.actions
                .get_mut(action_id)
                .ok_or_else(|| EnvironmentError::UnknownAction {
                    action_id: action_id.to_string(),
                })?;
        if action.state == EnvironmentActionState::Queued {
            return Err(EnvironmentError::ActionNotRunning {
                action_id: action_id.to_string(),
                state: action.state,
            });
        }
        if action.state != EnvironmentActionState::Running {
            return Err(EnvironmentError::ActionAlreadyTerminal {
                action_id: action_id.to_string(),
                state: action.state,
            });
        }
        action.state = state;
        self.reservations
            .retain(|_, reserved_action_id| reserved_action_id != action_id);
        let ownership_changed = self.finalize_takeovers();
        let started_action_ids = self.promote_queued_actions();
        self.compact_terminal_actions();
        Ok(ActionFinishEffect {
            state,
            ownership_changed,
            started_action_ids,
        })
    }

    pub(crate) fn request_takeover(
        &mut self,
        actor_id: &str,
        target: InputTarget,
        actors: &BTreeMap<String, EnvironmentActor>,
        tabs: &TabRegistry,
    ) -> Result<ActionTakeoverEffect, EnvironmentError> {
        let actor = actors
            .get(actor_id)
            .ok_or_else(|| EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            })?;
        if actor.kind != EnvironmentActorKind::Human {
            return Err(EnvironmentError::HumanActorRequired {
                actor_id: actor_id.to_string(),
            });
        }
        validate_target(tabs, &target)?;
        if let Some(owner_actor_id) = self.input_owners.get(&target) {
            if owner_actor_id == actor_id {
                return Ok(ActionTakeoverEffect {
                    outcome: TakeoverOutcome::Granted,
                    ownership_changed: false,
                    cancelled_action_ids: Vec::new(),
                    started_action_ids: Vec::new(),
                });
            }
            return Err(EnvironmentError::InputOwnedByAnotherActor {
                target,
                actor_id: owner_actor_id.clone(),
            });
        }
        if let Some(pending_actor_id) = self.pending_takeovers.get(&target) {
            if pending_actor_id != actor_id {
                return Err(EnvironmentError::InputOwnedByAnotherActor {
                    target,
                    actor_id: pending_actor_id.clone(),
                });
            }
            return Ok(ActionTakeoverEffect {
                outcome: TakeoverOutcome::CancellationRequired {
                    action_ids: self.blocking_action_ids(&target, actors),
                },
                ownership_changed: false,
                cancelled_action_ids: Vec::new(),
                started_action_ids: Vec::new(),
            });
        }
        let cancelled_action_ids = self.cancel_queued_agent_actions(&target, actors);
        let action_ids = self.blocking_action_ids(&target, actors);
        let (outcome, ownership_changed) = if action_ids.is_empty() {
            self.input_owners.insert(target, actor_id.to_string());
            (TakeoverOutcome::Granted, true)
        } else {
            self.pending_takeovers.insert(target, actor_id.to_string());
            (TakeoverOutcome::CancellationRequired { action_ids }, false)
        };
        let started_action_ids = self.promote_queued_actions();
        Ok(ActionTakeoverEffect {
            outcome,
            ownership_changed,
            cancelled_action_ids,
            started_action_ids,
        })
    }

    pub(crate) fn release(
        &mut self,
        actor_id: &str,
        target: &InputTarget,
    ) -> Result<(), EnvironmentError> {
        match self.input_owners.get(target) {
            Some(owner_actor_id) if owner_actor_id == actor_id => {
                self.input_owners.remove(target);
                Ok(())
            }
            Some(owner_actor_id) => Err(EnvironmentError::InputOwnedByAnotherActor {
                target: target.clone(),
                actor_id: owner_actor_id.clone(),
            }),
            None => Err(EnvironmentError::InputNotOwned {
                target: target.clone(),
            }),
        }
    }

    pub(crate) fn invalidate_runtime(&mut self) -> Vec<String> {
        self.reservations.clear();
        self.input_owners.clear();
        self.pending_takeovers.clear();
        let active_action_ids: Vec<_> = self
            .actions
            .values()
            .filter(|action| {
                matches!(
                    action.state,
                    EnvironmentActionState::Queued | EnvironmentActionState::Running
                )
            })
            .map(|action| action.action_id.clone())
            .collect();
        for action_id in &active_action_ids {
            if let Some(action) = self.actions.get_mut(action_id) {
                action.state = EnvironmentActionState::Failed;
            }
        }
        self.compact_terminal_actions();
        active_action_ids
    }

    pub(crate) fn clear_ownership(&mut self) -> bool {
        let changed = !self.input_owners.is_empty() || !self.pending_takeovers.is_empty();
        self.input_owners.clear();
        self.pending_takeovers.clear();
        changed
    }

    fn blocking_action_ids(
        &self,
        target: &InputTarget,
        actors: &BTreeMap<String, EnvironmentActor>,
    ) -> Vec<String> {
        self.reservations
            .get(target)
            .into_iter()
            .filter_map(|action_id| self.actions.get(action_id))
            .filter(|action| {
                actors
                    .get(&action.actor_id)
                    .is_some_and(|actor| actor.kind == EnvironmentActorKind::Agent)
            })
            .map(|action| action.action_id.clone())
            .collect()
    }

    fn cancel_queued_agent_actions(
        &mut self,
        target: &InputTarget,
        actors: &BTreeMap<String, EnvironmentActor>,
    ) -> Vec<String> {
        let action_ids: Vec<_> = self
            .order
            .iter()
            .filter_map(|action_id| self.actions.get(action_id))
            .filter(|action| {
                action.state == EnvironmentActionState::Queued
                    && action.targets.contains(target)
                    && actors
                        .get(&action.actor_id)
                        .is_some_and(|actor| actor.kind == EnvironmentActorKind::Agent)
            })
            .map(|action| action.action_id.clone())
            .collect();
        for action_id in &action_ids {
            self.actions
                .get_mut(action_id)
                .expect("queued action should remain in the ledger")
                .state = EnvironmentActionState::Cancelled;
        }
        self.compact_terminal_actions();
        action_ids
    }

    fn finalize_takeovers(&mut self) -> bool {
        let ready_targets: Vec<_> = self
            .pending_takeovers
            .keys()
            .filter(|target| !self.reservations.contains_key(*target))
            .cloned()
            .collect();
        let changed = !ready_targets.is_empty();
        for target in ready_targets {
            if let Some(actor_id) = self.pending_takeovers.remove(&target) {
                self.input_owners.insert(target, actor_id);
            }
        }
        changed
    }

    fn promote_queued_actions(&mut self) -> Vec<String> {
        let mut started_action_ids = Vec::new();
        for action_id in &self.order {
            let Some(action) = self.actions.get(action_id) else {
                continue;
            };
            if action.state != EnvironmentActionState::Queued
                || action.targets.iter().any(|target| {
                    self.reservations.contains_key(target)
                        || self.input_owners.contains_key(target)
                        || self.pending_takeovers.contains_key(target)
                })
            {
                continue;
            }
            let targets = action.targets.clone();
            for target in targets {
                self.reservations.insert(target, action_id.clone());
            }
            self.actions
                .get_mut(action_id)
                .expect("queued action should remain in the ledger")
                .state = EnvironmentActionState::Running;
            started_action_ids.push(action_id.clone());
        }
        started_action_ids
    }

    fn compact_terminal_actions(&mut self) {
        let terminal_ids: Vec<_> = self
            .order
            .iter()
            .filter(|action_id| {
                self.actions.get(*action_id).is_some_and(|action| {
                    matches!(
                        action.state,
                        EnvironmentActionState::Completed
                            | EnvironmentActionState::Failed
                            | EnvironmentActionState::Cancelled
                    )
                })
            })
            .cloned()
            .collect();
        let evict_count = terminal_ids.len().saturating_sub(self.terminal_capacity);
        for action_id in terminal_ids.into_iter().take(evict_count) {
            if let Some(action) = self.actions.remove(&action_id) {
                if let Some(idempotency_key) = action.idempotency_key {
                    if self.idempotency_actions.get(&idempotency_key) == Some(&action_id) {
                        self.idempotency_actions.remove(&idempotency_key);
                    }
                }
            }
            self.requests.remove(&action_id);
            self.order.retain(|candidate| candidate != &action_id);
        }
    }
}

fn validate_target(tabs: &TabRegistry, target: &InputTarget) -> Result<(), EnvironmentError> {
    if let InputTarget::BrowserTab(tab_id) = target {
        if !tabs.contains(tab_id) {
            return Err(EnvironmentError::UnknownTab {
                tab_id: tab_id.clone(),
            });
        }
    }
    Ok(())
}
