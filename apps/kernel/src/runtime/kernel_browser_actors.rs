//! MD-3: sessionless attachment of the shared Room actor/input model.
use super::browser_controller_process::BrowserCancellation;
use crate::session::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// MP-08/MP-11: a human actor's Desktop ownership lasts while one of its
/// desktop video subscriptions is live; the host display lease is 60 s.
pub(crate) const DESKTOP_VIEWER_LEASE: Duration = Duration::from_secs(60);
pub(crate) const DISPLAY_RESUME_GRACE: Duration = Duration::from_secs(5);
/// MP-10: a person's Tab takeover outlives the viewer that took it (reload, network loss, relay link
/// drop) and ends by Return control or after this long without input from the person.
pub(crate) const TAKEOVER_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
/// What an agent reads when its action meets a Tab a person has taken over: the wait it should declare,
/// so the kernel continues it when control returns.
pub(crate) const TAB_TAKEN_OVER_REFUSAL: &str = "MD-3: human_takeover: a person has control of this browser Tab. Call chariox.events.blocked with the reason \"human_takeover: waiting for the person to return control\" and end your turn; the kernel continues you when they return control.";

mod tabs;
use tabs::BrowserTabActivity;

pub(crate) struct KernelBrowserActors {
    tab_activity: BrowserTabActivity,
    generation: u64,
    observation: u64,
    desktop: Option<(String, String)>,

    viewport: (u64, u64),
    tabs: TabRegistry,
    actors: BTreeMap<String, EnvironmentActor>,
    /// When each actor last registered work: a full model retires the longest idle one.
    seen: BTreeMap<String, Instant>,
    ledger: EnvironmentActionLedger,
    active: BTreeMap<String, Arc<BrowserCancellation>>,
    pointers: BTreeMap<String, EnvironmentPointer>,
    pointer_tabs: BTreeMap<String, String>,
    host_tabs: BTreeMap<String, String>,
    /// Desktop video subscription -> (human actor, lease expiry).
    desktop_viewers: BTreeMap<String, (String, Instant)>,
    /// Human actor holding or awaiting a Tab takeover -> its last input; the takeover ends `TAKEOVER_IDLE_TIMEOUT` later.
    takeover_input: BTreeMap<String, Instant>,
    /// Human actors whose viewer went away while holding a Tab takeover.
    orphaned_takeovers: BTreeSet<String>,
}

/// Internal display seam: the adapter must derive the user/actor from its caller.
/// Binding travels with the protected frame, never inferred from a later poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelBrowserDocumentBinding {
    pub(crate) tab_id: String,
    pub(crate) generation: u64,
    pub(crate) document_id: String,
}

