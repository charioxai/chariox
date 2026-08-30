use std::collections::BTreeMap;

use super::model::{EnvironmentError, EnvironmentTab};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabState {
    controller_target_id: String,
    tab: EnvironmentTab,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TabRegistry {
    tabs: BTreeMap<String, TabState>,
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
        if let Some(existing) = self
            .tabs
            .values()
            .find(|state| state.controller_target_id == controller_target_id)
        {
            return (existing.tab.tab_id.clone(), false);
        }
        let tab_id = format!("tab-{}", self.next_sequence);
        self.next_sequence += 1;
        let focused = self.focused_tab_id.is_none();
        self.tabs.insert(
            tab_id.clone(),
            TabState {
                controller_target_id,
                tab: EnvironmentTab {
                    tab_id: tab_id.clone(),
                    url,
                    title,
                    document_revision: 1,
                    focused,
                },
            },
        );
        self.order.push(tab_id.clone());
        if focused {
            self.focused_tab_id = Some(tab_id.clone());
        }
        (tab_id, true)
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
        if self.tabs.remove(tab_id).is_none() {
            return Err(EnvironmentError::UnknownTab {
                tab_id: tab_id.to_string(),
            });
        }
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

    pub(crate) fn clear(&mut self) {
        self.tabs.clear();
        self.order.clear();
        self.focused_tab_id = None;
    }
}
