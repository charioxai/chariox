//! Process-local admission barrier for a retained publication kernel.
//! Durable state is restored before the gateway validates this boot's bindings.
//! Never persist activation: every replacement must prepare and activate again.

use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub(super) struct PublicationActivation {
    required: bool,
    active: AtomicBool,
    prepared: std::sync::Mutex<BTreeMap<(String, String, String), String>>,
    changed: Notify,
}

impl PublicationActivation {
    pub(super) fn new(requires_activation: bool) -> Self {
        Self {
            required: requires_activation,
            active: AtomicBool::new(!requires_activation),
            ..Self::default()
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(super) async fn wait(&self) {
        loop {
            let notified = self.changed.notified();
            if self.is_active() {
                return;
            }
            notified.await;
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn invalidate_publication_preparation(
        &self,
        request: &crate::local::MaterializeWorkflowPublicationRequest,
        caller_user_id: &str,
    ) {
        if let Some(key) = request.runtime_key.as_deref() {
            self.publication_activation
                .prepared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&(
                    request.publication_id.clone(),
                    caller_user_id.to_string(),
                    key.trim().to_string(),
                ));
        }
    }

    pub(super) fn record_prepared_publication(&self, session: &crate::session::RuntimeSession) {
        let mut prepared = self
            .publication_activation
            .prepared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for publication in session.workflow_publications() {
            if let Some(binding) = publication.runtime_materialization() {
                prepared.insert(
                    (
                        publication.id().to_string(),
                        session.owner_user_id().to_string(),
                        binding.key.clone(),
                    ),
                    session.id().to_string(),
                );
            }
        }
    }

    pub(super) fn activate_workflow_publication_runtime(
        &self,
        request: crate::local::ActivateWorkflowPublicationRuntimeRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let requested = request
            .runtime_keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if requested.is_empty()
            || requested.len() > 32
            || requested.len() != request.runtime_keys.len()
        {
            return Err(activation_error("requires distinct prepared runtime keys"));
        }
        // Use the same lane/creation lock as materialization. A failed or stale
        // preparation must not open the gate for a different retained session.
        let _creation = self
            .workflow_instance_provision_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prepared = self
            .publication_activation
            .prepared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut expected = BTreeSet::new();
        for session in self.session_store.list_all_sessions() {
            for publication in session.workflow_publications() {
                let Some(binding) = publication.runtime_materialization() else {
                    continue;
                };
                if !publication.enabled()
                    || session.status() == crate::session::SessionStatus::Ended
                {
                    continue;
                }
                if !self.publication_activation.required
                    && (publication.id() != request.publication_id
                        || session.owner_user_id() != caller_user_id
                        || !requested.contains(&binding.key))
                {
                    continue;
                }
                if publication.id() != request.publication_id
                    || session.owner_user_id() != caller_user_id
                    || prepared.get(&(
                        publication.id().to_string(),
                        caller_user_id.to_string(),
                        binding.key.clone(),
                    )) != Some(&session.id().to_string())
                {
                    return Err(activation_error(
                        "requires every retained runtime to be prepared by its owner",
                    ));
                }
                expected.insert(binding.key.clone());
            }
        }
        if requested != expected {
            return Err(activation_error("does not match the prepared runtime set"));
        }
        self.publication_activation
            .active
            .store(true, Ordering::Release);
        self.publication_activation.changed.notify_waiters();
        Ok(LocalDaemonResponse::WorkflowPublicationRuntimeActivated {
            publication_id: request.publication_id,
            runtime_keys: request.runtime_keys,
        })
    }

    pub(super) fn require_publication_activation(&self) -> Result<(), DaemonError> {
        if self.publication_activation.is_active() {
            return Ok(());
        }
        Err(activation_error(
            "is awaiting validated runtime preparation",
        ))
    }
}

fn activation_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "publication runtime activation",
        message: message.to_string(),
    }
}
