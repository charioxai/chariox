use super::*;
use crate::session::{
    WorkflowEventBinding, WorkflowEventBindingStatus, WorkflowEventDeliveryReceipt,
    WORKFLOW_PUBLICATION_KIND_EVENT_BASED,
};

impl SessionService {
    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow_event_binding(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        generator_id: String,
        generator_version: String,
        manifest_digest: String,
        connection_id: String,
        connection_scope: String,
        event_type: String,
        event_type_version: u32,
        filter: Value,
        environment_id: Option<String>,
        queue_ref: Option<String>,
        reply_mode: Option<String>,
    ) -> Result<WorkflowEventBinding, DaemonError> {
        let publication = self.resolve_workflow_publication_ref(session_id, publication_ref)?;
        if publication.kind() != WORKFLOW_PUBLICATION_KIND_EVENT_BASED {
            return Err(DaemonError::LocalTransport {
                operation: "create workflow event binding",
                message: format!(
                    "workflow publication `{}` is `{}` instead of `event_based`",
                    publication.id(),
                    publication.kind()
                ),
            });
        }
        if !publication.enabled() {
            return Err(DaemonError::LocalTransport {
                operation: "create workflow event binding",
                message: "workflow publication is disabled; create a new event-based publication before subscribing"
                    .to_string(),
            });
        }
        for (name, value) in [
            ("generator_id", generator_id.as_str()),
            ("generator_version", generator_version.as_str()),
            ("manifest_digest", manifest_digest.as_str()),
            ("connection_id", connection_id.as_str()),
            ("connection_scope", connection_scope.as_str()),
            ("event_type", event_type.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "create workflow event binding",
                    message: format!("{name} is required"),
                });
            }
        }
        if event_type_version == 0 {
            return Err(DaemonError::LocalTransport {
                operation: "create workflow event binding",
                message: "event_type_version must be greater than zero".to_string(),
            });
        }
        let environment_id = environment_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.event_environment_id.clone());
        let queue_ref = queue_ref
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| publication.queue_ref().map(str::to_string));
        let reply_mode = normalize_event_reply_mode(reply_mode)?;
        self.resolve_workflow_prompt_queue_ref(
            session_id,
            publication.workflow_id(),
            queue_ref.as_deref().unwrap_or("default"),
        )?;
        let event_interest_key = chariox_event_protocol::event_interest_key(
            &generator_id,
            &event_type,
            event_type_version,
            &connection_scope,
            &filter,
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "create workflow event binding",
            message: format!("failed to canonicalize event filter: {error}"),
        })?;

        for session in self.store.list() {
            if let Some(existing) = session.workflow_event_bindings().iter().find(|binding| {
                binding.environment_id == environment_id
                    && binding.event_interest_key == event_interest_key
                    && binding.active()
            }) {
                if existing.publication_id == publication.id()
                    && existing.endpoint_id == publication.endpoint_id()
                    && existing.queue_ref == queue_ref
                {
                    return Ok(existing.clone());
                }
                return Err(DaemonError::LocalTransport {
                    operation: "create workflow event binding",
                    message: format!(
                        "event route conflict: environment `{environment_id}` already routes interest `{event_interest_key}` to binding `{}` on publication `{}`; transfer it explicitly",
                        existing.id, existing.publication_id
                    ),
                });
            }
        }

        let now = unix_epoch_ms();
        let binding = WorkflowEventBinding {
            id: self.next_workflow_event_binding_id(),
            publication_id: publication.id().to_string(),
            generator_id,
            generator_version,
            manifest_digest,
            connection_id,
            connection_scope,
            event_type,
            event_type_version,
            filter,
            event_interest_key,
            environment_id,
            endpoint_id: publication.endpoint_id().to_string(),
            queue_ref,
            reply_mode: Some(reply_mode),
            revision: 1,
            status: WorkflowEventBindingStatus::Active,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_event_binding(binding))
    }

    pub fn list_workflow_event_bindings(
        &self,
        session_id: &str,
        publication_ref: Option<&str>,
    ) -> Result<Vec<WorkflowEventBinding>, DaemonError> {
        let publication_id = publication_ref
            .map(|reference| {
                self.resolve_workflow_publication_ref(session_id, reference)
                    .map(|publication| publication.id().to_string())
            })
            .transpose()?;
        Ok(self
            .get_session(session_id)?
            .workflow_event_bindings()
            .iter()
            .filter(|binding| {
                publication_id
                    .as_deref()
                    .is_none_or(|id| binding.publication_id == id)
            })
            .cloned()
            .collect())
    }

    pub fn set_workflow_event_binding_status(
        &mut self,
        session_id: &str,
        binding_id: &str,
        status: WorkflowEventBindingStatus,
    ) -> Result<WorkflowEventBinding, DaemonError> {
        let candidate = self
            .get_session(session_id)?
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == binding_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "set workflow event binding status",
                message: format!("workflow event binding `{binding_id}` was not found"),
            })?;
        if status == WorkflowEventBindingStatus::Active {
            for session in self.store.list() {
                if let Some(existing) = session.workflow_event_bindings().iter().find(|binding| {
                    binding.id != candidate.id
                        && binding.environment_id == candidate.environment_id
                        && binding.event_interest_key == candidate.event_interest_key
                        && binding.active()
                }) {
                    return Err(DaemonError::LocalTransport {
                        operation: "set workflow event binding status",
                        message: format!(
                            "event route conflict: environment `{}` already routes interest `{}` to binding `{}` on publication `{}`; transfer it explicitly",
                            candidate.environment_id,
                            candidate.event_interest_key,
                            existing.id,
                            existing.publication_id
                        ),
                    });
                }
            }
            let publication =
                self.resolve_workflow_publication_ref(session_id, &candidate.publication_id)?;
            if !publication.enabled() {
                return Err(DaemonError::LocalTransport {
                    operation: "set workflow event binding status",
                    message: "the owning publication is disabled".to_string(),
                });
            }
        }
        let binding = self
            .store
            .get_mut(session_id)
            .and_then(|session| session.workflow_event_binding_mut(binding_id))
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "set workflow event binding status",
                message: format!("workflow event binding `{binding_id}` was not found"),
            })?;
        binding.set_status(status);
        Ok(binding.clone())
    }

    pub fn transfer_workflow_event_binding(
        &mut self,
        source_session_id: &str,
        binding_id: &str,
        target_session_id: &str,
        target_publication_ref: &str,
    ) -> Result<WorkflowEventBinding, DaemonError> {
        let source_binding = self
            .get_session(source_session_id)?
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == binding_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "transfer workflow event binding",
                message: format!("workflow event binding `{binding_id}` was not found"),
            })?;
        let target_publication =
            self.resolve_workflow_publication_ref(target_session_id, target_publication_ref)?;
        if target_publication.kind() != WORKFLOW_PUBLICATION_KIND_EVENT_BASED {
            return Err(DaemonError::LocalTransport {
                operation: "transfer workflow event binding",
                message: "target publication must be event_based".to_string(),
            });
        }
        if !target_publication.enabled() {
            return Err(DaemonError::LocalTransport {
                operation: "transfer workflow event binding",
                message: "target publication is disabled".to_string(),
            });
        }
        let target_queue_ref = target_publication
            .queue_ref()
            .unwrap_or("default")
            .to_string();
        self.resolve_workflow_prompt_queue_ref(
            target_session_id,
            target_publication.workflow_id(),
            &target_queue_ref,
        )?;

        let mut moved = source_binding;
        moved.publication_id = target_publication.id().to_string();
        moved.endpoint_id = target_publication.endpoint_id().to_string();
        moved.queue_ref = Some(target_queue_ref);
        moved.status = WorkflowEventBindingStatus::Active;
        moved.revision = moved.revision.saturating_add(1);
        moved.updated_at_ms = unix_epoch_ms();

        if source_session_id == target_session_id {
            let session = self.store.get_mut(source_session_id).ok_or_else(|| {
                DaemonError::SessionNotFound {
                    session_id: source_session_id.to_string(),
                }
            })?;
            let _ = session.remove_workflow_event_binding(binding_id);
            return Ok(session.create_workflow_event_binding(moved));
        }
        {
            let source = self.store.get_mut(source_session_id).ok_or_else(|| {
                DaemonError::SessionNotFound {
                    session_id: source_session_id.to_string(),
                }
            })?;
            let _ = source.remove_workflow_event_binding(binding_id);
        }
        let target =
            self.store
                .get_mut(target_session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: target_session_id.to_string(),
                })?;
        Ok(target.create_workflow_event_binding(moved))
    }

    pub fn find_active_workflow_event_binding(
        &self,
        binding_id: &str,
    ) -> Option<(String, WorkflowEventBinding, WorkflowPublicationDefinition)> {
        self.store.list().into_iter().find_map(|session| {
            let binding = session
                .workflow_event_bindings()
                .iter()
                .find(|binding| binding.id == binding_id && binding.active())?
                .clone();
            let publication = session
                .workflow_publications()
                .iter()
                .find(|publication| {
                    publication.id() == binding.publication_id && publication.enabled()
                })?
                .clone();
            Some((session.id().to_string(), binding, publication))
        })
    }

    pub fn find_workflow_event_binding(
        &self,
        binding_id: &str,
    ) -> Option<(String, WorkflowEventBinding)> {
        self.store.list().into_iter().find_map(|session| {
            session
                .workflow_event_bindings()
                .iter()
                .find(|binding| binding.id == binding_id)
                .cloned()
                .map(|binding| (session.id().to_string(), binding))
        })
    }

    pub fn workflow_event_delivery_was_accepted(
        &self,
        session_id: &str,
        delivery_id: &str,
    ) -> Result<bool, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .workflow_event_delivery_receipts()
            .contains_key(delivery_id))
    }

    pub fn record_workflow_event_delivery_receipt(
        &mut self,
        session_id: &str,
        receipt: WorkflowEventDeliveryReceipt,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session.prune_expired_workflow_event_delivery_receipts(unix_epoch_ms());
        session.record_workflow_event_delivery_receipt(receipt);
        Ok(())
    }
}

fn normalize_event_reply_mode(value: Option<String>) -> Result<String, DaemonError> {
    let value = value
        .unwrap_or_else(|| "disabled".to_string())
        .trim()
        .to_ascii_lowercase();
    if matches!(value.as_str(), "disabled" | "thread" | "channel") {
        Ok(value)
    } else {
        Err(DaemonError::LocalTransport {
            operation: "create workflow event binding",
            message: "reply_mode must be `disabled`, `thread`, or `channel`".to_string(),
        })
    }
}