impl Default for KernelBrowserActors {
    fn default() -> Self {
        Self {
            tab_activity: BrowserTabActivity::default(),
            generation: 0,
            observation: 0,
            desktop: None,

            viewport: (1280, 800),
            tabs: TabRegistry::new(),
            actors: BTreeMap::new(),
            seen: BTreeMap::new(),
            ledger: EnvironmentActionLedger::new(256, 0),
            active: BTreeMap::new(),
            pointers: BTreeMap::new(),
            pointer_tabs: BTreeMap::new(),
            host_tabs: BTreeMap::new(),
            desktop_viewers: BTreeMap::new(),
            takeover_input: BTreeMap::new(),
            orphaned_takeovers: BTreeSet::new(),
        }
    }
}
impl KernelBrowserActors {
    pub(crate) fn reconcile(&mut self, state: &Value) -> Result<(), String> {
        self.reconcile_inventory(state, &state["_tab_creation_actions"])
            .map(drop)
    }
    /// Returns the Tabs this inventory first attributed to the actor whose admitted action opened them.
    pub(crate) fn reconcile_inventory(
        &mut self,
        state: &Value,
        creation_actions: &Value,
    ) -> Result<Vec<(String, EnvironmentActor)>, String> {
        let Some(generation) = state["generation"].as_u64() else {
            return Ok(Vec::new());
        };
        let Some(tabs) = state["tabs"].as_array() else {
            return Ok(Vec::new());
        };
        // MP-08/MP-10/MP-11: concurrent controller reads finish out of order; the
        // controller numbers its observations within a generation.
        let observation = state["_observation"].as_u64();
        if generation < self.generation
            || generation == self.generation
                && observation.is_some_and(|observation| observation <= self.observation)
        {
            return Ok(Vec::new());
        }
        self.observation = observation.unwrap_or(0);
        if generation != self.generation {
            for cancellation in self.active.values() {
                cancellation.request_cancel();
            }
            self.ledger.invalidate_after_kernel_restart(&self.actors);
            self.active.clear();
            self.pointers.clear();
            self.pointer_tabs.clear();
            self.generation = generation;
            self.tab_activity.browser_restarted();
        }
        if let (Some(width), Some(height)) = (
            state["viewport"]["css_width"].as_u64(),
            state["viewport"]["css_height"].as_u64(),
        ) {
            if matches!((width, height), (1280, 800) | (1920, 1080)) {
                self.viewport = (width, height);
            }
        }
        self.tab_activity.retain(&state["tabs"]);
        // MP-08: process delayed creation evidence before the tab becomes known.
        // A read, attach, or another tab's mutation can be its first inventory.
        let discovered = tabs
            .iter()
            .filter_map(|tab| {
                let id = tab["tab_id"].as_str()?;
                if self.tab(id).is_ok() {
                    return None;
                }
                let action = creation_actions[id].as_str()?;
                let actor = self.tab_activity.creation_actor(action)?;
                Some((id.to_string(), actor.clone()))
            })
            .collect::<Vec<_>>();
        let observations = tabs
            .iter()
            .map(|tab| {
                Ok(EnvironmentTabObservation {
                    runtime_target_id: tab["tab_id"]
                        .as_str()
                        .ok_or("MD-3: missing tab identity")?
                        .into(),
                    document_id: tab["document_id"]
                        .as_str()
                        .ok_or("MD-3: missing document identity")?
                        .into(),
                    // URLs/titles are protected by the existing observation boundary.
                    url: String::new(),
                    title: String::new(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.tabs.reconcile_controller_tabs(observations, None);
        self.ledger.retain_input_targets(&self.tabs);
        for tab in self.tabs.snapshot().0 {
            let binding = self
                .tabs
                .controller_binding(&tab.tab_id)
                .map_err(|_| "MD-3: missing document")?;
            self.host_tabs.insert(tab.tab_id, binding.runtime_target_id);
        }
        let mut retained = self
            .tabs
            .snapshot()
            .0
            .into_iter()
            .map(|tab| tab.tab_id)
            .collect::<std::collections::BTreeSet<_>>();
        for action in self.ledger.actions() {
            retained.extend(action.targets.into_iter().filter_map(|target| {
                if let InputTarget::BrowserTab(tab) = target {
                    Some(tab)
                } else {
                    None
                }
            }));
        }
        self.host_tabs.retain(|tab, _| retained.contains(tab));
        for (tab, actor) in &discovered {
            self.opened_tab(actor.clone(), tab);
        }
        Ok(discovered)
    }
    // MP-08/MP-10/MP-11: reuse only an already observed exact generation/document.
    // The controller still checks that live document before physical dispatch.
    pub(crate) fn input_is_current(&self, params: &Value) -> bool {
        if params["generation"].as_u64() != Some(self.generation) {
            return false;
        }
        let Some(host) = params["tab_id"].as_str() else {
            return false;
        };
        self.tab(host)
            .ok()
            .and_then(|tab| self.tabs.controller_binding(&tab).ok())
            .is_some_and(|binding| {
                params["document_id"].as_str() == Some(binding.document_id.as_str())
            })
    }
    // MP-08/MP-10: the private host already admitted the person's Chrome UI
    // window and active Tab. A home-page commit between that read and this
    // ledger admission must not lose omnibox keys. Attribute them to the
    // current document without replaying an action or changing its Tab/actor.
    pub(crate) fn refresh_chrome_input_document(&self, params: &mut Value) -> Result<(), String> {
        if params["op"] != "input" || params["generation"].as_u64() != Some(self.generation) {
            return Err("MD-3: stale Chrome input generation".into());
        }
        let tab = self.tab(
            params["tab_id"]
                .as_str()
                .ok_or("MD-3: missing Chrome input Tab")?,
        )?;
        let binding = self
            .tabs
            .controller_binding(&tab)
            .map_err(|_| "MD-3: missing Chrome input document")?;
        params["document_id"] = binding.document_id.clone().into();
        Ok(())
    }
    fn tab(&self, host_tab: &str) -> Result<String, String> {
        self.tabs
            .tab_id_for_controller_target(host_tab)
            .filter(|id| self.tabs.contains(id))
            .ok_or_else(|| "MD-3: unknown user-domain tab; refresh state".into())
    }
    fn register(&mut self, actor: EnvironmentActor) -> Result<(), String> {
        if !self.actors.contains_key(&actor.actor_id) && self.actors.len() >= 64 {
            // A viewer that closed its page over the relay never disconnects: make room by
            // retiring the longest idle actor that holds no input, action or desktop view.
            let idle = self
                .actors
                .keys()
                .filter(|id| !self.holds_work(id))
                .min_by_key(|id| self.seen.get(*id))
                .cloned()
                .ok_or("MD-3: browser actor limit reached")?;
            self.disconnect(&idle);
        }
        if self
            .actors
            .get(&actor.actor_id)
            .is_some_and(|old| old.kind != actor.kind)
        {
            return Err("MD-3: browser actor kind conflict".into());
        }
        self.seen.insert(actor.actor_id.clone(), Instant::now());
        self.actors.insert(actor.actor_id.clone(), actor);
        Ok(())
    }
    fn holds_work(&self, actor_id: &str) -> bool {
        self.ledger
            .ownership()
            .iter()
            .any(|owner| owner.actor_id == actor_id)
            || self
                .ledger
                .pending_takeovers()
                .iter()
                .any(|pending| pending.human_actor_id == actor_id)
            || self
                .desktop_viewers
                .values()
                .any(|(actor, _)| actor == actor_id)
            || self.ledger.actions().iter().any(|action| {
                action.actor_id == actor_id && self.active.contains_key(&action.action_id)
            })
    }
    /// MP-08/MP-11: dialog answers cannot reserve the lane held by the action
    /// that opened them, but still honor both active and pending human takeover.
    pub(crate) fn check_dialog_takeover(&mut self, host_tab: &str) -> Result<(), String> {
        self.reclaim_idle_takeovers(Instant::now());
        let tab = InputTarget::BrowserTab(self.tab(host_tab)?);
        let targets = [tab, InputTarget::Desktop];
        if targets
            .iter()
            .any(|target| self.ledger.owner(target).is_some())
            || self
                .ledger
                .pending_takeovers()
                .iter()
                .any(|pending| targets.contains(&pending.target))
        {
            return Err(TAB_TAKEN_OVER_REFUSAL.into());
        }
        Ok(())
    }
    pub(crate) fn dialog_answered(&mut self, actor: &EnvironmentActor, host_tab: &str) {
        self.tab_activity.acted(actor, host_tab);
    }
    pub(crate) fn begin(
        &mut self,
        actor: EnvironmentActor,
        params: &Value,
    ) -> Result<(String, Arc<BrowserCancellation>), String> {
        self.retire_lapsed_desktop_viewers(Instant::now());
        self.reclaim_idle_takeovers(Instant::now());
        self.register(actor.clone())?;
        self.adopt_orphaned_takeovers(&actor.actor_id);
        if let Some(host_tab) = params["tab_id"].as_str() {
            self.follow_person_takeover(&actor.actor_id, host_tab);
        }
        self.note_person_input(&actor.actor_id);
        let mut request = if let Some(host_tab) = params["tab_id"].as_str() {
            let tab = self.tab(host_tab)?;
            let binding = self
                .tabs
                .controller_binding(&tab)
                .map_err(|_| "MD-3: missing tab document")?;
            if params["generation"].as_u64() != Some(self.generation) {
                return Err("MD-3: stale browser generation".into());
            }
            EnvironmentActionRequest::browser_mutation(
                &actor.actor_id,
                self.generation,
                params["op"].as_str().unwrap_or("vault_input"),
                &tab,
                binding.document_revision,
            )
        } else {
            let mut request = EnvironmentActionRequest::computer_mutation(
                &actor.actor_id,
                self.generation,
                params["op"].as_str().unwrap_or("browser_mutation"),
                None,
            );
            request.mode = if params["_native"] == true {
                EnvironmentMode::Computer
            } else {
                EnvironmentMode::Browser
            };
            if params["op"] == "stop" {
                request.targets.extend(
                    self.tabs
                        .snapshot()
                        .0
                        .into_iter()
                        .map(|tab| InputTarget::BrowserTab(tab.tab_id)),
                );
            }
            request
        };
        if !request.targets.contains(&InputTarget::Desktop) {
            request.targets.push(InputTarget::Desktop);
        }
        // Whole-desktop native input can affect any foreground browser tab.
        // Honor existing tab-scoped human takeovers without widening ordinary
        // Browser-to-Browser ownership to unrelated tabs.
        if params["_native"] == true {
            request.targets.extend(
                self.tabs
                    .snapshot()
                    .0
                    .into_iter()
                    .map(|tab| InputTarget::BrowserTab(tab.tab_id)),
            );
        }
        // Store counts and coordinates only; payloads/secret bytes never enter the ledger.
        let outer = &params["input"];
        let input = if outer["kind"] == "mirror" {
            if outer["action"]["kind"] == "coordinate" {
                &outer["action"]["input"]
            } else {
                &outer["action"]
            }
        } else {
            outer
        };
        let request = match input["kind"].as_str() {
            Some("text") => request.with_arguments(EnvironmentActionArguments::KeyboardText {
                utf8_byte_count: input["text"].as_str().unwrap_or_default().len() as u32,
                character_count: input["text"].as_str().unwrap_or_default().chars().count() as u32,
            }),
            Some("click") => request.with_arguments(EnvironmentActionArguments::PointerClick {
                x: input["x"].as_u64().unwrap_or_default() as u32,
                y: input["y"].as_u64().unwrap_or_default() as u32,
                button: EnvironmentPointerButton::Left,
                click_count: 1,
                viewport_revision: 1,
            }),
            _ => request,
        };
        let action_id = match self
            .ledger
            .submit(
                request,
                EnvironmentLifecycle::Ready,
                self.generation,
                &self.actors,
                &self.tabs,
            )
            .map_err(|_| "MD-3: browser action admission failed")?
        {
            ActionAdmission::Accepted { action_id } => action_id,
            ActionAdmission::RejectedTakeover { .. } => return Err(TAB_TAKEN_OVER_REFUSAL.into()),
            _ => return Err("MD-3: browser action lane busy".into()),
        };
        if let (Some(x), Some(y)) = (input["x"].as_u64(), input["y"].as_u64()) {
            if x < self.viewport.0 && y < self.viewport.1 {
                if let Some(tab) = params["tab_id"].as_str() {
                    self.pointer_tabs.insert(actor.actor_id.clone(), tab.into());
                }
                self.pointers.insert(
                    actor.actor_id.clone(),
                    EnvironmentPointer {
                        actor_id: actor.actor_id.clone(),
                        x: x as u32,
                        y: y as u32,
                        viewport_revision: 1,
                    },
                );
            }
        }
        if let Some(tab) = params["tab_id"].as_str() {
            let binding = self
                .tabs
                .controller_binding(&self.tab(tab)?)
                .map_err(|_| "MD-3: missing tab document")?;
            self.tab_activity
                .remember_action(&action_id, &actor, tab, &binding.document_id);
            self.tab_activity.acted(&actor, tab);
        }
        let cancellation = Arc::new(BrowserCancellation::default());
        self.active.insert(action_id.clone(), cancellation.clone());
        Ok((action_id, cancellation))
    }
    pub(crate) fn finish(&mut self, action_id: &str, terminal: EnvironmentActionTerminal) {
        self.active.remove(action_id);
        if terminal != EnvironmentActionTerminal::Completed {
            self.tab_activity.forget_action(action_id);
        }
        // A reconciled process loss can have already finished this action.
        let _ = self.ledger.finish(action_id, terminal);
        self.ledger.compact_terminal_actions();
        self.ledger.compact_transient_history(256);
        self.tab_activity
            .retain_actions(|id| self.ledger.action(id).is_some());
    }
    pub(crate) fn reconcile_desktop(&mut self, state: &Value) -> Result<(), String> {
        let binding = (
            state["surface_id"]
                .as_str()
                .ok_or("MP-11: invalid desktop surface")?
                .to_string(),
            state["generation"]
                .as_str()
                .ok_or("MP-11: invalid desktop generation")?
                .to_string(),
        );
        if self.desktop.as_ref().is_some_and(|old| old != &binding) {
            for cancel in self.active.values() {
                cancel.request_cancel();
            }
            // A fresh desktop never adopts prior desktop ownership or pending input.
            self.ledger.clear_ownership();
        }
        self.desktop = Some(binding);
        Ok(())
    }
    fn desktop_current(&self, surface: &str, generation: &str) -> Result<(), String> {
        if self
            .desktop
            .as_ref()
            .is_none_or(|(s, g)| s != surface || g != generation)
        {
            return Err("MP-11: stale native desktop".into());
        }
        Ok(())
    }
    pub(crate) fn takeover_desktop(
        &mut self,
        actor: EnvironmentActor,
        surface: &str,
        generation: &str,
    ) -> Result<TakeoverOutcome, String> {
        self.desktop_current(surface, generation)?;
        self.retire_lapsed_desktop_viewers(Instant::now());
        self.register(actor.clone())?;
        let effect = self
            .ledger
            .request_takeover(
                &actor.actor_id,
                InputTarget::Desktop,
                &self.actors,
                &self.tabs,
            )
            .map_err(|_| "MP-11: desktop takeover denied")?;
        for action in effect.cancellation_requested_action_ids {
            if let Some(cancel) = self.active.get(&action) {
                cancel.request_cancel();
            }
        }
        Ok(effect.outcome)
    }
    pub(crate) fn check_desktop_release(
        &self,
        actor: &str,
        surface: &str,
        generation: &str,
    ) -> Result<(), String> {
        self.desktop_current(surface, generation)?;
        if !self.desktop_owner_is(actor) {
            return Err("MP-11: only the desktop owner may release".into());
        }
        Ok(())
    }
    pub(crate) fn desktop_owner_is(&self, actor: &str) -> bool {
        self.ledger.owner(&InputTarget::Desktop) == Some(actor)
    }
    pub(crate) fn release_desktop(
        &mut self,
        actor: &str,
        surface: &str,
        generation: &str,
    ) -> Result<(), String> {
        self.desktop_current(surface, generation)?;
        self.ledger
            .release(actor, &InputTarget::Desktop)
            .map_err(|_| "MP-11: only the desktop owner may release".into())
    }
    pub(crate) fn takeover(
        &mut self,
        actor: EnvironmentActor,
        host_tab: &str,
        generation: u64,
    ) -> Result<TakeoverOutcome, String> {
        if generation != self.generation {
            return Err("MD-3: stale browser generation".into());
        }
        let tab = self.tab(host_tab)?;
        self.retire_lapsed_desktop_viewers(Instant::now());
        self.reclaim_idle_takeovers(Instant::now());
        self.register(actor.clone())?;
        self.adopt_orphaned_takeovers(&actor.actor_id);
        self.follow_person_takeover(&actor.actor_id, host_tab);
        self.takeover_input
            .insert(actor.actor_id.clone(), Instant::now());
        let effect = self
            .ledger
            .request_takeover(
                &actor.actor_id,
                InputTarget::BrowserTab(tab),
                &self.actors,
                &self.tabs,
            )
            .map_err(|_| "MD-3: browser takeover denied")?;
        for action in effect.cancellation_requested_action_ids {
            if let Some(cancellation) = self.active.get(&action) {
                cancellation.request_cancel();
            }
        }
        self.tab_activity.forget_source(host_tab);
        Ok(effect.outcome)
    }
    pub(crate) fn release(
        &mut self,
        actor_id: &str,
        host_tab: &str,
        generation: u64,
    ) -> Result<(), String> {
        if generation != self.generation {
            return Err("MD-3: stale browser generation".into());
        }
        let target = InputTarget::BrowserTab(self.tab(host_tab)?);
        self.reclaim_idle_takeovers(Instant::now());
        // The person is one party to the kernel: a viewer that reconnected or reloaded is a new actor, and
        // returns the control an earlier connection of the same person took. An agent never releases it.
        let agent_requests = self
            .actors
            .get(actor_id)
            .is_some_and(|actor| actor.kind == EnvironmentActorKind::Agent);
        let holder = self
            .ledger
            .ownership()
            .into_iter()
            .find(|owner| {
                owner.target == target && !agent_requests && self.is_human(&owner.actor_id)
            })
            .map_or(actor_id.to_string(), |owner| owner.actor_id);
        self.ledger
            .release(&holder, &target)
            .map_err(|_| "MD-3: only the input owner may release takeover")?;
        self.end_takeover_bookkeeping(&holder);
        Ok(())
    }
    fn is_human(&self, actor_id: &str) -> bool {
        self.actors
            .get(actor_id)
            .is_some_and(|actor| actor.kind == EnvironmentActorKind::Human)
    }
    /// MP-08/MP-11: a human actor's desktop video subscription (lease start).
    pub(crate) fn desktop_viewer(&mut self, actor_id: &str, subscription: &str, now: Instant) {
        if self.desktop_viewers.len() < 256 || self.desktop_viewers.contains_key(subscription) {
            self.desktop_viewers.insert(
                subscription.into(),
                (actor_id.into(), now + DESKTOP_VIEWER_LEASE),
            );
        }
    }
    /// Any display request on a tracked desktop subscription renews it.
    pub(crate) fn renew_desktop_viewer(&mut self, subscription: &str, now: Instant) {
        if let Some((_, expiry)) = self.desktop_viewers.get_mut(subscription) {
            *expiry = now + DESKTOP_VIEWER_LEASE;
        }
    }
    /// An unsubscribed desktop video ends its lease now.
    pub(crate) fn end_desktop_viewer(&mut self, subscription: &str, now: Instant) {
        if let Some((_, expiry)) = self.desktop_viewers.get_mut(subscription) {
            *expiry = now;
        }
    }
    /// MP-08/MP-11: retain this actor's ownership during the bounded display gap.
    pub(crate) fn suspend_desktop_viewer(
        &mut self,
        actor: &str,
        subscription: &str,
        now: Instant,
    ) -> Result<(), String> {
        if let Some((owner, expiry)) = self.desktop_viewers.get_mut(subscription) {
            if owner != actor {
                return Err("MP-11: foreign desktop viewer".into());
            }
            *expiry = (*expiry).min(now + DISPLAY_RESUME_GRACE);
        }
        Ok(())
    }
    /// Fresh protected host admission has already returned the replacement ID.
    /// Moving its lease must not create a moment with no live owner lease.
    pub(crate) fn resume_desktop_viewer(
        &mut self,
        actor: &str,
        old: &str,
        new: &str,
        now: Instant,
    ) -> Result<(), String> {
        if let Some((owner, expiry)) = self.desktop_viewers.get(old) {
            if owner != actor || *expiry <= now {
                return Err("MP-11: stale or foreign desktop viewer".into());
            }
        }
        self.desktop_viewers.remove(old);
        self.desktop_viewer(actor, new, now);
        Ok(())
    }
    /// MP-08/MP-11 (option b, 2026-10-09): relay clients have no gone
    /// notification. When a human actor's last desktop video subscription
    /// lapses or ends, its Desktop ownership retires (in-flight input is
    /// cancelled first) so a fresh client is never refused by a stale owner.
    pub(crate) fn retire_lapsed_desktop_viewers(&mut self, now: Instant) {
        let lapsed = self
            .desktop_viewers
            .values()
            .filter(|(_, expiry)| *expiry <= now)
            .map(|(actor, _)| actor.clone())
            .collect::<Vec<_>>();
        self.desktop_viewers.retain(|_, (_, expiry)| *expiry > now);
        for actor in lapsed {
            if self
                .desktop_viewers
                .values()
                .any(|(live, _)| live == &actor)
                || !self.desktop_owner_is(&actor)
            {
                continue;
            }
            for action in self
                .ledger
                .actions()
                .iter()
                .filter(|action| action.actor_id == actor)
            {
                if let Some(cancellation) = self.active.get(&action.action_id) {
                    cancellation.request_cancel();
                }
            }
            let _ = self.ledger.release(&actor, &InputTarget::Desktop);
        }
    }
    pub(crate) fn disconnect(&mut self, actor_id: &str) {
        self.desktop_viewers
            .retain(|_, (actor, _)| actor != actor_id);
        let keeps_takeover = self.is_human(actor_id) && self.ledger.owns_tab(actor_id);
        if keeps_takeover {
            self.ledger.release_actor_input_keeping_tabs(actor_id);
        } else {
            self.ledger.release_actor_input(actor_id);
        }
        for action in self
            .ledger
            .actions()
            .iter()
            .filter(|action| action.actor_id == actor_id)
        {
            if let Some(cancellation) = self.active.get(&action.action_id) {
                cancellation.request_cancel();
            }
        }
        if keeps_takeover {
            if let Some(remaining) = self.live_viewer_other_than(actor_id) {
                self.transfer_takeover(actor_id, &remaining);
                self.actors.remove(actor_id);
                self.seen.remove(actor_id);
            } else {
                self.orphaned_takeovers.insert(actor_id.into());
            }
        } else {
            self.takeover_input.remove(actor_id);
            self.actors.remove(actor_id);
            self.seen.remove(actor_id);
        }
        self.pointers.remove(actor_id);
        self.pointer_tabs.remove(actor_id);
    }

    /// An authenticated viewer of the person connected, possibly under a new identity: it continues
    /// the Tab takeovers whose viewer went away, until the person goes idle.
    pub(crate) fn viewer_connected(&mut self, actor: EnvironmentActor) {
        let actor_id = actor.actor_id.clone();
        if self.register(actor).is_ok() {
            self.adopt_orphaned_takeovers(&actor_id);
        }
    }

    /// A viewer of the person that is still connected: it continues a takeover whose own viewer left,
    /// so a takeover is only orphaned when no viewer of the person is left.
    fn live_viewer_other_than(&self, actor_id: &str) -> Option<String> {
        self.actors
            .iter()
            .find(|(id, actor)| {
                id.as_str() != actor_id
                    && actor.kind == EnvironmentActorKind::Human
                    && !self.orphaned_takeovers.contains(id.as_str())
            })
            .map(|(id, _)| id.clone())
    }

    /// A renewed authenticated connection no longer has an orphan deadline.
    fn reconnect(&mut self, actor_id: &str) {
        self.orphaned_takeovers.remove(actor_id);
    }

    /// The person is one party to the kernel: a viewer of theirs that connects after the one that took a
    /// Tab went away continues that takeover instead of finding the Tab fenced against itself.
    fn adopt_orphaned_takeovers(&mut self, actor_id: &str) {
        if !self.is_human(actor_id) {
            return;
        }
        self.reconnect(actor_id);
        let orphans = self
            .orphaned_takeovers
            .iter()
            .filter(|orphan| orphan.as_str() != actor_id)
            .cloned()
            .collect::<Vec<_>>();
        for orphan in orphans {
            self.transfer_takeover(&orphan, actor_id);
            self.end_takeover_bookkeeping(&orphan);
        }
    }
    /// The takeover belongs to the person, not to the window that took it: input from another viewer
    /// of the person on a Tab the person holds continues that takeover there.
    fn follow_person_takeover(&mut self, actor_id: &str, host_tab: &str) {
        if !self.is_human(actor_id) {
            return;
        }
        let Ok(tab) = self.tab(host_tab) else {
            return;
        };
        let target = InputTarget::BrowserTab(tab);
        let holder = self
            .ledger
            .ownership()
            .into_iter()
            .map(|owner| (owner.target, owner.actor_id))
            .chain(
                self.ledger
                    .pending_takeovers()
                    .into_iter()
                    .map(|pending| (pending.target, pending.human_actor_id)),
            )
            .find(|(held, owner)| *held == target && owner != actor_id && self.is_human(owner));
        if let Some((_, holder)) = holder {
            self.transfer_takeover(&holder, actor_id);
        }
    }
    /// A replacement viewer inherits the person's input time; connecting never renews it.
    fn transfer_takeover(&mut self, from: &str, to: &str) {
        self.ledger.transfer_tab_ownership(from, to);
        if let Some(last) = self.takeover_input.remove(from) {
            self.takeover_input
                .entry(to.into())
                .and_modify(|current| *current = (*current).max(last))
                .or_insert(last);
        }
    }
    #[cfg(test)]
    pub(crate) fn elapse_idle_takeovers(&mut self) {
        for last in self.takeover_input.values_mut() {
            *last = Instant::now() - TAKEOVER_IDLE_TIMEOUT;
        }
    }
    /// Input from the person (pointer, keyboard, scroll, navigation) is the only thing that keeps a takeover.
    /// Frames, state reads and reconnects of their viewer never reach this.
    fn note_person_input(&mut self, actor_id: &str) {
        if self.is_human(actor_id) && self.ledger.owns_tab(actor_id) {
            self.takeover_input.insert(actor_id.into(), Instant::now());
        }
    }
    /// Returns control to the agent for every Tab whose person has sent no input for `TAKEOVER_IDLE_TIMEOUT`,
    /// with or without a viewer of theirs connected. True when a takeover ended.
    pub(crate) fn reclaim_idle_takeovers(&mut self, now: Instant) -> bool {
        let idle = self
            .takeover_input
            .iter()
            .filter(|(_, last)| now.saturating_duration_since(**last) >= TAKEOVER_IDLE_TIMEOUT)
            .map(|(actor, _)| actor.clone())
            .collect::<Vec<_>>();
        let mut ended = false;
        for actor in idle {
            ended |= self.ledger.has_tab_takeover(&actor);
            self.ledger.release_tab_input(&actor);
            self.end_takeover_bookkeeping(&actor);
        }
        ended
    }
    /// Keep the idle clock until the person's last held or pending Tab takeover ends.
    fn end_takeover_bookkeeping(&mut self, actor_id: &str) {
        if self.ledger.has_tab_takeover(actor_id) {
            return;
        }
        self.takeover_input.remove(actor_id);
        if self.orphaned_takeovers.remove(actor_id) {
            self.actors.remove(actor_id);
            self.seen.remove(actor_id);
        }
    }

    pub(crate) fn opened_tab(&mut self, actor: EnvironmentActor, tab: &str) {
        self.tab_activity.opened(actor, tab);
    }
    pub(crate) fn set_attention(&mut self, tabs: std::collections::BTreeSet<String>) {
        self.tab_activity.set_attention(tabs);
    }
    pub(crate) fn agent_tab_groups(&self) -> Value {
        self.tab_activity.agent_groups()
    }
    pub(crate) fn project_tabs(&mut self, state: &mut Value) {
        self.reclaim_idle_takeovers(Instant::now());
        self.tab_activity.project(state);
        let held = self.person_held_tabs();
        if let Some(tabs) = state.get_mut("tabs").and_then(Value::as_array_mut) {
            for tab in tabs {
                let taken = tab["tab_id"].as_str().is_some_and(|id| held.contains(id));
                tab["taken_over"] = taken.into();
            }
        }
    }
    /// The Tabs a person drives after Take over, by controller Tab id: a viewer that reloads reads this back.
    fn person_held_tabs(&self) -> std::collections::BTreeSet<String> {
        self.ledger
            .ownership()
            .into_iter()
            .filter_map(|owner| match owner.target {
                InputTarget::BrowserTab(tab) if self.is_human(&owner.actor_id) => {
                    Some(self.host_tabs.get(&tab).cloned().unwrap_or(tab))
                }
                _ => None,
            })
            .collect()
    }
    pub(crate) fn snapshot(&self) -> Value {
        let translate = |target: &mut InputTarget| {
            if let InputTarget::BrowserTab(tab) = target {
                if let Some(host_tab) = self.host_tabs.get(tab) {
                    *tab = host_tab.clone();
                }
            }
        };
        let mut ownership = self.ledger.ownership();
        for owner in &mut ownership {
            translate(&mut owner.target);
        }
        let mut pending = self.ledger.pending_takeovers();
        for takeover in &mut pending {
            translate(&mut takeover.target);
        }
        let mut actions = self.ledger.actions();
        for action in &mut actions {
            for target in &mut action.targets {
                translate(target);
            }
        }
        serde_json::json!({"agent_activity":self.tab_activity.activity(),"generation":self.generation,"actors":self.actors.values().collect::<Vec<_>>(),
            "pointers":self.pointers.values().collect::<Vec<_>>(),"pointer_tabs":self.pointer_tabs,"input_ownership":ownership,
            "pending_input_takeovers":pending,"actions":actions})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ready() -> KernelBrowserActors {
        let mut model = KernelBrowserActors::default();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"},{"tab_id":"host-tab-b","document_id":"b"}]})).unwrap();
        model
    }
    fn human(id: &str) -> EnvironmentActor {
        EnvironmentActor::new(id, EnvironmentActorKind::Human, "Human")
    }
    fn agent() -> EnvironmentActor {
        EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Agent")
    }
    fn input(tab: &str) -> Value {
        json!({"op":"input","tab_id":tab,"generation":1,"input":{"kind":"text","text":"synthetic-private-value"}})
    }
    #[test]
    fn mp10_takeover_is_listed_on_the_tab_for_a_viewer_that_reloads() {
        let mut model = ready();
        let listed = |model: &mut KernelBrowserActors| {
            let mut state =
                json!({"generation":1,"tabs":[{"tab_id":"host-tab-a"},{"tab_id":"host-tab-b"}]});
            model.project_tabs(&mut state);
            (
                state["tabs"][0]["taken_over"].clone(),
                state["tabs"][1]["taken_over"].clone(),
            )
        };
        assert_eq!(listed(&mut model), (json!(false), json!(false)));
        model
            .takeover(human("terminal:a"), "host-tab-a", 1)
            .unwrap();
        assert_eq!(
            listed(&mut model),
            (json!(true), json!(false)),
            "the person holds Tab a"
        );
        model.release("terminal:a", "host-tab-a", 1).unwrap();
        assert_eq!(listed(&mut model), (json!(false), json!(false)));
        model
            .takeover(human("terminal:a"), "host-tab-a", 1)
            .unwrap();
        model.release("terminal:reloaded", "host-tab-a", 1).unwrap();
        assert_eq!(
            listed(&mut model),
            (json!(false), json!(false)),
            "the person's reloaded viewer returns the control its earlier connection took"
        );
        let (_, _) = model.begin(agent(), &input("host-tab-b")).unwrap();
        assert_eq!(
            listed(&mut model),
            (json!(false), json!(false)),
            "an agent acting is not a person's takeover"
        );
    }
    #[test]
    fn a_second_viewer_of_the_person_keeps_typing_while_the_first_holds_the_takeover() {
        let mut model = ready();
        model
            .takeover(human("terminal:first"), "host-tab-a", 1)
            .unwrap();
        let (action, _) = model
            .begin(human("terminal:second"), &input("host-tab-a"))
            .expect("the person's other viewer is not fenced by the person's own takeover");
        model.finish(&action, EnvironmentActionTerminal::Completed);
        assert!(
            model.begin(agent(), &input("host-tab-a")).is_err(),
            "the agent stays fenced while the person holds the Tab"
        );
        let (action, _) = model
            .begin(human("terminal:first"), &input("host-tab-a"))
            .expect("the first viewer types again");
        model.finish(&action, EnvironmentActionTerminal::Completed);
        model.release("terminal:second", "host-tab-a", 1).unwrap();
        let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
    }
    #[test]
    fn a_second_viewer_can_take_over_a_tab_the_first_viewer_holds() {
        let mut model = ready();
        model
            .takeover(human("terminal:first"), "host-tab-a", 1)
            .unwrap();
        model
            .takeover(human("terminal:second"), "host-tab-a", 1)
            .expect("the takeover follows the person to the viewer that asks");
        model.release("terminal:first", "host-tab-a", 1).unwrap();
        assert!(model.begin(agent(), &input("host-tab-a")).is_ok());
    }
    #[test]
    fn mp10_takeover_survives_the_viewer_disconnect_and_follows_the_person_to_a_new_connection() {
        let mut model = ready();
        let held = |model: &mut KernelBrowserActors| {
            let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-tab-a"}]});
            model.project_tabs(&mut state);
            state["tabs"][0]["taken_over"].clone()
        };
        model
            .takeover(human("terminal:old"), "host-tab-a", 1)
            .unwrap();
        model.disconnect("terminal:old");
        assert_eq!(
            held(&mut model),
            json!(true),
            "the person still holds the Tab after the viewer went away"
        );
        assert!(
            model.begin(agent(), &input("host-tab-a")).is_err(),
            "the agent stays fenced while the person's viewer is away"
        );
        let (action, _) = model
            .begin(human("terminal:new"), &input("host-tab-a"))
            .expect("the person's reconnected viewer acts on the Tab it still holds");
        model.finish(&action, EnvironmentActionTerminal::Completed);
        assert_eq!(held(&mut model), json!(true));
        model.release("terminal:new", "host-tab-a", 1).unwrap();
        assert_eq!(held(&mut model), json!(false));
        let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
    }
    #[test]
    fn mp10_orphaned_takeover_ends_by_return_control_or_when_the_person_goes_idle() {
        let mut model = ready();
        model
            .takeover(human("terminal:old"), "host-tab-a", 1)
            .unwrap();
        model.disconnect("terminal:old");
        assert!(!model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT / 2));
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        assert!(model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT));
        assert!(model.snapshot()["input_ownership"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!model.snapshot()["actors"]
            .to_string()
            .contains("terminal:old"));
        let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);

        model
            .takeover(human("terminal:old"), "host-tab-a", 1)
            .unwrap();
        model.disconnect("terminal:old");
        model.release("terminal:other", "host-tab-a", 1).unwrap();
        let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
    }
    #[test]
    fn mp10_same_actor_input_or_takeover_renews_its_idle_clock() {
        for request_takeover in [false, true] {
            let mut model = ready();
            let person = human("terminal:viewer");
            model.takeover(person.clone(), "host-tab-a", 1).unwrap();
            model.disconnect(&person.actor_id);
            let last = Instant::now() - TAKEOVER_IDLE_TIMEOUT / 2;
            model.takeover_input.insert(person.actor_id.clone(), last);
            let deadline = last + TAKEOVER_IDLE_TIMEOUT;
            if request_takeover {
                model.takeover(person.clone(), "host-tab-a", 1).unwrap();
            } else {
                let (action, _) = model.begin(person.clone(), &input("host-tab-a")).unwrap();
                model.finish(&action, EnvironmentActionTerminal::Completed);
            }
            model.reclaim_idle_takeovers(deadline + Duration::from_secs(1));
            assert!(
                model.begin(agent(), &input("host-tab-a")).is_err(),
                "a live viewer must keep the agent fenced past its old orphan deadline"
            );
            let (action, _) = model.begin(person.clone(), &input("host-tab-a")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
            model.release(&person.actor_id, "host-tab-a", 1).unwrap();
        }
    }
    #[test]
    fn mp10_returning_one_orphaned_tab_preserves_the_other_tabs_idle_clock() {
        for resume_input in [false, true] {
            let mut model = ready();
            let person = human("terminal:old");
            model.takeover(person.clone(), "host-tab-a", 1).unwrap();
            model.takeover(person.clone(), "host-tab-b", 1).unwrap();
            model.disconnect(&person.actor_id);
            let last = Instant::now() - TAKEOVER_IDLE_TIMEOUT / 2;
            model.takeover_input.insert(person.actor_id.clone(), last);
            let deadline = last + TAKEOVER_IDLE_TIMEOUT;
            model.release("terminal:reloaded", "host-tab-a", 1).unwrap();
            assert_eq!(
                model.takeover_input.get(&person.actor_id),
                Some(&last),
                "returning one Tab must leave the remaining Tab recoverable"
            );
            let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
            assert!(model.begin(agent(), &input("host-tab-b")).is_err());
            if resume_input {
                let (action, _) = model
                    .begin(human("terminal:reloaded"), &input("host-tab-b"))
                    .unwrap();
                model.finish(&action, EnvironmentActionTerminal::Completed);
                model.reclaim_idle_takeovers(deadline + Duration::from_secs(1));
                assert!(model.begin(agent(), &input("host-tab-b")).is_err());
                model.release("terminal:reloaded", "host-tab-b", 1).unwrap();
            } else {
                model.reclaim_idle_takeovers(deadline + Duration::from_secs(1));
            }
            let (action, _) = model.begin(agent(), &input("host-tab-b")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
        }
    }
    #[test]
    fn mp10_an_agent_acts_again_once_the_idle_takeover_has_lapsed() {
        let mut model = ready();
        model
            .takeover(human("terminal:old"), "host-tab-a", 1)
            .unwrap();
        model.disconnect("terminal:old");
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        model.takeover_input.insert(
            "terminal:old".into(),
            Instant::now() - TAKEOVER_IDLE_TIMEOUT,
        );
        let (action, _) = model
            .begin(agent(), &input("host-tab-a"))
            .expect("the lapsed takeover no longer fences the agent");
        model.finish(&action, EnvironmentActionTerminal::Completed);
    }
    #[test]
    fn mp10_takeover_returns_after_three_idle_minutes_even_with_the_viewer_connected() {
        let mut model = ready();
        model
            .takeover(human("terminal:viewer"), "host-tab-a", 1)
            .unwrap();
        // The viewer stays connected and busy with frames and state reads; none of that reaches the ledger.
        let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-tab-a"}]});
        model.project_tabs(&mut state);
        assert_eq!(state["tabs"][0]["taken_over"], json!(true));
        assert!(!model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT / 2));
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        assert!(model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT));
        let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-tab-a"}]});
        model.project_tabs(&mut state);
        assert_eq!(state["tabs"][0]["taken_over"], json!(false));
        assert!(
            model.snapshot()["actors"]
                .to_string()
                .contains("terminal:viewer"),
            "the connected viewer stays registered"
        );
        let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        assert!(
            model
                .takeover(human("terminal:viewer"), "host-tab-a", 1)
                .is_ok(),
            "the person can take over again"
        );
    }
    #[test]
    fn mp10_pending_takeover_keeps_its_idle_clock_after_another_tab_is_returned() {
        for finish_before_timeout in [true, false] {
            let mut model = ready();
            let person = human("terminal:viewer");
            model.takeover(person.clone(), "host-tab-a", 1).unwrap();
            let (action, cancellation) = model.begin(agent(), &input("host-tab-b")).unwrap();
            assert_eq!(
                model.takeover(person, "host-tab-b", 1).unwrap(),
                TakeoverOutcome::CancellationRequired {
                    action_ids: vec![action.clone()]
                }
            );
            assert!(cancellation.requested());
            model.release("terminal:viewer", "host-tab-a", 1).unwrap();
            if finish_before_timeout {
                model.finish(&action, EnvironmentActionTerminal::Cancelled);
            }
            assert!(
                model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT),
                "the pending takeover expires whether cancellation has finished or not"
            );
            if !finish_before_timeout {
                model.finish(&action, EnvironmentActionTerminal::Cancelled);
            }
            assert!(model.snapshot()["pending_input_takeovers"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(model.snapshot()["input_ownership"]
                .as_array()
                .unwrap()
                .is_empty());
            let (action, _) = model.begin(agent(), &input("host-tab-b")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
        }
    }
    #[test]
    fn mp10_replacement_viewers_keep_the_original_idle_clock() {
        for already_connected in [false, true] {
            let mut model = ready();
            model
                .takeover(human("terminal:old"), "host-tab-a", 1)
                .unwrap();
            let last = Instant::now() - TAKEOVER_IDLE_TIMEOUT + Duration::from_secs(5);
            model.takeover_input.insert("terminal:old".into(), last);
            if already_connected {
                model.viewer_connected(human("terminal:new"));
            }
            model.disconnect("terminal:old");
            if !already_connected {
                model.viewer_connected(human("terminal:new"));
            }
            assert_eq!(model.takeover_input.get("terminal:new"), Some(&last));
            assert!(model.begin(agent(), &input("host-tab-a")).is_err());
            assert!(model.reclaim_idle_takeovers(last + TAKEOVER_IDLE_TIMEOUT));
            let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
        }
    }
    #[test]
    fn mp10_the_persons_input_restarts_the_idle_clock() {
        let mut model = ready();
        model
            .takeover(human("terminal:viewer"), "host-tab-a", 1)
            .unwrap();
        model.takeover_input.insert(
            "terminal:viewer".into(),
            Instant::now() - TAKEOVER_IDLE_TIMEOUT + Duration::from_secs(5),
        );
        let (action, _) = model
            .begin(human("terminal:viewer"), &input("host-tab-a"))
            .unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        assert!(
            !model.reclaim_idle_takeovers(Instant::now() + Duration::from_secs(60)),
            "input a minute ago keeps the takeover"
        );
        assert!(model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT));
    }
    #[test]
    fn mp10_the_agents_refusal_names_the_wait_it_should_declare() {
        let mut model = ready();
        model
            .takeover(human("terminal:viewer"), "host-tab-a", 1)
            .unwrap();
        let refusal = model
            .begin(agent(), &input("host-tab-a"))
            .err()
            .expect("the agent is fenced");
        assert!(refusal.contains("human_takeover") && refusal.contains("chariox.events.blocked"));
    }
    #[test]
    fn mp08_chrome_input_tracks_home_commit_without_retargeting_or_replay() {
        let model = ready();
        let mut params =
            json!({"op":"input","generation":1,"tab_id":"host-tab-a","document_id":"old"});
        model.refresh_chrome_input_document(&mut params).unwrap();
        assert_eq!(params["document_id"], "d");
        assert_eq!(params["tab_id"], "host-tab-a");
        params["generation"] = 0.into();
        assert!(model.refresh_chrome_input_document(&mut params).is_err());
        params["generation"] = 1.into();
        params["tab_id"] = "unknown".into();
        assert!(model.refresh_chrome_input_document(&mut params).is_err());
    }
    #[test]
    fn mp11_input_cache_requires_exact_observed_generation_target_and_document() {
        let mut model = ready();
        let current = json!({"generation":1,"tab_id":"host-tab-a","document_id":"d"});
        assert!(model.input_is_current(&current));
        for stale in [
            json!({"generation":0,"tab_id":"host-tab-a","document_id":"d"}),
            json!({"generation":1,"tab_id":"missing","document_id":"d"}),
            json!({"generation":1,"tab_id":"host-tab-a","document_id":"old"}),
            json!({"generation":1,"tab_id":"host-tab-a"}),
        ] {
            assert!(!model.input_is_current(&stale));
        }
        model
            .reconcile(
                &json!({"generation":2,"tabs":[{"tab_id":"host-tab-a","document_id":"new"}]}),
            )
            .unwrap();
        assert!(!model.input_is_current(&current));
        assert!(model
            .input_is_current(&json!({"generation":2,"tab_id":"host-tab-a","document_id":"new"})));
    }
    #[test]
    fn mp08_pointer_projection_uses_the_bounded_host_viewport() {
        let mut model = ready();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-tab-a","document_id":"d"}],"viewport":{"css_width":1920,"css_height":1080}})).unwrap();
        let (action, _) = model.begin(human("terminal:1080p"), &json!({"op":"input","tab_id":"host-tab-a","generation":1,"input":{"kind":"click","x":1919,"y":1079}})).unwrap();
        assert_eq!(model.snapshot()["pointers"][0]["x"], 1919);
        assert_eq!(model.snapshot()["pointers"][0]["y"], 1079);
        model.finish(&action, EnvironmentActionTerminal::Completed);
    }

    #[test]
    fn md3_a_full_model_retires_the_longest_idle_actor_and_keeps_busy_ones() {
        // F-5 (b3, browserfix r5): each relay viewer page is its own actor and never disconnects;
        // the 65th viewer was refused every browser request.
        let mut model = ready();
        model
            .takeover(human("terminal:busy"), "host-tab-a", 1)
            .unwrap();
        for viewer in 0..63 {
            let id = format!("terminal:{viewer}");
            let (action, _) = model.begin(human(&id), &input("host-tab-b")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
        }
        let (action, _) = model
            .begin(human("terminal:new"), &input("host-tab-b"))
            .unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        let actors = model.snapshot()["actors"].to_string();
        assert!(
            !actors.contains("\"terminal:0\""),
            "the longest idle viewer is retired"
        );
        assert!(actors.contains("terminal:busy") && actors.contains("terminal:new"));
    }

    #[test]
    fn md3_terminal_disconnect_releases_input_and_reclaims_actor_capacity() {
        let mut model = ready();
        for connection in 0..128 {
            let id = format!("terminal:{connection}");
            let (action, cancellation) = model.begin(human(&id), &input("host-tab-a")).unwrap();
            model.disconnect(&id);
            assert!(cancellation.requested());
            model.finish(&action, EnvironmentActionTerminal::Cancelled);
            model.takeover(human(&id), "host-tab-a", 1).unwrap();
            model.disconnect(&id);
            model.reclaim_idle_takeovers(Instant::now() + TAKEOVER_IDLE_TIMEOUT);
            assert!(model.snapshot()["input_ownership"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(model.snapshot()["actors"].as_array().unwrap().is_empty());
        }
        let (action, cancellation) = model.begin(agent(), &input("host-tab-a")).unwrap();
        model
            .takeover(human("departing-human"), "host-tab-a", 1)
            .unwrap();
        assert!(cancellation.requested());
        model.disconnect("departing-human");
        model.finish(&action, EnvironmentActionTerminal::Cancelled);
        assert!(model.snapshot()["pending_input_takeovers"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(model.snapshot()["input_ownership"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn takeover_cancels_running_input_and_holds_ownership_until_release() {
        let mut model = ready();
        let (action, cancellation) = model.begin(agent(), &input("host-tab-a")).unwrap();
        assert!(matches!(
            model.takeover(human("user:a"), "host-tab-a", 1).unwrap(),
            TakeoverOutcome::CancellationRequired { .. }
        ));
        assert!(cancellation.requested());
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        model.finish(&action, EnvironmentActionTerminal::Cancelled);
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        assert!(model.release("agent:a", "host-tab-a", 1).is_err());
        assert!(!model
            .snapshot()
            .to_string()
            .contains("synthetic-private-value"));
        model.release("user:a", "host-tab-a", 1).unwrap();
        assert!(model.begin(agent(), &input("host-tab-a")).is_ok());
    }
    #[test]
    fn takeover_is_tab_scoped_human_only_and_generation_bound() {
        let mut model = ready();
        assert!(model.takeover(agent(), "host-tab-a", 1).is_err());
        assert!(model.takeover(human("user:a"), "host-tab-a", 2).is_err());
        model.takeover(human("user:a"), "host-tab-a", 1).unwrap();
        assert!(model.begin(agent(), &input("host-tab-a")).is_err());
        assert!(model.begin(agent(), &input("host-tab-b")).is_ok());
    }
    #[test]
    fn browser_recovery_retains_human_takeover_and_invalidates_running_actions() {
        let mut model = ready();
        model.takeover(human("user:a"), "host-tab-a", 1).unwrap();
        let (_, cancellation) = model.begin(agent(), &input("host-tab-b")).unwrap();
        model
            .reconcile(
                &json!({"generation":2,"tabs":[{"tab_id":"host-tab-a","document_id":"new"}]}),
            )
            .unwrap();
        assert!(cancellation.requested());
        assert!(model.release("user:a", "host-tab-a", 1).is_err());
        assert_eq!(model.snapshot()["input_ownership"][0]["actor_id"], "user:a");
        model.release("user:a", "host-tab-a", 2).unwrap();
    }
    #[test]
    fn transient_actions_remain_bounded_and_do_not_retain_input_payloads() {
        let mut model = ready();
        for _ in 0..600 {
            let (action, _) = model.begin(agent(), &input("host-tab-a")).unwrap();
            model.finish(&action, EnvironmentActionTerminal::Completed);
        }
        assert_eq!(model.ledger.actions().len(), 256);
        assert_eq!(model.ledger.action_history(None, 1000).actions.len(), 256);
        assert!(!model
            .snapshot()
            .to_string()
            .contains("synthetic-private-value"));
    }

    #[test]
    fn an_agent_group_names_the_tabs_a_person_is_asked_to_act_in() {
        let mut model = ready();
        model.opened_tab(agent(), "host-tab-a");
        model.opened_tab(agent(), "host-tab-b");
        assert_eq!(
            model
                .agent_tab_groups()
                .as_array()
                .unwrap()
                .iter()
                .map(|g| g["needs_person"].clone())
                .collect::<Vec<_>>(),
            vec![json!(false), json!(false)]
        );
        model.set_attention(["host-tab-b".to_string()].into_iter().collect());
        let groups = model.agent_tab_groups();
        let needs = |tab: &str| {
            groups
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["tab_id"] == tab)
                .unwrap()["needs_person"]
                .clone()
        };
        assert_eq!(needs("host-tab-a"), json!(false));
        assert_eq!(needs("host-tab-b"), json!(true));
    }
}

#[cfg(test)]
mod visible_tab_tests {
    use super::*;
    use serde_json::json;
    fn actor(id: &str) -> EnvironmentActor {
        EnvironmentActor::new(id, EnvironmentActorKind::Agent, id)
    }
    fn completed(model: &mut KernelBrowserActors, id: &str, tab: &str) -> String {
        let (action, _) = model
            .begin(
                actor(id),
                &json!({"op":"input","tab_id":tab,"generation":1}),
            )
            .unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        action
    }
    #[test]
    fn mp08_native_human_popup_after_completed_agent_input_is_unattributed() {
        let mut model = KernelBrowserActors::default();
        let state = json!({"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"}]});
        model.reconcile(&state).unwrap();
        completed(&mut model, "agent:a", "source");
        let activity = model.tab_activity.activity();
        // Native input never enters begin/takeover. An opener is not actor evidence.
        let mut later = json!({"generation":1,"_tab_openers":{"human-popup":"source"},"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"human-popup","document_id":"popup"}]});
        model.reconcile(&later).unwrap();
        model.project_tabs(&mut later);
        assert!(later["tabs"][1]["opened_by"].is_null());
        assert_eq!(later["agent_activity"], activity);
    }
    #[test]
    fn mp08_captured_popup_survives_source_document_change_or_close() {
        for changed in [true, false] {
            let mut model = KernelBrowserActors::default();
            model
                .reconcile(
                    &json!({"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"}]}),
                )
                .unwrap();
            let action = completed(&mut model, "agent:a", "source");
            let tabs = if changed {
                json!([{"tab_id":"source","document_id":"new"}])
            } else {
                json!([])
            };
            model
                .reconcile(&json!({"generation":1,"tabs":tabs}))
                .unwrap();
            let mut later = json!({"generation":1,"_tab_creation_actions":{"popup":action},"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"popup","document_id":"popup"}]});
            model.reconcile(&later).unwrap();
            model.project_tabs(&mut later);
            assert_eq!(later["tabs"][1]["opened_by"]["actor_id"], "agent:a");
            assert_eq!(later["agent_activity"]["tab_id"], "popup");
        }
    }
    #[test]
    fn mp08_delayed_popup_uses_its_creation_action_after_another_actor_acts() {
        let mut model = KernelBrowserActors::default();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"other","document_id":"other"}]})).unwrap();
        let a = completed(&mut model, "agent:a", "source");
        // Even a later mutation on the same unchanged document cannot relabel it.
        completed(&mut model, "agent:c", "source");
        let b = completed(&mut model, "agent:b", "other");
        let mut later = json!({"generation":1,"_tab_creation_actions":{"popup":a,"other-popup":b},"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"other","document_id":"other"},{"tab_id":"popup","document_id":"popup"},{"tab_id":"other-popup","document_id":"other-popup"},{"tab_id":"native","document_id":"native"}]});
        model.reconcile(&later).unwrap();
        model.project_tabs(&mut later);
        assert_eq!(later["tabs"][2]["opened_by"]["actor_id"], "agent:a");
        assert_eq!(later["tabs"][3]["opened_by"]["actor_id"], "agent:b");
        assert!(later["tabs"][4]["opened_by"].is_null());
    }
    #[test]
    fn mp08_native_tab_during_agent_mutation_has_no_agent_opener() {
        let mut model = KernelBrowserActors::default();
        model
            .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"}]}))
            .unwrap();
        let (action, _) = model
            .begin(
                actor("agent:a"),
                &json!({"op":"navigate","tab_id":"host-a","generation":1}),
            )
            .unwrap();
        let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"next"},{"tab_id":"native-human","document_id":"human"}]});
        model.reconcile(&state).unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        model.project_tabs(&mut state);
        assert!(state["tabs"][1]["opened_by"].is_null());
        assert_eq!(state["agent_activity"]["tab_id"], "host-a");
    }
    #[test]
    fn mp08_only_popup_with_admitted_creation_action_gets_agent_attribution() {
        let mut model = KernelBrowserActors::default();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"other","document_id":"other"}]})).unwrap();
        let action = completed(&mut model, "agent:a", "host-a");
        let mut state = json!({"generation":1,"_tab_creation_actions":{"popup":action,"other-popup":"unadmitted"},"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"other","document_id":"other"},{"tab_id":"popup","document_id":"popup"},{"tab_id":"native-human","document_id":"human"},{"tab_id":"other-popup","document_id":"other-popup"}]});
        model.reconcile(&state).unwrap();
        model.project_tabs(&mut state);
        assert_eq!(state["tabs"][2]["opened_by"]["actor_id"], "agent:a");
        assert!(state["tabs"][3]["opened_by"].is_null());
        assert!(state["tabs"][4]["opened_by"].is_null());
        assert_eq!(state["agent_activity"]["tab_id"], "popup");
        assert_eq!(state["agent_activity"]["sequence"], 2);
    }
    #[test]
    fn mp08_browser_restart_does_not_attribute_restored_popups_to_mutation() {
        let mut model = KernelBrowserActors::default();
        model
            .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"}]}))
            .unwrap();
        let action = completed(&mut model, "agent:a", "host-a");
        model.opened_tab(actor("agent:a"), "host-a");
        let mut state = json!({"generation":2,"_tab_creation_actions":{"restored":action},"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"restored","document_id":"restored"}]});
        model.reconcile(&state).unwrap();
        model.project_tabs(&mut state);
        assert_eq!(state["tabs"][0]["opened_by"]["actor_id"], "agent:a");
        assert!(state["tabs"][1]["opened_by"].is_null());
        assert!(state["agent_activity"].is_null());
    }
    #[test]
    fn mp08_failed_or_cancelled_action_cannot_attribute_later_creation() {
        for terminal in [
            EnvironmentActionTerminal::Failed,
            EnvironmentActionTerminal::Cancelled,
        ] {
            let mut model = KernelBrowserActors::default();
            model
                .reconcile(
                    &json!({"generation":1,"tabs":[{"tab_id":"source","document_id":"doc"}]}),
                )
                .unwrap();
            let (action, _) = model
                .begin(
                    actor("agent:a"),
                    &json!({"op":"input","tab_id":"source","generation":1}),
                )
                .unwrap();
            model.finish(&action, terminal);
            let mut later = json!({"generation":1,"_tab_creation_actions":{"popup":action},"tabs":[{"tab_id":"source","document_id":"doc"},{"tab_id":"popup","document_id":"popup"}]});
            model.reconcile(&later).unwrap();
            model.project_tabs(&mut later);
            assert!(later["tabs"][1]["opened_by"].is_null());
        }
    }
    #[test]
    fn mp08_agent_activity_is_visible_without_changing_input_ownership() {
        let mut model = KernelBrowserActors::default();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"d","url":"https://developer.mozilla.org","title":"MDN"}]})).unwrap();
        let agent = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Mara");
        let (id, _) = model.begin(agent, &json!({"op":"input","tab_id":"host-a","generation":1,"input":{"kind":"text","text":"public search"}})).unwrap();
        let state = model.snapshot();
        assert_eq!(state["agent_activity"]["tab_id"], "host-a");
        assert_eq!(state["agent_activity"]["actor"]["display_label"], "Mara");
        assert_eq!(state["agent_activity"]["sequence"], 1);
        model.finish(&id, EnvironmentActionTerminal::Completed);
        let before = model.snapshot()["input_ownership"].clone();
        let _view = model.snapshot();
        assert_eq!(model.snapshot()["input_ownership"], before);
        assert!(!model.snapshot().to_string().contains("public search"));
    }
}
