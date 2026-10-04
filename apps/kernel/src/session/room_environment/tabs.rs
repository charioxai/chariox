use std::collections::{BTreeMap, VecDeque};

use super::model::{
    EnvironmentError, EnvironmentTab, EnvironmentTabApp, EnvironmentTabObservation,
    EnvironmentTabRuntimeBinding,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TabState {
    controller_target_id: String,
    document_id: Option<String>,
    tab: EnvironmentTab,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TabRegistry {
    tabs: BTreeMap<String, TabState>,
    tab_id_by_controller_target: BTreeMap<String, String>,
    retired_tab_id_by_controller_target: BTreeMap<String, String>,
    retired_controller_targets: VecDeque<String>,
    order: Vec<String>,
    focused_tab_id: Option<String>,
    next_sequence: u64,
    /// Logical App identities retained for the next physical browser generation.
    /// Only a new kernel-verified App binding may claim one, never a URL.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    restoring_apps: BTreeMap<String, EnvironmentTab>,
    /// A reclaimed App waits for physical activation. Intermediate or delayed
    /// startup-blank receipts must not replace its restored focus intent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovered_app_focus: Option<String>,
}

impl TabRegistry {
    const MAX_RETIRED_CONTROLLER_TARGETS: usize = 2_048;

    pub(crate) fn new() -> Self {
        Self {
            next_sequence: 1,
            ..Self::default()
        }
    }

    pub(crate) fn snapshot(&self) -> (Vec<EnvironmentTab>, Option<String>) {
        let tabs = self
            .order
            .iter()
            .filter_map(|tab_id| self.tabs.get(tab_id))
            .map(|state| {
                let mut tab = state.tab.clone();
                tab.focused = self.focused_tab_id.as_deref() == Some(tab.tab_id.as_str());
                tab
            })
            .collect();
        (tabs, self.focused_tab_id.clone())
    }

    pub(crate) fn register_or_reconcile(
        &mut self,
        controller_target_id: String,
        url: String,
        title: String,
    ) -> (String, bool) {
        if let Some(tab_id) = self.tab_id_by_controller_target.get(&controller_target_id) {
            return (tab_id.clone(), false);
        }
        self.forget_retired_controller_target(&controller_target_id);
        let tab_id = format!("tab-{}", self.next_sequence);
        self.next_sequence += 1;
        let focused = self.focused_tab_id.is_none();
        self.tabs.insert(
            tab_id.clone(),
            TabState {
                controller_target_id: controller_target_id.clone(),
                document_id: None,
                tab: EnvironmentTab {
                    tab_id: tab_id.clone(),
                    url,
                    title,
                    document_revision: 1,
                    focused,
                    app: None,
                },
            },
        );
        self.tab_id_by_controller_target
            .insert(controller_target_id, tab_id.clone());
        self.order.push(tab_id.clone());
        if focused {
            self.focused_tab_id = Some(tab_id.clone());
        }
        (tab_id, true)
    }

    pub(crate) fn reconcile_controller_tabs(
        &mut self,
        observations: Vec<EnvironmentTabObservation>,
        focused_controller_target_id: Option<&str>,
    ) -> bool {
        let mut changed = false;
        let mut observed_targets = std::collections::BTreeSet::new();
        for observation in observations {
            if !observed_targets.insert(observation.runtime_target_id.clone()) {
                continue;
            }
            let tab_id = self
                .tab_id_by_controller_target
                .get(&observation.runtime_target_id)
                .cloned();
            match tab_id.and_then(|tab_id| self.tabs.get_mut(&tab_id)) {
                Some(state) => {
                    let document_changed = state
                        .document_id
                        .as_deref()
                        .is_some_and(|document_id| document_id != observation.document_id)
                        || (state.document_id.is_none() && state.tab.url != observation.url);
                    if document_changed {
                        state.tab.document_revision = state.tab.document_revision.saturating_add(1);
                        changed = true;
                    }
                    if state.document_id.as_deref() != Some(observation.document_id.as_str()) {
                        state.document_id = Some(observation.document_id);
                    }
                    if state.tab.url != observation.url {
                        state.tab.url = observation.url;
                        changed = true;
                    }
                    if state.tab.title != observation.title {
                        state.tab.title = observation.title;
                        changed = true;
                    }
                }
                None => {
                    self.forget_retired_controller_target(&observation.runtime_target_id);
                    let tab_id = format!("tab-{}", self.next_sequence);
                    self.next_sequence = self.next_sequence.saturating_add(1);
                    self.tabs.insert(
                        tab_id.clone(),
                        TabState {
                            controller_target_id: observation.runtime_target_id.clone(),
                            document_id: Some(observation.document_id),
                            tab: EnvironmentTab {
                                tab_id: tab_id.clone(),
                                url: observation.url,
                                title: observation.title,
                                document_revision: 1,
                                focused: false,
                                app: None,
                            },
                        },
                    );
                    self.tab_id_by_controller_target
                        .insert(observation.runtime_target_id, tab_id.clone());
                    self.order.push(tab_id);
                    changed = true;
                }
            }
        }

        let removed_tab_ids = self
            .tabs
            .iter()
            .filter(|(_, state)| !observed_targets.contains(&state.controller_target_id))
            .map(|(tab_id, _)| tab_id.clone())
            .collect::<Vec<_>>();
        for tab_id in removed_tab_ids {
            if let Some(state) = self.tabs.remove(&tab_id) {
                self.tab_id_by_controller_target
                    .remove(&state.controller_target_id);
                self.remember_retired_controller_target(state.controller_target_id, tab_id.clone());
                self.order.retain(|candidate| candidate != &tab_id);
                changed = true;
            }
        }

        let observed_focus = focused_controller_target_id
            .and_then(|target_id| self.tab_id_by_controller_target.get(target_id).cloned());
        let recovery_focus = self
            .recovered_app_focus
            .as_ref()
            .filter(|id| {
                self.tabs.get(*id).is_some_and(|tab| tab.tab.app.is_some())
                    && observed_focus.as_ref().is_none_or(|observed| {
                        observed == *id
                            || self.tabs.get(observed).is_some_and(|tab| {
                                tab.tab.url == "about:blank" && tab.tab.app.is_none()
                            })
                    })
            })
            .cloned();
        if recovery_focus.is_none() {
            self.recovered_app_focus = None;
        }
        let next_focus = recovery_focus
            .or(observed_focus)
            .or_else(|| {
                self.focused_tab_id
                    .as_ref()
                    .filter(|tab_id| self.tabs.contains_key(*tab_id))
                    .cloned()
            })
            .or_else(|| self.order.first().cloned());
        if self.focused_tab_id != next_focus {
            self.focused_tab_id = next_focus;
            changed = true;
        }
        changed
    }

    /// Reattach kernel-held App intent after verified assets opened a new
    /// physical target. Retire the old binding and invalidate its document;
    /// ordinary user close/open reconciliation never calls this path.
    pub(crate) fn restore_app_tab(&mut self, saved: &EnvironmentTab, target: &str) -> bool {
        if self.tab_id_by_controller_target.get(target) == Some(&saved.tab_id) {
            return false;
        }
        if let Some(temporary_id) = self.tab_id_by_controller_target.remove(target) {
            self.tabs.remove(&temporary_id);
            self.order.retain(|id| id != &temporary_id);
            if self.focused_tab_id.as_ref() == Some(&temporary_id) {
                self.focused_tab_id = Some(saved.tab_id.clone());
            }
        }
        let mut tab = saved.clone();
        if let Some(old) = self.tabs.remove(&saved.tab_id) {
            tab.document_revision = tab.document_revision.max(old.tab.document_revision);
            self.tab_id_by_controller_target
                .remove(&old.controller_target_id);
            self.remember_retired_controller_target(old.controller_target_id, saved.tab_id.clone());
        }
        tab.document_revision = tab.document_revision.saturating_add(1);
        self.forget_retired_controller_target(target);
        self.tab_id_by_controller_target
            .insert(target.to_owned(), tab.tab_id.clone());
        if !self.order.contains(&tab.tab_id) {
            self.order.push(tab.tab_id.clone());
        }
        self.tabs.insert(
            tab.tab_id.clone(),
            TabState {
                controller_target_id: target.to_owned(),
                document_id: None,
                tab,
            },
        );
        true
    }

    /// Marks App view Tabs by controller target; true when any Tab changed.
    pub(crate) fn set_apps(&mut self, apps: &BTreeMap<String, EnvironmentTabApp>) -> bool {
        let mut changed = false;
        for state in self.tabs.values_mut() {
            let app = apps.get(&state.controller_target_id).cloned();
            if state.tab.app != app {
                state.tab.app = app;
                changed = true;
            }
        }
        let restored = self
            .tabs
            .iter()
            .filter_map(|(tab_id, state)| {
                let installation = &state.tab.app.as_ref()?.installation_id;
                self.restoring_apps
                    .get(installation)
                    .filter(|previous| {
                        previous.tab_id != *tab_id && !self.tabs.contains_key(&previous.tab_id)
                    })
                    .map(|_| (tab_id.clone(), installation.clone()))
            })
            .collect::<Vec<_>>();
        for (temporary_id, installation) in restored {
            let Some(previous) = self.restoring_apps.remove(&installation) else {
                continue;
            };
            let Some(mut state) = self.tabs.remove(&temporary_id) else {
                continue;
            };
            state.tab.tab_id = previous.tab_id.clone();
            state.tab.document_revision = state
                .tab
                .document_revision
                .max(previous.document_revision.saturating_add(1));
            self.tab_id_by_controller_target
                .insert(state.controller_target_id.clone(), previous.tab_id.clone());
            for tab_id in &mut self.order {
                if *tab_id == temporary_id {
                    *tab_id = previous.tab_id.clone();
                }
            }
            if self.focused_tab_id.is_none()
                || self.focused_tab_id.as_deref() == Some(temporary_id.as_str())
                || previous.focused
                    && self
                        .focused_tab_id
                        .as_ref()
                        .and_then(|id| self.tabs.get(id))
                        .is_some_and(|tab| tab.tab.url == "about:blank" && tab.tab.app.is_none())
            {
                self.focused_tab_id = Some(previous.tab_id.clone());
                if previous.focused {
                    self.recovered_app_focus = Some(previous.tab_id.clone());
                }
            }
            // Late events from a retired physical target must not be labelled
            // as events from the live page that reclaimed this logical id.
            self.retired_tab_id_by_controller_target
                .retain(|_, id| *id != previous.tab_id);
            self.retired_controller_targets.retain(|target| {
                self.retired_tab_id_by_controller_target
                    .contains_key(target)
            });
            self.tabs.insert(previous.tab_id, state);
            changed = true;
        }
        changed
    }

    pub(crate) fn tab_id_for_controller_target(
        &self,
        controller_target_id: &str,
    ) -> Option<String> {
        self.tab_id_by_controller_target
            .get(controller_target_id)
            .or_else(|| {
                self.retired_tab_id_by_controller_target
                    .get(controller_target_id)
            })
            .cloned()
    }

    fn remember_retired_controller_target(&mut self, controller_target_id: String, tab_id: String) {
        self.forget_retired_controller_target(&controller_target_id);
        self.retired_controller_targets
            .push_back(controller_target_id.clone());
        self.retired_tab_id_by_controller_target
            .insert(controller_target_id, tab_id);
        while self.retired_controller_targets.len() > Self::MAX_RETIRED_CONTROLLER_TARGETS {
            if let Some(target_id) = self.retired_controller_targets.pop_front() {
                self.retired_tab_id_by_controller_target.remove(&target_id);
            }
        }
    }

    fn forget_retired_controller_target(&mut self, controller_target_id: &str) {
        self.retired_tab_id_by_controller_target
            .remove(controller_target_id);
        self.retired_controller_targets
            .retain(|candidate| candidate != controller_target_id);
    }

    pub(crate) fn record_navigation(
        &mut self,
        tab_id: &str,
        url: String,
        title: String,
    ) -> Result<(), EnvironmentError> {
        let state = self
            .tabs
            .get_mut(tab_id)
            .ok_or_else(|| EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            })?;
        state.tab.url = url;
        state.tab.title = title;
        state.tab.document_revision += 1;
        Ok(())
    }

    pub(crate) fn close(&mut self, tab_id: &str) -> Result<(), EnvironmentError> {
        let Some(state) = self.tabs.remove(tab_id) else {
            return Err(EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            });
        };
        self.tab_id_by_controller_target
            .remove(&state.controller_target_id);
        self.remember_retired_controller_target(state.controller_target_id, tab_id.to_string());
        self.order.retain(|candidate| candidate != tab_id);
        if self.focused_tab_id.as_deref() == Some(tab_id) {
            self.focused_tab_id = self.order.first().cloned();
        }
        if self.recovered_app_focus.as_deref() == Some(tab_id) {
            self.recovered_app_focus = None;
        }
        Ok(())
    }

    pub(crate) fn validate_reference(
        &self,
        runtime_generation: u64,
        current_generation: u64,
        tab_id: &str,
        document_revision: u64,
    ) -> Result<(), EnvironmentError> {
        if runtime_generation != current_generation {
            return Err(EnvironmentError::StaleRuntimeGeneration {
                expected: current_generation,
                actual: runtime_generation,
            });
        }
        let state = self
            .tabs
            .get(tab_id)
            .ok_or_else(|| EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            })?;
        if document_revision != state.tab.document_revision {
            return Err(EnvironmentError::StaleDocumentRevision {
                tab_id: tab_id.to_string(),
                expected: state.tab.document_revision,
                actual: document_revision,
            });
        }
        Ok(())
    }

    pub(crate) fn contains(&self, tab_id: &str) -> bool {
        self.tabs.contains_key(tab_id)
    }

    pub(crate) fn controller_binding(
        &self,
        tab_id: &str,
    ) -> Result<EnvironmentTabRuntimeBinding, EnvironmentError> {
        let state = self
            .tabs
            .get(tab_id)
            .ok_or_else(|| EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            })?;
        let document_id = state.document_id.clone().ok_or_else(|| {
            EnvironmentError::StructuredObservationUnavailable {
                tab_id: tab_id.to_string(),
            }
        })?;
        Ok(EnvironmentTabRuntimeBinding {
            runtime_target_id: state.controller_target_id.clone(),
            document_id,
            document_revision: state.tab.document_revision,
        })
    }

    pub(crate) fn prepare_app_recovery(&mut self) {
        self.recovered_app_focus = None;
        self.restoring_apps.clear();
        for state in self.tabs.values() {
            if let Some(app) = &state.tab.app {
                let mut tab = state.tab.clone();
                tab.focused = self.focused_tab_id.as_deref() == Some(tab.tab_id.as_str());
                let previous = self
                    .restoring_apps
                    .entry(app.installation_id.clone())
                    .or_insert_with(|| tab.clone());
                if tab.focused {
                    *previous = tab;
                }
            }
        }
    }

    pub(crate) fn cancel_app_recovery_focus(&mut self) {
        self.recovered_app_focus = None;
        for tab in self.restoring_apps.values_mut() {
            tab.focused = false;
        }
    }

    pub(crate) fn clear(&mut self) {
        // Recovery belongs to one physical generation. A failed or uninstalled
        // view must not leave a durable claim over later browser generations.
        // Retry before a binding returns also starts a new generation and
        // deliberately drops its old claim; only this recovery can reclaim it.
        self.prepare_app_recovery();
        self.tabs.clear();
        self.tab_id_by_controller_target.clear();
        self.retired_tab_id_by_controller_target.clear();
        self.retired_controller_targets.clear();
        self.order.clear();
        self.focused_tab_id = None;
    }
}
