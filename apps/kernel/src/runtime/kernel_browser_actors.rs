//! MD-3: sessionless attachment of the shared Room actor/input model.
use super::browser_controller_process::BrowserCancellation;
use crate::session::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) struct KernelBrowserActors {
    generation: u64,
    desktop: Option<(String, String)>,
    tabs: TabRegistry,
    actors: BTreeMap<String, EnvironmentActor>,
    ledger: EnvironmentActionLedger,
    active: BTreeMap<String, Arc<BrowserCancellation>>,
    pointers: BTreeMap<String, EnvironmentPointer>,
    pointer_tabs: BTreeMap<String, String>,
    host_tabs: BTreeMap<String, String>,
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
            generation: 0,
            desktop: None,
            tabs: TabRegistry::new(),
            actors: BTreeMap::new(),
            ledger: EnvironmentActionLedger::new(256, 0),
            active: BTreeMap::new(),
            pointers: BTreeMap::new(),
            pointer_tabs: BTreeMap::new(),
            host_tabs: BTreeMap::new(),
        }
    }
}
impl KernelBrowserActors {
    pub(crate) fn reconcile(&mut self, state: &Value) -> Result<(), String> {
        let Some(generation) = state["generation"].as_u64() else {
            return Ok(());
        };
        let Some(tabs) = state["tabs"].as_array() else {
            return Ok(());
        };
        if generation != self.generation {
            for cancellation in self.active.values() {
                cancellation.request_cancel();
            }
            self.ledger.invalidate_after_kernel_restart(&self.actors);
            self.active.clear();
            self.pointers.clear();
            self.pointer_tabs.clear();
            self.generation = generation;
        }
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
    pub(crate) fn begin(
        &mut self,
        actor: EnvironmentActor,
        params: &Value,
    ) -> Result<(String, Arc<BrowserCancellation>), String> {
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
            Some("text" | "composition") => {
                request.with_arguments(EnvironmentActionArguments::KeyboardText {
                    utf8_byte_count: input["text"].as_str().unwrap_or_default().len() as u32,
                    character_count: input["text"].as_str().unwrap_or_default().chars().count()
                        as u32,
                })
            }
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
            if x < 1280 && y < 800 {
                if let Some(tab) = params["tab_id"].as_str() {
                    self.pointer_tabs.insert(actor.actor_id.clone(), tab.into());
                }
                self.pointers.insert(
                    actor.actor_id.clone(),
                    EnvironmentPointer {
                        actor_id: actor.actor_id,
                        x: x as u32,
                        y: y as u32,
                        viewport_revision: 1,
                    },
                );
            }
        }
        let cancellation = Arc::new(BrowserCancellation::default());
        self.active.insert(action_id.clone(), cancellation.clone());
        Ok((action_id, cancellation))
    }
    pub(crate) fn finish(&mut self, action_id: &str, terminal: EnvironmentActionTerminal) {
        self.active.remove(action_id);
        // A reconciled process loss can have already finished this action.
        let _ = self.ledger.finish(action_id, terminal);
        self.ledger.compact_terminal_actions();
        self.ledger.compact_transient_history(256);
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
    pub(crate) fn disconnect(&mut self, actor_id: &str) {
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
        serde_json::json!({"generation":self.generation,"actors":self.actors.values().collect::<Vec<_>>(),
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
