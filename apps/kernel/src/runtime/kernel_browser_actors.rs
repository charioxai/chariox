//! MD-3: sessionless attachment of the shared Room actor/input model.
use super::browser_controller_process::BrowserCancellation;
use crate::session::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

mod tabs;
use tabs::BrowserTabActivity;

pub(crate) struct KernelBrowserActors {
    tab_activity: BrowserTabActivity,
    generation: u64,
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
            tab_activity: BrowserTabActivity::default(),
            generation: 0,
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
            self.tab_activity = BrowserTabActivity::default();
        }
        self.tab_activity.retain(&state["tabs"]);
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
        let request = if let Some(host_tab) = params["tab_id"].as_str() {
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
            request.mode = EnvironmentMode::Browser;
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
                        actor_id: actor.actor_id.clone(),
                        x: x as u32,
                        y: y as u32,
                        viewport_revision: 1,
                    },
                );
            }
        }
        if let Some(tab) = params["tab_id"].as_str() {
            self.tab_activity.acted(&actor, tab);
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

    pub(crate) fn reconcile_attributed(
        &mut self,
        state: &Value,
        actor: &EnvironmentActor,
        source_tab: Option<&str>,
        openers: &Value,
    ) -> Result<(), String> {
        let same_generation = state["generation"].as_u64() == Some(self.generation);
        let discovered = state["tabs"]
            .as_array()
            .map(|tabs| {
                tabs.iter()
                    .filter_map(|tab| {
                        let id = tab["tab_id"].as_str()?;
                        // MP-08: a coincident native tab is not evidence of this
                        // actor's creation. Only Chromium's opener links it to
                        // the mutated target; explicit open is handled separately.
                        (same_generation
                            && self.tab(id).is_err()
                            && source_tab.is_some_and(|source| openers[id] == source))
                        .then(|| id.to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.reconcile(state)?;
        for tab in discovered {
            self.opened_tab(actor.clone(), &tab);
        }
        Ok(())
    }
    pub(crate) fn opened_tab(&mut self, actor: EnvironmentActor, tab: &str) {
        self.tab_activity.opened(actor, tab);
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
    #[test]
    fn mp08_native_tab_during_agent_mutation_has_no_agent_opener() {
        let mut model = KernelBrowserActors::default();
        model
            .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"}]}))
            .unwrap();
        let actor = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Mara");
        let (action, _) = model
            .begin(
                actor.clone(),
                &json!({"op":"navigate","tab_id":"host-a","generation":1}),
            )
            .unwrap();
        let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"next"},{"tab_id":"native-human","document_id":"human"}]});
        model
            .reconcile_attributed(&state, &actor, Some("host-a"), &Value::Null)
            .unwrap();
        model.finish(&action, EnvironmentActionTerminal::Completed);
        model.project_tabs(&mut state);
        assert!(state["tabs"][1]["opened_by"].is_null());
        assert_eq!(state["agent_activity"]["tab_id"], "host-a");
    }

    #[test]
    fn mp08_only_popup_from_mutated_tab_gets_agent_attribution() {
        let mut model = KernelBrowserActors::default();
        model.reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"other","document_id":"other"}]})).unwrap();
        let actor = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Mara");
        let mut state = json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"other","document_id":"other"},{"tab_id":"popup","document_id":"popup"},{"tab_id":"native-human","document_id":"human"},{"tab_id":"other-popup","document_id":"other-popup"}]});
        model
            .reconcile_attributed(
                &state,
                &actor,
                Some("host-a"),
                &json!({"popup":"host-a","other-popup":"other"}),
            )
            .unwrap();
        model.project_tabs(&mut state);
        assert_eq!(state["tabs"][2]["opened_by"]["actor_id"], "agent:a");
        assert!(state["tabs"][3]["opened_by"].is_null());
        assert!(state["tabs"][4]["opened_by"].is_null());
        assert_eq!(state["agent_activity"]["tab_id"], "popup");
        assert_eq!(state["agent_activity"]["sequence"], 1);
    }

    #[test]
    fn mp08_browser_restart_does_not_attribute_restored_popups_to_mutation() {
        let mut model = KernelBrowserActors::default();
        model
            .reconcile(&json!({"generation":1,"tabs":[{"tab_id":"host-a","document_id":"a"}]}))
            .unwrap();
        let actor = EnvironmentActor::new("agent:a", EnvironmentActorKind::Agent, "Mara");
        let mut state = json!({"generation":2,"tabs":[{"tab_id":"host-a","document_id":"a"},{"tab_id":"restored","document_id":"restored"}]});
        model
            .reconcile_attributed(
                &state,
                &actor,
                Some("host-a"),
                &json!({"restored":"host-a"}),
            )
            .unwrap();
        model.project_tabs(&mut state);
        assert!(state["tabs"][1]["opened_by"].is_null());
        assert!(state["agent_activity"].is_null());
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
