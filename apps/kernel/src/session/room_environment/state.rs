use std::collections::BTreeMap;

use super::action::{
    ActionAdmission, EnvironmentActionRequest, EnvironmentActionState, EnvironmentActionTerminal,
    InputTarget,
};
use super::action_ledger::EnvironmentActionLedger;
use super::event::{EnvironmentEventKind, EnvironmentReplay};
use super::event_log::{EnvironmentEventLog, EnvironmentReplayPlan};
use super::model::{
    CanonicalViewport, EnvironmentActor, EnvironmentActorPresence, EnvironmentComponent,
    EnvironmentComponentHealth, EnvironmentComponentHealthState, EnvironmentError,
    EnvironmentLifecycle, RoomEnvironmentSnapshot,
};
use super::ownership::TakeoverOutcome;
use super::tabs::TabRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomEnvironment {
    session_id: String,
    environment_id: String,
    runtime_generation: u64,
    has_started: bool,
    lifecycle: EnvironmentLifecycle,
    viewport: CanonicalViewport,
    health: BTreeMap<EnvironmentComponent, EnvironmentComponentHealth>,
    actors: BTreeMap<String, EnvironmentActor>,
    tabs: TabRegistry,
    action_ledger: EnvironmentActionLedger,
    event_log: EnvironmentEventLog,
}

impl RoomEnvironment {
    pub fn new(
        session_id: impl Into<String>,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
    ) -> Result<Self, EnvironmentError> {
        Self::new_with_event_capacity(session_id, environment_id, viewport, 128)
    }

    pub fn new_with_event_capacity(
        session_id: impl Into<String>,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
        event_capacity: usize,
    ) -> Result<Self, EnvironmentError> {
        Ok(Self {
            session_id: session_id.into(),
            environment_id: environment_id.into(),
            runtime_generation: 1,
            has_started: false,
            lifecycle: EnvironmentLifecycle::Stopped,
            viewport,
            health: default_component_health(),
            actors: BTreeMap::new(),
            tabs: TabRegistry::new(),
            action_ledger: EnvironmentActionLedger::new(event_capacity),
            event_log: EnvironmentEventLog::new(event_capacity)?,
        })
    }

    pub fn snapshot(&self) -> RoomEnvironmentSnapshot {
        let (tabs, focused_tab_id) = self.tabs.snapshot();
        RoomEnvironmentSnapshot {
            session_id: self.session_id.clone(),
            environment_id: self.environment_id.clone(),
            runtime_generation: self.runtime_generation,
            lifecycle: self.lifecycle,
            health: self.health.values().cloned().collect(),
            viewport: self.viewport.clone(),
            actors: self.actors.values().cloned().collect(),
            tabs,
            focused_tab_id,
            actions: self.action_ledger.actions(),
            input_ownership: self.action_ledger.ownership(),
            pending_input_takeovers: self.action_ledger.pending_takeovers(),
            event_cursor: self.event_log.cursor(),
        }
    }

    pub fn transition_to(&mut self, next: EnvironmentLifecycle) -> Result<(), EnvironmentError> {
        if !allows_transition(self.lifecycle, next) {
            return Err(EnvironmentError::InvalidLifecycleTransition {
                from: self.lifecycle,
                to: next,
            });
        }
        self.lifecycle = next;
        if matches!(
            next,
            EnvironmentLifecycle::Stopped | EnvironmentLifecycle::Failed
        ) && self.action_ledger.clear_ownership()
        {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        self.emit(EnvironmentEventKind::LifecycleChanged { lifecycle: next });
        Ok(())
    }

    pub fn start_runtime(&mut self) -> Result<(), EnvironmentError> {
        if !matches!(
            self.lifecycle,
            EnvironmentLifecycle::Stopped
                | EnvironmentLifecycle::Failed
                | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::InvalidLifecycleTransition {
                from: self.lifecycle,
                to: EnvironmentLifecycle::Starting,
            });
        }
        if self.has_started {
            self.invalidate_runtime();
        } else {
            self.has_started = true;
            self.lifecycle = EnvironmentLifecycle::Starting;
            self.emit(EnvironmentEventKind::LifecycleChanged {
                lifecycle: EnvironmentLifecycle::Starting,
            });
        }
        Ok(())
    }

