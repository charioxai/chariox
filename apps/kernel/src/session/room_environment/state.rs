use std::collections::BTreeMap;

use super::action::{
    ActionAdmission, ActionCancellationOutcome, EnvironmentActionHistoryPage,
    EnvironmentActionRequest, EnvironmentActionState, EnvironmentActionTerminal, EnvironmentMode,
    InputTarget,
};
use super::action_ledger::{
    ActionCancellationEffect, ActionRecoveryEffect, ActionTakeoverEffect, EnvironmentActionLedger,
};
use super::elements::{ElementReferenceRegistry, EnvironmentElementTarget};
use super::event::{EnvironmentEventKind, EnvironmentReplay};
use super::event_log::{EnvironmentEventLog, EnvironmentReplayPlan};
use super::model::{
    CanonicalViewport, EnvironmentActor, EnvironmentActorPresence, EnvironmentComponent,
    EnvironmentComponentHealth, EnvironmentComponentHealthState, EnvironmentError,
    EnvironmentLifecycle, EnvironmentPointer, EnvironmentPointerPosition,
    EnvironmentTabObservation, EnvironmentTabRuntimeBinding, RoomEnvironmentSnapshot,
};
use super::ownership::TakeoverOutcome;
use super::tabs::TabRegistry;

const DEFAULT_ACTION_QUEUE_CAPACITY: usize = 128;
const MAX_ACTION_HISTORY_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RoomEnvironment {
    pub(super) session_id: String,
    pub(super) environment_id: String,
    pub(super) runtime_generation: u64,
    pub(super) has_started: bool,
    pub(super) lifecycle: EnvironmentLifecycle,
    pub(super) viewport: CanonicalViewport,
    pub(super) health: BTreeMap<EnvironmentComponent, EnvironmentComponentHealth>,
    pub(super) actors: BTreeMap<String, EnvironmentActor>,
    pub(super) pointers: BTreeMap<String, EnvironmentPointer>,
    pub(super) tabs: TabRegistry,
    pub(super) element_references: ElementReferenceRegistry,
    pub(super) action_ledger: EnvironmentActionLedger,
    pub(super) browser_controller_recovering: bool,
    /// Ordinary Tabs' windows show Chromium's tab strip and address bar
    /// (maximized) instead of covering the desktop (fullscreen).
    /// A user's choice that survives a kernel restart like the browser does.
    #[serde(default)]
    pub(super) browser_bar_visible: bool,
    /// App view Tabs by controller target (installation and panel layout), and
    /// the focus agent their panels show. Kept across Tab churn so a Tab gets
    /// its marker once it appears. Not durable: the App view runtime publishes
    /// them again after a kernel restart.
    #[serde(skip)]
    pub(super) app_installations: BTreeMap<String, (String, super::model::AppPanelLayout)>,
    /// The browser controller lays App pages out beside the panel. An older
    /// one does not: its App Tabs are marked without a panel.
    #[serde(skip)]
    pub(super) app_panels: bool,
    #[serde(skip)]
    pub(super) panel_agent_id: Option<String>,
    pub(super) event_log: EnvironmentEventLog,
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
        Self::new_with_capacities(
            session_id,
            environment_id,
            viewport,
            event_capacity,
            DEFAULT_ACTION_QUEUE_CAPACITY,
        )
    }

    pub(crate) fn new_with_capacities(
        session_id: impl Into<String>,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
        event_capacity: usize,
        action_queue_capacity: usize,
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
            pointers: BTreeMap::new(),
            tabs: TabRegistry::new(),
            element_references: ElementReferenceRegistry::new(),
            action_ledger: EnvironmentActionLedger::new(event_capacity, action_queue_capacity),
            browser_controller_recovering: false,
            browser_bar_visible: false,
            app_installations: BTreeMap::new(),
            app_panels: false,
            panel_agent_id: None,
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
            pointers: self.pointers.values().cloned().collect(),
            tabs,
            focused_tab_id,
            actions: self.action_ledger.actions(),
            input_ownership: self.action_ledger.ownership(),
            pending_input_takeovers: self.action_ledger.pending_takeovers(),
            browser_bar_visible: self.browser_bar_visible,
            event_cursor: self.event_log.cursor(),
        }
    }

    pub fn action_history(
        &self,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> EnvironmentActionHistoryPage {
        self.action_ledger.action_history(
            before_sequence,
            limit.clamp(1, MAX_ACTION_HISTORY_PAGE_SIZE),
        )
    }

    #[cfg(test)]
    pub(crate) fn retained_action_request_count(&self) -> usize {
        self.action_ledger.retained_request_count()
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
        ) {
            if self.action_ledger.clear_ownership() {
                self.emit(EnvironmentEventKind::InputOwnershipChanged);
            }
            self.clear_pointers();
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
            self.mark_browser_components_starting();
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
        if created | self.mark_app_tabs() {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
        Ok(tab_id)
    }

    pub(crate) fn reconcile_controller_tabs(
        &mut self,
        observations: Vec<EnvironmentTabObservation>,
        focused_runtime_target_id: Option<&str>,
    ) {
        let changed = self
            .tabs
            .reconcile_controller_tabs(observations, focused_runtime_target_id)
            | self.mark_app_tabs();
        let input_ownership_changed = self.action_ledger.retain_input_targets(&self.tabs);
        self.element_references
            .retain_current(&self.tabs, self.runtime_generation);
        if changed {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
        if input_ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
    }

    pub(crate) fn restore_app_tab(&mut self, saved: &super::EnvironmentTab, target: &str) {
        if self.tabs.restore_app_tab(saved, target) {
            let input_ownership_changed = self.action_ledger.retain_input_targets(&self.tabs);
            self.element_references
                .retain_current(&self.tabs, self.runtime_generation);
            self.emit(EnvironmentEventKind::TabsChanged);
            if input_ownership_changed {
                self.emit(EnvironmentEventKind::InputOwnershipChanged);
            }
        }
    }

    pub(crate) fn controller_tab_binding(
        &self,
        tab_id: &str,
    ) -> Result<EnvironmentTabRuntimeBinding, EnvironmentError> {
        self.tabs.controller_binding(tab_id)
    }

    pub(crate) fn tab_id_for_controller_target(
        &self,
        controller_target_id: &str,
    ) -> Option<String> {
        self.tabs.tab_id_for_controller_target(controller_target_id)
    }

    /// The open App views (installation and panel layout by controller target)
    /// and the session's focus agent.
    pub(crate) fn prepare_app_recovery(&mut self) {
        self.tabs.prepare_app_recovery();
    }

    pub(crate) fn cancel_app_recovery_focus(&mut self) {
        self.tabs.cancel_app_recovery_focus();
    }

    pub(crate) fn set_app_tabs(
        &mut self,
        apps: BTreeMap<String, (String, super::model::AppPanelLayout)>,
        agent_id: Option<String>,
        app_panels: bool,
    ) {
        self.app_installations = apps;
        self.panel_agent_id = agent_id;
        self.app_panels = app_panels;
        if self.mark_app_tabs() {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
    }

    /// The session's focus agent, shown in every App view's panel.
    pub(crate) fn set_panel_agent(&mut self, agent_id: Option<String>) {
        self.panel_agent_id = agent_id;
        if self.mark_app_tabs() {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
    }

    fn mark_app_tabs(&mut self) -> bool {
        let apps = self
            .app_installations
            .iter()
            .map(|(target, (installation_id, layout))| {
                let app = super::model::EnvironmentTabApp {
                    installation_id: installation_id.clone(),
                    panel: self
                        .app_panels
                        .then(|| {
                            self.viewport
                                .app_layout(*layout, self.panel_agent_id.clone())
                                .1
                        })
                        .flatten(),
                };
                (target.clone(), app)
            })
            .collect();
        self.tabs.set_apps(&apps)
    }

    /// Each App page's CSS size: the canonical viewport less its panel.
    pub(crate) fn app_page_sizes(&self) -> BTreeMap<String, (u32, u32)> {
        self.app_installations
            .iter()
            .map(|(target, (_, layout))| {
                (target.clone(), self.viewport.app_layout(*layout, None).0)
            })
            .collect()
    }

    pub(crate) fn register_element_references(
        &mut self,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, String>, EnvironmentError> {
        self.element_references.register(
            &self.tabs,
            self.runtime_generation,
            tab_id,
            runtime_generation,
            document_revision,
            controller_node_refs,
        )
    }

    pub(crate) fn resolve_element_reference(
        &self,
        reference_id: &str,
    ) -> Result<EnvironmentElementTarget, EnvironmentError> {
        self.element_references
            .resolve(&self.tabs, self.runtime_generation, reference_id)
    }

    pub fn register_actor(&mut self, actor: EnvironmentActor) -> Result<(), EnvironmentError> {
        let changed = if let Some(existing) = self.actors.get_mut(&actor.actor_id) {
            if existing.kind != actor.kind {
                return Err(EnvironmentError::ActorKindConflict {
                    actor_id: actor.actor_id,
                });
            }
            if existing.display_label == actor.display_label
                && existing.presence == EnvironmentActorPresence::Present
            {
                false
            } else {
                existing.display_label = actor.display_label;
                existing.presence = EnvironmentActorPresence::Present;
                true
            }
        } else {
            self.actors.insert(actor.actor_id.clone(), actor);
            true
        };
        if changed {
            self.emit(EnvironmentEventKind::ActorsChanged);
        }
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
            let present_actor_ids = self
                .actors
                .iter()
                .filter_map(|(actor_id, actor)| {
                    (actor.presence == EnvironmentActorPresence::Present)
                        .then_some(actor_id.clone())
                })
                .collect::<std::collections::BTreeSet<_>>();
            let pointer_count = self.pointers.len();
            self.pointers
                .retain(|actor_id, _| present_actor_ids.contains(actor_id));
            if self.pointers.len() != pointer_count {
                self.emit(EnvironmentEventKind::PointersChanged);
            }
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
        if presence != EnvironmentActorPresence::Present {
            self.remove_pointer(actor_id);
        }
        Ok(())
    }

    pub fn update_pointer(
        &mut self,
        actor_id: &str,
        runtime_generation: u64,
        viewport_revision: u64,
        position: Option<EnvironmentPointerPosition>,
    ) -> Result<(), EnvironmentError> {
        let actor = self
            .actors
            .get(actor_id)
            .ok_or_else(|| EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            })?;
        if actor.presence != EnvironmentActorPresence::Present {
            return Err(EnvironmentError::UnknownActor {
                actor_id: actor_id.to_string(),
            });
        }
        self.validate_pointer_update(runtime_generation, viewport_revision, position)?;
        self.apply_pointer(actor_id, viewport_revision, position);
        Ok(())
    }

    pub fn update_pointer_as_actor(
        &mut self,
        actor: EnvironmentActor,
        runtime_generation: u64,
        viewport_revision: u64,
        position: Option<EnvironmentPointerPosition>,
    ) -> Result<(), EnvironmentError> {
        if let Some(existing) = self.actors.get(&actor.actor_id) {
            if existing.kind != actor.kind {
                return Err(EnvironmentError::ActorKindConflict {
                    actor_id: actor.actor_id,
                });
            }
        }
        self.validate_pointer_update(runtime_generation, viewport_revision, position)?;
        let actor_id = actor.actor_id.clone();
        if position.is_none() && !self.pointers.contains_key(&actor_id) {
            return Ok(());
        }
        self.register_actor(actor)?;
        self.apply_pointer(&actor_id, viewport_revision, position);
        Ok(())
    }

    fn validate_pointer_update(
        &self,
        runtime_generation: u64,
        viewport_revision: u64,
        position: Option<EnvironmentPointerPosition>,
    ) -> Result<(), EnvironmentError> {
        if !matches!(
            self.lifecycle,
            EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::EnvironmentNotReady {
                lifecycle: self.lifecycle,
            });
        }
        if runtime_generation != self.runtime_generation {
            return Err(EnvironmentError::StaleRuntimeGeneration {
                expected: self.runtime_generation,
                actual: runtime_generation,
            });
        }
        if viewport_revision != self.viewport.revision {
            return Err(EnvironmentError::StaleViewportRevision {
                expected: self.viewport.revision,
                actual: viewport_revision,
            });
        }
        if let Some(position) = position {
            if position.x >= self.viewport.desktop_pixel_width
                || position.y >= self.viewport.desktop_pixel_height
            {
                return Err(EnvironmentError::PointerOutOfBounds {
                    x: position.x,
                    y: position.y,
                    width: self.viewport.desktop_pixel_width,
                    height: self.viewport.desktop_pixel_height,
                });
            }
        }
        Ok(())
    }

    fn apply_pointer(
        &mut self,
        actor_id: &str,
        viewport_revision: u64,
        position: Option<EnvironmentPointerPosition>,
    ) {
        let changed = match position {
            Some(position) => {
                let pointer = EnvironmentPointer {
                    actor_id: actor_id.to_string(),
                    x: position.x,
                    y: position.y,
                    viewport_revision,
                };
                if self.pointers.get(actor_id) == Some(&pointer) {
                    false
                } else {
                    self.pointers.insert(actor_id.to_string(), pointer);
                    true
                }
            }
            None => self.pointers.remove(actor_id).is_some(),
        };
        if changed {
            self.emit(EnvironmentEventKind::PointersChanged);
        }
    }

    pub fn update_component_health(
        &mut self,
        component: EnvironmentComponent,
        state: EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) {
        let unchanged = self.health.get(&component).is_some_and(|health| {
            health.state == state && health.diagnostic_code.as_deref() == diagnostic_code
        });
        if !unchanged {
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
        self.settle_health_lifecycle();
    }

    // Component receipts and the aggregate are committed by the same owner.
    // Initial Start/Retry still needs its explicit completion, and recovery
    // cannot become Ready before the controller has reconciled its actions.
    fn settle_health_lifecycle(&mut self) {
        let healthy = !self.browser_controller_recovering
            && self
                .health
                .values()
                .all(|health| health.state == EnvironmentComponentHealthState::Ready);
        let next = match (self.lifecycle, healthy) {
            (EnvironmentLifecycle::Ready, false) => EnvironmentLifecycle::Degraded,
            (EnvironmentLifecycle::Degraded, true) => EnvironmentLifecycle::Ready,
            _ => return,
        };
        let _ = self.transition_to(next);
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

    /// Shows or hides the Room browser bar. Like a viewport change, it waits
    /// while another actor drives the desktop.
    pub fn set_browser_bar_visible_as_actor(
        &mut self,
        actor: EnvironmentActor,
        visible: bool,
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
            if owner_actor_id != actor.actor_id {
                return Err(EnvironmentError::InputOwnedByAnotherActor {
                    target: InputTarget::Desktop,
                    actor_id: owner_actor_id.to_string(),
                });
            }
        }
        self.register_actor(actor)?;
        // Existing clients refresh the snapshot on TabsChanged; a new event
        // kind would break their replay parsers.
        if self.browser_bar_visible != visible {
            self.browser_bar_visible = visible;
            self.emit(EnvironmentEventKind::TabsChanged);
        }
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
        self.clear_pointers();
        self.emit(EnvironmentEventKind::ViewportChanged {
            revision: self.viewport.revision,
        });
        if self.mark_app_tabs() {
            self.emit(EnvironmentEventKind::TabsChanged);
        }
    }

    pub fn submit_action(
        &mut self,
        request: EnvironmentActionRequest,
    ) -> Result<ActionAdmission, EnvironmentError> {
        let admission_lifecycle = self.input_lifecycle(request.mode);
        let admission = self.action_ledger.submit(
            request,
            admission_lifecycle,
            self.runtime_generation,
            &self.actors,
            &self.tabs,
        )?;
        match &admission {
            ActionAdmission::Accepted { action_id } => {
                self.tabs.cancel_app_recovery_focus();
                self.emit_action_changed(action_id, EnvironmentActionState::Running);
            }
            ActionAdmission::Queued { action_id, .. } => {
                self.tabs.cancel_app_recovery_focus();
                self.emit_action_changed(action_id, EnvironmentActionState::Queued);
            }
            _ => {}
        }
        Ok(admission)
    }

    pub(crate) fn existing_action(
        &self,
        request: &EnvironmentActionRequest,
    ) -> Result<Option<ActionAdmission>, EnvironmentError> {
        self.action_ledger.existing(request)
    }

    pub(crate) fn begin_browser_controller_recovery(&mut self) {
        self.browser_controller_recovering = true;
        self.element_references.clear();
        let effect = self.action_ledger.begin_controller_recovery();
        self.emit_action_recovery_effect(effect);
        self.settle_health_lifecycle();
    }

    pub(crate) fn complete_browser_controller_recovery(&mut self) {
        let effect = self
            .action_ledger
            .complete_controller_recovery(self.runtime_generation, &self.tabs);
        self.browser_controller_recovering = false;
        self.emit_action_recovery_effect(effect);
        self.settle_health_lifecycle();
    }

    fn input_lifecycle(&self, mode: EnvironmentMode) -> EnvironmentLifecycle {
        if self.browser_controller_recovering {
            return EnvironmentLifecycle::Starting;
        }
        if self.lifecycle == EnvironmentLifecycle::Starting
            && mode == EnvironmentMode::Browser
            && self.browser_components_ready()
        {
            EnvironmentLifecycle::Ready
        } else {
            self.lifecycle
        }
    }

    fn browser_components_ready(&self) -> bool {
        [
            EnvironmentComponent::BrowserController,
            EnvironmentComponent::Browser,
        ]
        .into_iter()
        .all(|component| {
            self.health
                .get(&component)
                .is_some_and(|health| health.state == EnvironmentComponentHealthState::Ready)
        })
    }

    pub fn finish_action(
        &mut self,
        action_id: &str,
        terminal: EnvironmentActionTerminal,
    ) -> Result<(), EnvironmentError> {
        let effect = self.action_ledger.finish(action_id, terminal)?;
        self.emit_action_changed(action_id, effect.state);
        if effect.ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        for started_action_id in effect.started_action_ids {
            self.emit_action_changed(&started_action_id, EnvironmentActionState::Running);
        }
        self.action_ledger.compact_terminal_actions();
        Ok(())
    }

    pub fn cancel_action(
        &mut self,
        actor_id: &str,
        action_id: &str,
    ) -> Result<ActionCancellationOutcome, EnvironmentError> {
        let ActionCancellationEffect {
            outcome,
            action_changed,
            started_action_ids,
        } = self
            .action_ledger
            .cancel_as_actor(actor_id, action_id, &self.actors)?;
        if action_changed {
            let state = match outcome {
                ActionCancellationOutcome::Cancelled => EnvironmentActionState::Cancelled,
                ActionCancellationOutcome::CancellationRequested => EnvironmentActionState::Running,
                ActionCancellationOutcome::AlreadyTerminal { action_state } => action_state,
            };
            self.emit_action_changed(action_id, state);
        }
        for started_action_id in started_action_ids {
            self.emit_action_changed(&started_action_id, EnvironmentActionState::Running);
        }
        self.action_ledger.compact_terminal_actions();
        Ok(outcome)
    }

    pub fn cancel_action_as_actor(
        &mut self,
        actor: EnvironmentActor,
        action_id: &str,
    ) -> Result<ActionCancellationOutcome, EnvironmentError> {
        let lifecycle = self
            .action_ledger
            .action(action_id)
            .map(|action| self.input_lifecycle(action.mode))
            .unwrap_or(self.lifecycle);
        if !matches!(
            lifecycle,
            EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::EnvironmentNotReady { lifecycle });
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
        let ActionCancellationEffect {
            outcome,
            action_changed,
            started_action_ids,
        } = action_ledger.cancel_as_actor(&actor.actor_id, action_id, &actors)?;
        let actors_changed = actors != self.actors;
        self.actors = actors;
        self.action_ledger = action_ledger;
        if actors_changed {
            self.emit(EnvironmentEventKind::ActorsChanged);
        }
        if action_changed {
            let state = match outcome {
                ActionCancellationOutcome::Cancelled => EnvironmentActionState::Cancelled,
                ActionCancellationOutcome::CancellationRequested => EnvironmentActionState::Running,
                ActionCancellationOutcome::AlreadyTerminal { action_state } => action_state,
            };
            self.emit_action_changed(action_id, state);
        }
        for started_action_id in started_action_ids {
            self.emit_action_changed(&started_action_id, EnvironmentActionState::Running);
        }
        self.action_ledger.compact_terminal_actions();
        Ok(outcome)
    }

    pub fn request_takeover(
        &mut self,
        actor_id: &str,
        target: InputTarget,
    ) -> Result<TakeoverOutcome, EnvironmentError> {
        let ActionTakeoverEffect {
            outcome,
            input_state_changed,
            cancelled_action_ids,
            started_action_ids,
            cancellation_requested_action_ids,
        } = self
            .action_ledger
            .request_takeover(actor_id, target, &self.actors, &self.tabs)?;
        for action_id in cancelled_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Cancelled);
        }
        for action_id in started_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Running);
        }
        for action_id in cancellation_requested_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Running);
        }
        if input_state_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        self.action_ledger.compact_terminal_actions();
        Ok(outcome)
    }

    pub fn request_takeover_as_actor(
        &mut self,
        actor: EnvironmentActor,
        target: InputTarget,
    ) -> Result<TakeoverOutcome, EnvironmentError> {
        let mode = match &target {
            InputTarget::BrowserTab(_) => EnvironmentMode::Browser,
            InputTarget::Desktop => EnvironmentMode::Computer,
        };
        let lifecycle = self.input_lifecycle(mode);
        if !matches!(
            lifecycle,
            EnvironmentLifecycle::Ready | EnvironmentLifecycle::Degraded
        ) {
            return Err(EnvironmentError::EnvironmentNotReady { lifecycle });
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
        let ActionTakeoverEffect {
            outcome,
            cancelled_action_ids,
            started_action_ids,
            cancellation_requested_action_ids,
            ..
        } = action_ledger.request_takeover(&actor.actor_id, target, &actors, &self.tabs)?;
        let actors_changed = actors != self.actors;
        let input_state_changed = action_ledger != self.action_ledger;
        self.actors = actors;
        self.action_ledger = action_ledger;
        if actors_changed {
            self.emit(EnvironmentEventKind::ActorsChanged);
        }
        for action_id in cancelled_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Cancelled);
        }
        for action_id in started_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Running);
        }
        for action_id in cancellation_requested_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Running);
        }
        if input_state_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        self.action_ledger.compact_terminal_actions();
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
        self.element_references
            .retain_current(&self.tabs, self.runtime_generation);
        self.emit(EnvironmentEventKind::TabsChanged);
        Ok(())
    }

    pub fn close_tab(&mut self, tab_id: &str) -> Result<(), EnvironmentError> {
        self.tabs.close(tab_id)?;
        let input_ownership_changed = self.action_ledger.retain_input_targets(&self.tabs);
        self.element_references
            .retain_current(&self.tabs, self.runtime_generation);
        self.emit(EnvironmentEventKind::TabsChanged);
        if input_ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
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
        self.browser_controller_recovering = false;
        self.runtime_generation += 1;
        self.lifecycle = EnvironmentLifecycle::Starting;
        self.tabs.clear();
        self.clear_pointers();
        self.element_references.clear();
        self.health = default_component_health();
        let failed_action_ids = self.action_ledger.invalidate_runtime();
        for action_id in failed_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Failed);
        }
        self.action_ledger.compact_terminal_actions();
        self.mark_browser_components_starting();
        self.emit(EnvironmentEventKind::RuntimeInvalidated);
        self.emit(EnvironmentEventKind::LifecycleChanged {
            lifecycle: EnvironmentLifecycle::Starting,
        });
    }

    fn mark_browser_components_starting(&mut self) {
        for component in [
            EnvironmentComponent::BrowserController,
            EnvironmentComponent::Browser,
        ] {
            self.update_component_health(
                component,
                EnvironmentComponentHealthState::Starting,
                None,
            );
        }
    }

    fn emit_action_recovery_effect(&mut self, effect: ActionRecoveryEffect) {
        for action_id in effect.failed_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Failed);
        }
        if effect.ownership_changed {
            self.emit(EnvironmentEventKind::InputOwnershipChanged);
        }
        for action_id in effect.started_action_ids {
            self.emit_action_changed(&action_id, EnvironmentActionState::Running);
        }
        self.action_ledger.compact_terminal_actions();
    }

    pub(super) fn clear_pointers(&mut self) {
        if !self.pointers.is_empty() {
            self.pointers.clear();
            self.emit(EnvironmentEventKind::PointersChanged);
        }
    }

    fn remove_pointer(&mut self, actor_id: &str) {
        if self.pointers.remove(actor_id).is_some() {
            self.emit(EnvironmentEventKind::PointersChanged);
        }
    }

    pub(super) fn emit(&mut self, kind: EnvironmentEventKind) {
        self.event_log
            .push(&self.environment_id, self.runtime_generation, kind);
    }

    pub(super) fn emit_action_changed(&mut self, action_id: &str, state: EnvironmentActionState) {
        let Some(action) = self.action_ledger.action(action_id) else {
            debug_assert!(
                false,
                "Action change must reference an Action in the ledger"
            );
            return;
        };
        debug_assert_eq!(action.state, state);
        let cancellation_requested = action.cancellation_requested;
        let submitted_at_ms = action.submitted_at_ms;
        let started_at_ms = action.started_at_ms;
        let finished_at_ms = action.finished_at_ms;
        let outcome = action.outcome;
        self.emit(EnvironmentEventKind::ActionChanged {
            action_id: action_id.to_string(),
            state,
            cancellation_requested,
            submitted_at_ms,
            started_at_ms,
            finished_at_ms,
            outcome,
        });
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
