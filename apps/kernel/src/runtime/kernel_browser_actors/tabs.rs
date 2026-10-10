//! MP-08/MP-11: shared opener attribution and agent activity, without input authority.
use crate::session::{EnvironmentActor, EnvironmentActorKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct BrowserTabActivity {
    openers: BTreeMap<String, EnvironmentActor>,
    // MP-08: only controller evidence naming an admitted action can attribute
    // a creation. A persistent opener/document alone says nothing about actor.
    actions: BTreeMap<String, (String, String, EnvironmentActor)>,
    sequence: u64,
    activity: Option<Value>,
}
impl BrowserTabActivity {
    pub(super) fn retain(&mut self, tabs: &Value) {
        if let Some(tabs) = tabs.as_array() {
            let live = |id: &str| tabs.iter().any(|tab| tab["tab_id"] == id);
            self.openers.retain(|id, _| live(id));
            // MP-08/MP-11: captured creations survive their source navigation.
            // finish() bounds these associations by the compacted action ledger;
            // cancellation, takeover and process restart still discard them.
            if self
                .activity
                .as_ref()
                .and_then(|a| a["tab_id"].as_str())
                .is_some_and(|id| !live(id))
            {
                self.activity = None;
            }
        }
    }
    pub(super) fn remember_action(
        &mut self,
        action: &str,
        actor: &EnvironmentActor,
        source: &str,
        document: &str,
    ) {
        self.actions.insert(
            action.into(),
            (source.into(), document.into(), actor.clone()),
        );
    }
    pub(super) fn creation_actor(&self, action: &str) -> Option<&EnvironmentActor> {
        self.actions.get(action).map(|(_, _, actor)| actor)
    }
    pub(super) fn forget_source(&mut self, source: &str) {
        self.actions.retain(|_, (tab, _, _)| tab != source);
    }
    pub(super) fn forget_action(&mut self, action: &str) {
        self.actions.remove(action);
    }
    pub(super) fn retain_actions(&mut self, live: impl Fn(&str) -> bool) {
        self.actions.retain(|id, _| live(id));
    }
    pub(super) fn opened(&mut self, actor: EnvironmentActor, tab: &str) {
        self.openers.insert(tab.into(), actor.clone());
        self.acted(&actor, tab);
    }
    pub(super) fn acted(&mut self, actor: &EnvironmentActor, tab: &str) {
        if actor.kind == EnvironmentActorKind::Agent {
            self.sequence = self.sequence.saturating_add(1);
            self.activity =
                Some(json!({"sequence":self.sequence,"tab_id":tab,"actor":actor_view(actor)}));
        }
    }
    pub(super) fn activity(&self) -> Value {
        self.activity.clone().unwrap_or(Value::Null)
    }
    pub(super) fn project(&self, state: &mut Value) {
        if let Some(tabs) = state["tabs"].as_array_mut() {
            for tab in tabs {
                let opener = tab["tab_id"].as_str().and_then(|id| self.openers.get(id));
                tab["opened_by"] = opener.map(actor_view).unwrap_or(Value::Null);
            }
            state["agent_activity"] = self.activity();
        }
    }
}

fn actor_view(actor: &EnvironmentActor) -> Value {
    json!({"actor_id":actor.actor_id,"kind":actor.kind,"display_label":actor.display_label})
}
