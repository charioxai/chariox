use std::collections::BTreeMap;

use super::model::EnvironmentError;
use super::tabs::TabRegistry;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ElementIdentity {
    tab_id: String,
    runtime_generation: u64,
    document_revision: u64,
    controller_node_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentElementTarget {
    pub(crate) tab_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) document_revision: u64,
    pub(crate) controller_node_ref: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ElementReferenceRegistry {
    target_by_reference: BTreeMap<String, EnvironmentElementTarget>,
    reference_by_identity: BTreeMap<ElementIdentity, String>,
    next_sequence: u64,
}

impl ElementReferenceRegistry {
    pub(crate) fn new() -> Self {
        Self {
            next_sequence: 1,
            ..Self::default()
        }
    }

    pub(crate) fn register(
        &mut self,
        tabs: &TabRegistry,
        current_runtime_generation: u64,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, String>, EnvironmentError> {
        tabs.validate_reference(
            runtime_generation,
            current_runtime_generation,
            tab_id,
            document_revision,
        )?;
        self.retain_current(tabs, current_runtime_generation);
        let mut registered = BTreeMap::new();
        for controller_node_ref in controller_node_refs {
            let identity = ElementIdentity {
                tab_id: tab_id.to_string(),
                runtime_generation,
                document_revision,
                controller_node_ref: controller_node_ref.clone(),
            };
            let reference_id = match self.reference_by_identity.get(&identity) {
                Some(existing) => existing.clone(),
                None => {
                    let reference_id = format!("element-{}", self.next_sequence);
                    self.next_sequence = self.next_sequence.saturating_add(1);
                    self.reference_by_identity
                        .insert(identity, reference_id.clone());
                    self.target_by_reference.insert(
                        reference_id.clone(),
                        EnvironmentElementTarget {
                            tab_id: tab_id.to_string(),
                            runtime_generation,
                            document_revision,
                            controller_node_ref: controller_node_ref.clone(),
                        },
                    );
                    reference_id
                }
            };
            registered.insert(controller_node_ref, reference_id);
        }
        Ok(registered)
    }

    pub(crate) fn resolve(
        &self,
        tabs: &TabRegistry,
        current_runtime_generation: u64,
        reference_id: &str,
    ) -> Result<EnvironmentElementTarget, EnvironmentError> {
        let target = self.target_by_reference.get(reference_id).ok_or_else(|| {
            EnvironmentError::StaleElementReference {
                reference_id: reference_id.to_string(),
            }
        })?;
        tabs.validate_reference(
            target.runtime_generation,
            current_runtime_generation,
            &target.tab_id,
            target.document_revision,
        )
        .map_err(|_| EnvironmentError::StaleElementReference {
            reference_id: reference_id.to_string(),
        })?;
        Ok(target.clone())
    }

    pub(crate) fn retain_current(&mut self, tabs: &TabRegistry, current_runtime_generation: u64) {
        let stale_references = self
            .target_by_reference
            .iter()
            .filter(|(_, target)| {
                tabs.validate_reference(
                    target.runtime_generation,
                    current_runtime_generation,
                    &target.tab_id,
                    target.document_revision,
                )
                .is_err()
            })
            .map(|(reference_id, _)| reference_id.clone())
            .collect::<Vec<_>>();
        for reference_id in stale_references {
            if let Some(target) = self.target_by_reference.remove(&reference_id) {
                self.reference_by_identity.remove(&ElementIdentity {
                    tab_id: target.tab_id,
                    runtime_generation: target.runtime_generation,
                    document_revision: target.document_revision,
                    controller_node_ref: target.controller_node_ref,
                });
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.target_by_reference.clear();
        self.reference_by_identity.clear();
    }
}