    pub fn reset_runtime(&mut self) -> Result<(), EnvironmentError> {
        if !matches!(
            self.lifecycle,
            EnvironmentLifecycle::Stopped | EnvironmentLifecycle::Failed
        ) {
            return Err(EnvironmentError::InvalidLifecycleTransition {
                from: self.lifecycle,
                to: EnvironmentLifecycle::Starting,
            });
        }
        self.invalidate_runtime();
        Ok(())
    }

    pub fn invalidate_runtime_after_process_loss(&mut self) -> Result<(), EnvironmentError> {
        self.invalidate_runtime();
        Ok(())
    }

    pub fn register_or_reconcile_tab(
        &mut self,
        controller_target_id: impl Into<String>,
        url: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<String, EnvironmentError> {
        let (tab_id, created) =
            self.tabs
                .register_or_reconcile(controller_target_id.into(), url.into(), title.into());
        if created {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
        Ok(tab_id)
    }

    pub fn register_actor(&mut self, actor: EnvironmentActor) -> Result<(), EnvironmentError> {
        if let Some(existing) = self.actors.get_mut(&actor.actor_id) {
            if existing.kind != actor.kind {
                return Err(EnvironmentError::ActorKindConflict {
                    actor_id: actor.actor_id,
                });
            }
            existing.display_label = actor.display_label;
            existing.presence = EnvironmentActorPresence::Present;
        } else {
            self.actors.insert(actor.actor_id.clone(), actor);
        }
        self.emit(EnvironmentEventKind::ActorsChanged);
        Ok(())
    }

    pub fn reconcile_actors(
        &mut self,
        actors: Vec<EnvironmentActor>,
    ) -> Result<(), EnvironmentError> {
        let mut reconciled = self.actors.clone();
        for actor in reconciled.values_mut() {
            actor.presence = EnvironmentActorPresence::Disconnected;
        }
        for mut actor in actors {
            if let Some(existing) = reconciled.get(&actor.actor_id) {
                if existing.kind != actor.kind {
                    return Err(EnvironmentError::ActorKindConflict {
                        actor_id: actor.actor_id,
                    });
                }
            }
            actor.presence = EnvironmentActorPresence::Present;
            reconciled.insert(actor.actor_id.clone(), actor);
        }
        if reconciled != self.actors {
            self.actors = reconciled;
            self.emit(EnvironmentEventKind::ActorsChanged);
        }
        Ok(())
    }

    pub fn set_actor_presence(
        &mut self,
        actor_id: &str,
        presence: EnvironmentActorPresence,
    ) -> Result<(), EnvironmentError> {
        self.actors
            .get_mut(actor_id)
            .ok_or_else(|| EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            })?
            .presence = presence;
        self.emit(EnvironmentEventKind::ActorsChanged);
        Ok(())
    }

    pub fn update_component_health(
        &mut self,
        component: EnvironmentComponent,
        state: EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) {
        self.health.insert(
            component,
            EnvironmentComponentHealth {
                component,
                state,
                diagnostic_code: diagnostic_code.map(str::to_string),
            },
        );
        self.emit(EnvironmentEventKind::HealthChanged);
    }

    pub fn update_viewport(
        &mut self,
        actor_id: &str,
        expected_revision: u64,
        replacement: CanonicalViewport,
    ) -> Result<(), EnvironmentError> {
        if !self.actors.contains_key(actor_id) {
            return Err(EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            });
        }
        self.validate_viewport_update(actor_id, expected_revision)?;
        self.apply_viewport(actor_id, replacement);
        Ok(())
    }

