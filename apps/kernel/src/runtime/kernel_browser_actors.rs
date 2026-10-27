//! MD-3: sessionless attachment of the shared Room actor/input model.
use super::browser_controller_process::BrowserCancellation;
use crate::session::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// MP-08/MP-11: a human actor's Desktop ownership lasts while one of its
/// desktop video subscriptions is live; the host display lease is 60 s.
pub(crate) const DESKTOP_VIEWER_LEASE: Duration = Duration::from_secs(60);
pub(crate) const DISPLAY_RESUME_GRACE: Duration = Duration::from_secs(5);

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
    ledger: EnvironmentActionLedger,
    active: BTreeMap<String, Arc<BrowserCancellation>>,
    pointers: BTreeMap<String, EnvironmentPointer>,
    pointer_tabs: BTreeMap<String, String>,
    host_tabs: BTreeMap<String, String>,
    /// Desktop video subscription -> (human actor, lease expiry).
    desktop_viewers: BTreeMap<String, (String, Instant)>,
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
            ledger: EnvironmentActionLedger::new(256, 0),
            active: BTreeMap::new(),
            pointers: BTreeMap::new(),
            pointer_tabs: BTreeMap::new(),
            host_tabs: BTreeMap::new(),
            desktop_viewers: BTreeMap::new(),
        }
    }
}
impl KernelBrowserActors {
    pub(crate) fn reconcile(&mut self, state: &Value) -> Result<(), String> {
        self.reconcile_inventory(state, &state["_tab_creation_actions"])
    }
    pub(crate) fn reconcile_inventory(
        &mut self,
        state: &Value,
        creation_actions: &Value,
    ) -> Result<(), String> {
        let Some(generation) = state["generation"].as_u64() else {
            return Ok(());
        };
        let Some(tabs) = state["tabs"].as_array() else {
            return Ok(());
        };
        // MP-08/MP-10/MP-11: concurrent controller reads finish out of order; the
        // controller numbers its observations within a generation.
        let observation = state["_observation"].as_u64();
        if generation < self.generation
            || generation == self.generation
                && observation.is_some_and(|observation| observation <= self.observation)
        {
            return Ok(());
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
        for (tab, actor) in discovered {
            self.opened_tab(actor, &tab);
        }
        Ok(())
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
            return Err("MD-3: browser actor limit reached".into());
        }
        if self
            .actors
            .get(&actor.actor_id)
            .is_some_and(|old| old.kind != actor.kind)
        {
            return Err("MD-3: browser actor kind conflict".into());
        }
        self.actors.insert(actor.actor_id.clone(), actor);
        Ok(())
    }
    /// MP-08/MP-11: dialog answers cannot reserve the lane held by the action
    /// that opened them, but still honor both active and pending human takeover.
    pub(crate) fn check_dialog_takeover(&self, host_tab: &str) -> Result<(), String> {
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
            return Err("MD-3: human owns browser input; release takeover first".into());
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
        self.register(actor.clone())?;
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
            ActionAdmission::RejectedTakeover { .. } => {
                return Err("MD-3: human owns browser input; release takeover first".into())
            }
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
        self.register(actor.clone())?;
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
        let tab = self.tab(host_tab)?;
        self.ledger
            .release(actor_id, &InputTarget::BrowserTab(tab))
            .map_err(|_| "MD-3: only the input owner may release takeover".into())
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
        self.ledger.release_actor_input(actor_id);
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
        self.actors.remove(actor_id);
        self.pointers.remove(actor_id);
        self.pointer_tabs.remove(actor_id);
    }

    pub(crate) fn opened_tab(&mut self, actor: EnvironmentActor, tab: &str) {
        self.tab_activity.opened(actor, tab);
    }
    pub(crate) fn agent_tab_groups(&self) -> Value {
        self.tab_activity.agent_groups()
    }
    pub(crate) fn project_tabs(&self, state: &mut Value) {
        self.tab_activity.project(state);
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
        assert!(model.release("user:b", "host-tab-a", 1).is_err());
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
