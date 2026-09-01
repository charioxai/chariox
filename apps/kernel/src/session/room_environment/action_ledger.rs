use std::collections::{BTreeMap, BTreeSet};

use super::action::{
    ActionAdmission, ActionCancellationOutcome, EnvironmentAction,
    EnvironmentActionCancellationReason, EnvironmentActionFailureCode, EnvironmentActionOutcome,
    EnvironmentActionRequest, EnvironmentActionState, EnvironmentActionTerminal, EnvironmentMode,
    InputTarget,
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
    pub(crate) input_state_changed: bool,
    pub(crate) cancelled_action_ids: Vec<String>,
    pub(crate) started_action_ids: Vec<String>,
    pub(crate) cancellation_requested_action_ids: Vec<String>,
}

pub(crate) struct ActionCancellationEffect {
    pub(crate) outcome: ActionCancellationOutcome,
    pub(crate) action_changed: bool,
    pub(crate) started_action_ids: Vec<String>,
}

pub(crate) struct ActionRecoveryEffect {
    pub(crate) failed_action_ids: Vec<String>,
    pub(crate) ownership_changed: bool,
    pub(crate) started_action_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentActionLedger {
    actions: BTreeMap<String, EnvironmentAction>,
    history_records: BTreeMap<u64, EnvironmentAction>,
    history_action_sequences: BTreeMap<String, u64>,
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
            history_records: BTreeMap::new(),
            history_action_sequences: BTreeMap::new(),
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

    pub(crate) fn action(&self, action_id: &str) -> Option<&EnvironmentAction> {
        self.actions.get(action_id)
    }

    pub(crate) fn action_history(
        &self,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> super::action::EnvironmentActionHistoryPage {
        let mut actions = match before_sequence {
            Some(sequence) => self
                .history_records
                .range(..sequence)
                .rev()
                .take(limit.saturating_add(1))
                .map(|(_, action)| action.clone())
                .collect::<Vec<_>>(),
            None => self
                .history_records
                .iter()
                .rev()
                .take(limit.saturating_add(1))
                .map(|(_, action)| action.clone())
                .collect::<Vec<_>>(),
        };
        let has_more = actions.len() > limit;
        actions.truncate(limit);
        let next_before_sequence = if has_more {
            actions.last().map(|action| action.sequence)
        } else {
            None
        };
        super::action::EnvironmentActionHistoryPage {
            actions,
            next_before_sequence,
        }
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

    pub(crate) fn existing(
        &self,
        request: &EnvironmentActionRequest,
    ) -> Result<Option<ActionAdmission>, EnvironmentError> {
        let Some(idempotency_key) = request.idempotency_key.as_ref() else {
            return Ok(None);
        };
        let Some(action_id) = self.idempotency_actions.get(idempotency_key) else {
            return Ok(None);
        };
        let original = self
            .requests
            .get(action_id)
            .expect("idempotency index must reference an Action request");
        if !original.matches_idempotent_operation(request) {
            return Err(EnvironmentError::IdempotencyConflict {
                idempotency_key: idempotency_key.clone(),
            });
        }
        let action = self
            .actions
            .get(action_id)
            .or_else(|| {
                self.history_action_sequences
                    .get(action_id)
                    .and_then(|sequence| self.history_records.get(sequence))
            })
            .expect("idempotency index must reference Action history");
        Ok(Some(ActionAdmission::Existing {
            action_id: action_id.clone(),
            state: action.state,
        }))
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
        if let Some(existing) = self.existing(&request)? {
            return Ok(existing);
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
        let submitted_at_ms = crate::session::unix_epoch_ms();
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
            cancellation_requested: false,
            submitted_at_ms,
            started_at_ms: (!queued).then_some(submitted_at_ms),
            finished_at_ms: None,
            outcome: None,
        };
        if request.mutates && !queued {
            for target in targets {
                self.reservations.insert(target, action_id.clone());
            }
        }
        self.history_records.insert(sequence, action.clone());
        self.history_action_sequences
            .insert(action_id.clone(), sequence);
        self.actions.insert(action_id.clone(), action);
        self.requests.insert(action_id.clone(), accepted_request);
        if let Some(idempotency_key) = request.idempotency_key {
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
        let finished_at_ms = next_action_timestamp(action);
        action.finished_at_ms = Some(finished_at_ms);
        action.outcome = Some(match terminal {
            EnvironmentActionTerminal::Completed => EnvironmentActionOutcome::Completed,
            EnvironmentActionTerminal::Failed => EnvironmentActionOutcome::Failed {
                code: EnvironmentActionFailureCode::ControllerFailure,
            },
            EnvironmentActionTerminal::Cancelled if action.cancellation_requested => {
                EnvironmentActionOutcome::Cancelled {
                    reason: EnvironmentActionCancellationReason::Requested,
                }
            }
            EnvironmentActionTerminal::Cancelled => EnvironmentActionOutcome::Cancelled {
                reason: EnvironmentActionCancellationReason::ControllerCancellation,
            },
        });
        action.cancellation_requested = false;
        self.sync_history_action(action_id);
        self.reservations
            .retain(|_, reserved_action_id| reserved_action_id != action_id);
        let ownership_changed = self.finalize_takeovers();
        let started_action_ids = self.promote_queued_actions();
        Ok(ActionFinishEffect {
            state,
            ownership_changed,
            started_action_ids,
        })
    }

    pub(crate) fn cancel_as_actor(
        &mut self,
        actor_id: &str,
        action_id: &str,
        actors: &BTreeMap<String, EnvironmentActor>,
    ) -> Result<ActionCancellationEffect, EnvironmentError> {
        let actor = actors
            .get(actor_id)
            .ok_or_else(|| EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            })?;
        let action =
            self.actions
                .get(action_id)
                .ok_or_else(|| EnvironmentError::UnknownAction {
                    action_id: action_id.to_string(),
                })?;
        let authorized = action.actor_id == actor_id
            || (actor.kind == EnvironmentActorKind::Human
                && action.targets.iter().any(|target| {
                    self.input_owners.get(target).map(String::as_str) == Some(actor_id)
                        || self.pending_takeovers.get(target).map(String::as_str) == Some(actor_id)
                }));
        if !authorized {
            return Err(EnvironmentError::ActionCancellationForbidden {
                actor_id: actor_id.to_string(),
                action_id: action_id.to_string(),
            });
        }
        if matches!(
            action.state,
            EnvironmentActionState::Completed
                | EnvironmentActionState::Failed
                | EnvironmentActionState::Cancelled
        ) {
            return Ok(ActionCancellationEffect {
                outcome: ActionCancellationOutcome::AlreadyTerminal {
                    action_state: action.state,
                },
                action_changed: false,
                started_action_ids: Vec::new(),
            });
        }

        if action.state == EnvironmentActionState::Queued {
            let action = self
                .actions
                .get_mut(action_id)
                .expect("queued action should remain in the ledger");
            action.state = EnvironmentActionState::Cancelled;
            action.finished_at_ms = Some(next_action_timestamp(action));
            action.outcome = Some(EnvironmentActionOutcome::Cancelled {
                reason: EnvironmentActionCancellationReason::Requested,
            });
            self.sync_history_action(action_id);
            let started_action_ids = self.promote_queued_actions();
            return Ok(ActionCancellationEffect {
                outcome: ActionCancellationOutcome::Cancelled,
                action_changed: true,
                started_action_ids,
            });
        }

        let action = self
            .actions
            .get_mut(action_id)
            .expect("running action should remain in the ledger");
        let action_changed = !action.cancellation_requested;
        action.cancellation_requested = true;
        self.sync_history_action(action_id);
        Ok(ActionCancellationEffect {
            outcome: ActionCancellationOutcome::CancellationRequested,
            action_changed,
            started_action_ids: Vec::new(),
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
                    input_state_changed: false,
                    cancelled_action_ids: Vec::new(),
                    started_action_ids: Vec::new(),
                    cancellation_requested_action_ids: Vec::new(),
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
            let action_ids = self.blocking_action_ids(&target, actors);
            let cancellation_requested_action_ids = self.request_action_cancellation(&action_ids);
            return Ok(ActionTakeoverEffect {
                outcome: TakeoverOutcome::CancellationRequired { action_ids },
                input_state_changed: false,
                cancelled_action_ids: Vec::new(),
                started_action_ids: Vec::new(),
                cancellation_requested_action_ids,
            });
        }
        let cancelled_action_ids = self.cancel_queued_agent_actions(&target, actors);
        let action_ids = self.blocking_action_ids(&target, actors);
        let cancellation_requested_action_ids = self.request_action_cancellation(&action_ids);
        let outcome = if action_ids.is_empty() {
            self.input_owners.insert(target, actor_id.to_string());
            TakeoverOutcome::Granted
        } else {
            self.pending_takeovers.insert(target, actor_id.to_string());
            TakeoverOutcome::CancellationRequired { action_ids }
        };
        let started_action_ids = self.promote_queued_actions();
        Ok(ActionTakeoverEffect {
            outcome,
            input_state_changed: true,
            cancelled_action_ids,
            started_action_ids,
            cancellation_requested_action_ids,
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
            .order
            .iter()
            .filter(|action_id| {
                self.actions.get(*action_id).is_some_and(|action| {
                    matches!(
                        action.state,
                        EnvironmentActionState::Queued | EnvironmentActionState::Running
                    )
                })
            })
            .cloned()
            .collect();
        for action_id in &active_action_ids {
            if let Some(action) = self.actions.get_mut(action_id) {
                action.state = EnvironmentActionState::Failed;
                action.cancellation_requested = false;
                action.finished_at_ms = Some(next_action_timestamp(action));
                action.outcome = Some(EnvironmentActionOutcome::Failed {
                    code: EnvironmentActionFailureCode::ProcessLost,
                });
            }
            self.sync_history_action(action_id);
        }
        active_action_ids
    }

    pub(crate) fn begin_controller_recovery(&mut self) -> ActionRecoveryEffect {
        let failed_action_ids = self
            .order
            .iter()
            .filter(|action_id| {
                self.actions.get(*action_id).is_some_and(|action| {
                    action.state == EnvironmentActionState::Running
                        && action.mode == EnvironmentMode::Browser
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        for action_id in &failed_action_ids {
            self.fail_action_for_process_loss(action_id);
        }
        self.reservations
            .retain(|_, action_id| !failed_action_ids.contains(action_id));
        let ownership_changed = self.finalize_takeovers();
        ActionRecoveryEffect {
            failed_action_ids,
            ownership_changed,
            started_action_ids: Vec::new(),
        }
    }

    pub(crate) fn complete_controller_recovery(
        &mut self,
        runtime_generation: u64,
        tabs: &TabRegistry,
    ) -> ActionRecoveryEffect {
        let failed_action_ids =
            self.order
                .iter()
                .filter(|action_id| {
                    let Some(action) = self.actions.get(*action_id) else {
                        return false;
                    };
                    if action.state != EnvironmentActionState::Queued {
                        return false;
                    }
                    self.requests.get(*action_id).is_none_or(|request| {
                        request.runtime_generation != runtime_generation
                            || request.tab_preconditions.iter().any(
                                |(tab_id, document_revision)| {
                                    tabs.validate_reference(
                                        request.runtime_generation,
                                        runtime_generation,
                                        tab_id,
                                        *document_revision,
                                    )
                                    .is_err()
                                },
                            )
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
        for action_id in &failed_action_ids {
            self.fail_action_for_process_loss(action_id);
        }
        let ownership_changed = self.finalize_takeovers();
        let started_action_ids = self.promote_queued_actions();
        ActionRecoveryEffect {
            failed_action_ids,
            ownership_changed,
            started_action_ids,
        }
    }

    pub(crate) fn clear_ownership(&mut self) -> bool {
        let changed = !self.input_owners.is_empty() || !self.pending_takeovers.is_empty();
        self.input_owners.clear();
        self.pending_takeovers.clear();
        changed
    }

    fn fail_action_for_process_loss(&mut self, action_id: &str) {
        let action = self
            .actions
            .get_mut(action_id)
            .expect("recovery action should remain in the ledger");
        action.state = EnvironmentActionState::Failed;
        action.cancellation_requested = false;
        action.finished_at_ms = Some(next_action_timestamp(action));
        action.outcome = Some(EnvironmentActionOutcome::Failed {
            code: EnvironmentActionFailureCode::ProcessLost,
        });
        self.sync_history_action(action_id);
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
            let action = self
                .actions
                .get_mut(action_id)
                .expect("queued action should remain in the ledger");
            action.state = EnvironmentActionState::Cancelled;
            action.finished_at_ms = Some(next_action_timestamp(action));
            action.outcome = Some(EnvironmentActionOutcome::Cancelled {
                reason: EnvironmentActionCancellationReason::HumanTakeover,
            });
            self.sync_history_action(action_id);
        }
        action_ids
    }

    fn request_action_cancellation(&mut self, action_ids: &[String]) -> Vec<String> {
        let mut changed_action_ids = Vec::new();
        for action_id in action_ids {
            let action = self
                .actions
                .get_mut(action_id)
                .expect("blocking action should remain in the ledger");
            if !action.cancellation_requested {
                action.cancellation_requested = true;
                changed_action_ids.push(action_id.clone());
            }
            self.sync_history_action(action_id);
        }
        changed_action_ids
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
            let action = self
                .actions
                .get_mut(action_id)
                .expect("queued action should remain in the ledger");
            action.state = EnvironmentActionState::Running;
            action.started_at_ms = Some(next_action_timestamp(action));
            let history_action = action.clone();
            self.history_records
                .insert(history_action.sequence, history_action);
            started_action_ids.push(action_id.clone());
        }
        started_action_ids
    }

    pub(crate) fn compact_terminal_actions(&mut self) {
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
            self.actions.remove(&action_id);
            self.order.retain(|candidate| candidate != &action_id);
        }
    }

    fn sync_history_action(&mut self, action_id: &str) {
        let action = self
            .actions
            .get(action_id)
            .expect("Action history must reference a hot Action")
            .clone();
        self.history_records.insert(action.sequence, action);
    }
}

fn next_action_timestamp(action: &EnvironmentAction) -> u64 {
    crate::session::unix_epoch_ms().max(
        action
            .started_at_ms
            .unwrap_or(action.submitted_at_ms)
            .max(action.submitted_at_ms),
    )
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
