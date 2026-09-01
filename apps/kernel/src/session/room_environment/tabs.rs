use std::collections::BTreeMap;

use super::model::{
    EnvironmentError, EnvironmentTab, EnvironmentTabObservation, EnvironmentTabRuntimeBinding,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabState {
    controller_target_id: String,
    document_id: Option<String>,
    tab: EnvironmentTab,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TabRegistry {
    tabs: BTreeMap<String, TabState>,
    tab_id_by_controller_target: BTreeMap<String, String>,
    order: Vec<String>,
    focused_tab_id: Option<String>,
    next_sequence: u64,
}

impl TabRegistry {
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
                self.order.retain(|candidate| candidate != &tab_id);
                changed = true;
            }
        }

        let observed_focus = focused_controller_target_id
            .and_then(|target_id| self.tab_id_by_controller_target.get(target_id).cloned());
        let next_focus = observed_focus
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
        self.order.retain(|candidate| candidate != tab_id);
        if self.focused_tab_id.as_deref() == Some(tab_id) {
            self.focused_tab_id = self.order.first().cloned();
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

    pub(crate) fn clear(&mut self) {
        self.tabs.clear();
        self.tab_id_by_controller_target.clear();
        self.order.clear();
        self.focused_tab_id = None;
    }
}