    pub fn update_viewport_as_actor(
        &mut self,
        actor: EnvironmentActor,
        expected_revision: u64,
        replacement: CanonicalViewport,
    ) -> Result<(), EnvironmentError> {
        self.validate_viewport_update(&actor.actor_id, expected_revision)?;
        let actor_id = actor.actor_id.clone();
        self.register_actor(actor)?;
        self.apply_viewport(&actor_id, replacement);
        Ok(())
    }

    fn validate_viewport_update(
        &self,
        actor_id: &str,
        expected_revision: u64,
    ) -> Result<(), EnvironmentError> {
        if !matches!(
            self.lifecycle,
            EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::EnvironmentNotReady {
                lifecycle: self.lifecycle,
            });
        }
        if let Some(owner_actor_id) = self.action_ledger.owner(&InputTarget::Desktop) {
            if owner_actor_id != actor_id {
                return Err(EnvironmentError::InputOwnedByAnotherActor {
                    target: InputTarget::Desktop,
                    actor_id: owner_actor_id.to_string(),
                });
            }
        }
        if expected_revision != self.viewport.revision {
            return Err(EnvironmentError::StaleViewportRevision {
                expected: self.viewport.revision,
                actual: expected_revision,
            });
        }
        Ok(())
    }

    fn apply_viewport(&mut self, actor_id: &str, mut replacement: CanonicalViewport) {
        replacement.revision = self.viewport.revision + 1;
        replacement.last_actor_id = Some(actor_id.to_string());
        self.viewport = replacement;
        self.emit(EnvironmentEventKind::ViewportChanged {
            revision: self.viewport.revision,
        });
    }

    pub fn submit_action(
        &mut self,
        request: EnvironmentActionRequest,
    ) -> Result<ActionAdmission, EnvironmentError> {
        let admission = self.action_ledger.submit(
            request,
            self.lifecycle,
            self.runtime_generation,
            &self.actors,
            &self.tabs,
        )?;
        if let ActionAdmission::Accepted { action_id } = &admission {
            self.emit(EnvironmentEventKind::ActionChanged {
                action_id: action_id.clone(),
                state: EnvironmentActionState::Running,
            });
        }
        Ok(admission)
    }

    pub fn finish_action(
        &mut self,
        action_id: &str,
        terminal: EnvironmentActionTerminal,
    ) -> Result<(), EnvironmentError> {
        let effect = self.action_ledger.finish(action_id, terminal)?;
        self.emit(EnvironmentEventKind::ActionChanged {
            action_id: action_id.to_string(),
            state: effect.state,
        });
        if effect.ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        Ok(())
    }

    pub fn request_takeover(
        &mut self,
        actor_id: &str,
        target: InputTarget,
    ) -> Result<TakeoverOutcome, EnvironmentError> {
        let (outcome, ownership_changed) =
            self.action_ledger
                .request_takeover(actor_id, target, &self.actors, &self.tabs)?;
        if ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        Ok(outcome)
    }

    pub fn request_takeover_as_actor(
        &mut self,
        actor: EnvironmentActor,
        target: InputTarget,
    ) -> Result<TakeoverOutcome, EnvironmentError> {
        if !matches!(
            self.lifecycle,
            EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::EnvironmentNotReady {
                lifecycle: self.lifecycle,
            });
        }

        let mut actors = self.actors.clone();
        if let Some(existing) = actors.get(&actor.actor_id) {
            if existing.kind != actor.kind {
                return Err(EnvironmentError::ActorKindConflict {
                    actor_id: actor.actor_id,
                });
            }
        }
        actors.insert(actor.actor_id.clone(), actor.clone());

        let mut action_ledger = self.action_ledger.clone();
        let (outcome, _) =
            action_ledger.request_takeover(&actor.actor_id, target, &actors, &self.tabs)?;
        let actors_changed = actors != self.actors;
        let input_state_changed = action_ledger != self.action_ledger;
        self.actors = actors;
        self.action_ledger = action_ledger;
        if actors_changed {
            self.emit(EnvironmentEventKind::ActorsChanged);
        }
        if input_state_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        Ok(outcome)
    }

    pub fn release_input(
        &mut self,
        actor_id: &str,
        target: &InputTarget,
    ) -> Result<(), EnvironmentError> {
        self.action_ledger.release(actor_id, target)?;
        self.emit(EnvironmentEventKind::InputOwnershipChanged);
        Ok(())
    }

    pub fn record_navigation(
        &mut self,
        tab_id: &str,
        url: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<(), EnvironmentError> {
        self.tabs
            .record_navigation(tab_id, url.into(), title.into())?;
        self.emit(EnvironmentEventKind::TabsChanged);
        Ok(())
    }

    pub fn close_tab(&mut self, tab_id: &str) -> Result<(), EnvironmentError> {
        self.tabs.close(tab_id)?;
        self.emit(EnvironmentEventKind::TabsChanged);
        Ok(())
    }

    pub fn events_after(&self, cursor: u64) -> EnvironmentReplay {
        match self.event_log.replay(cursor) {
            EnvironmentReplayPlan::Events {
                events,
                next_cursor,
            } => EnvironmentReplay::Events {
                events,
                next_cursor,
            },
            EnvironmentReplayPlan::SnapshotRequired => EnvironmentReplay::SnapshotRequired {
                snapshot: Box::new(self.snapshot()),
            },
        }
    }

    pub fn validate_tab_reference(
        &self,
        runtime_generation: u64,
        tab_id: &str,
        document_revision: u64,
    ) -> Result<(), EnvironmentError> {
        self.tabs.validate_reference(
            runtime_generation,
            self.runtime_generation,
            tab_id,
            document_revision,
        )
    }

    fn invalidate_runtime(&mut self) {
        self.has_started = true;
        self.runtime_generation += 1;
        self.lifecycle = EnvironmentLifecycle::Starting;
        self.tabs.clear();
        self.health = default_component_health();
        let failed_action_ids = self.action_ledger.invalidate_runtime();
        for action_id in failed_action_ids {
            self.emit(EnvironmentEventKind::ActionChanged {
                action_id,
                state: EnvironmentActionState::Failed,
            });
        }
        self.emit(EnvironmentEventKind::RuntimeInvalidated);
        self.emit(EnvironmentEventKind::LifecycleChanged {
            lifecycle: EnvironmentLifecycle::Starting,
        });
    }

    fn emit(&mut self, kind: EnvironmentEventKind) {
        self.event_log
            .push(&self.environment_id, self.runtime_generation, kind);
    }
}

fn default_component_health() -> BTreeMap<EnvironmentComponent, EnvironmentComponentHealth> {
    [
        EnvironmentComponent::BrowserController,
        EnvironmentComponent::Browser,
        EnvironmentComponent::Desktop,
        EnvironmentComponent::Streamer,
    ]
    .into_iter()
    .map(|component| {
        (
            component,
            EnvironmentComponentHealth {
                component,
                state: EnvironmentComponentHealthState::Unavailable,
                diagnostic_code: None,
            },
        )
    })
    .collect()
}

fn allows_transition(from: EnvironmentLifecycle, to: EnvironmentLifecycle) -> bool {
    use EnvironmentLifecycle::*;
    matches!(
        (from, to),
        (Starting, Ready | Degraded | Failed | Stopping)
            | (Ready, Degraded | Saving | Restoring | Stopping | Failed)
            | (Degraded, Ready | Saving | Restoring | Stopping | Failed)
            | (Saving, Ready | Degraded | Stopping | Failed)
            | (Restoring, Ready | Degraded | Stopping | Failed)
            | (Stopping, Stopped | Failed)
            | (Failed, Stopped)
    )
}
