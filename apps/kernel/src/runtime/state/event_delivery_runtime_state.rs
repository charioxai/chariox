use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptedWorkflowEventDelivery {
    pub delivery_id: String,
    pub queued_prompt_id: String,
    pub duplicate: bool,
    pub session: crate::session::RuntimeSession,
}

impl KernelRuntimeState {
    pub(crate) fn event_generator_subscription_claims(
        &self,
    ) -> BTreeMap<String, Vec<chariox_event_protocol::AegsSubscriptionClaim>> {
        let mut generators =
            BTreeMap::<String, Vec<chariox_event_protocol::AegsSubscriptionClaim>>::new();
        for session in self.owned.session_store.read().list_sessions() {
            for binding in session.workflow_event_bindings() {
                if binding.status == crate::session::WorkflowEventBindingStatus::Tombstoned {
                    continue;
                }
                generators
                    .entry(binding.generator_id.clone())
                    .or_default()
                    .push(chariox_event_protocol::AegsSubscriptionClaim {
                        binding_id: binding.id.clone(),
                        generator_id: binding.generator_id.clone(),
                        connection_id: binding.connection_id.clone(),
                        connection_scope: binding.connection_scope.clone(),
                        event_interest_key: binding.event_interest_key.clone(),
                        event_type: binding.event_type.clone(),
                        event_type_version: binding.event_type_version,
                        filter: binding.filter.clone(),
                        revision: binding.revision,
                        active: event_binding_effectively_active(&session, binding),
                    });
            }
        }
        for claims in generators.values_mut() {
            claims.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        }
        generators
    }

    pub(crate) fn active_event_route_claims(
        &self,
        kernel_id: &str,
    ) -> Vec<chariox_event_protocol::EnvironmentRouteClaim> {
        self.owned
            .session_store
            .read()
            .list_sessions()
            .into_iter()
            .flat_map(|session| {
                session
                    .workflow_event_bindings()
                    .iter()
                    .filter(|binding| event_binding_effectively_active(&session, binding))
                    .map(|binding| binding.route_claim(kernel_id.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(crate) fn event_delivery_resumes(
        &self,
        kernel_id: &str,
        default_environment_id: &str,
    ) -> Vec<chariox_event_protocol::KernelEnvironmentResume> {
        let mut environments = BTreeMap::<
            String,
            (
                Vec<chariox_event_protocol::EnvironmentRouteClaim>,
                Option<(u64, String)>,
            ),
        >::new();
        environments.insert(default_environment_id.to_string(), (Vec::new(), None));
        for session in self.owned.session_store.read().list_sessions() {
            let binding_environments = session
                .workflow_event_bindings()
                .iter()
                .map(|binding| (binding.id.as_str(), binding.environment_id.as_str()))
                .collect::<BTreeMap<_, _>>();
            for binding in session.workflow_event_bindings() {
                let entry = environments
                    .entry(binding.environment_id.clone())
                    .or_insert_with(|| (Vec::new(), None));
                if event_binding_effectively_active(&session, binding) {
                    entry.0.push(binding.route_claim(kernel_id.to_string()));
                }
            }
            for receipt in session.workflow_event_delivery_receipts().values() {
                let Some(environment_id) = binding_environments.get(receipt.binding_id.as_str())
                else {
                    continue;
                };
                let entry = environments
                    .entry((*environment_id).to_string())
                    .or_insert_with(|| (Vec::new(), None));
                if entry
                    .1
                    .as_ref()
                    .is_none_or(|(accepted_at_ms, _)| *accepted_at_ms < receipt.accepted_at_ms)
                {
                    entry.1 = Some((receipt.accepted_at_ms, receipt.delivery_id.clone()));
                }
            }
        }
        environments
            .into_iter()
            .map(|(environment_id, (routes, last_delivery))| {
                chariox_event_protocol::KernelEnvironmentResume {
                    environment_id,
                    last_accepted_delivery_id: last_delivery.map(|(_, delivery_id)| delivery_id),
                    routes,
                }
            })
            .collect()
    }

    pub(crate) fn apply_event_route_conflicts(
        &self,
        conflicts: &[chariox_event_protocol::EventRouteConflict],
    ) {
        let mut changed_session_ids = BTreeSet::new();
        for conflict in conflicts {
            let Some((session_id, binding)) = self
                .owned
                .session_store
                .read()
                .find_workflow_event_binding(&conflict.requested_binding_id)
            else {
                continue;
            };
            if binding.status == crate::session::WorkflowEventBindingStatus::Conflict {
                continue;
            }
            if self
                .owned
                .session_store
                .write()
                .set_workflow_event_binding_status(
                    &session_id,
                    &binding.id,
                    crate::session::WorkflowEventBindingStatus::Conflict,
                )
                .is_ok()
            {
                changed_session_ids.insert(session_id);
            }
        }
        for session_id in changed_session_ids {
            if let Err(error) = self
                .owned
                .persist_workflow_runtime_session(&session_id, "workflow_event_route_conflict")
            {
                crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "failed to persist event route conflict",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    pub(crate) fn accept_workflow_event_delivery(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
    ) -> Result<AcceptedWorkflowEventDelivery, DaemonError> {
        let now_ms = crate::session::unix_epoch_ms();
        delivery
            .validate(now_ms)
            .map_err(|message| DaemonError::LocalTransport {
                operation: "accept workflow event delivery",
                message,
            })?;
        let (session_id, binding, publication) = self
            .owned
            .session_store
            .read()
            .find_active_workflow_event_binding(&delivery.binding_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "accept workflow event delivery",
                message: format!(
                    "active workflow event binding `{}` was not found",
                    delivery.binding_id
                ),
            })?;
        if binding.event_type != delivery.event_type
            || binding.event_type_version != delivery.event_type_version
        {
            return Err(DaemonError::LocalTransport {
                operation: "accept workflow event delivery",
                message: format!(
                    "delivery event `{}@{}` does not match binding `{}@{}`",
                    delivery.event_type,
                    delivery.event_type_version,
                    binding.event_type,
                    binding.event_type_version
                ),
            });
        }
        if let Some(receipt) = self
            .owned
            .session_store
            .read()
            .get_session(&session_id)?
            .workflow_event_delivery_receipts()
            .get(&delivery.delivery_id)
            .cloned()
        {
            return Ok(AcceptedWorkflowEventDelivery {
                delivery_id: delivery.delivery_id,
                queued_prompt_id: receipt.queued_prompt_id,
                duplicate: true,
                session: self.owned.session_snapshot(&session_id)?,
            });
        }

        let before = self.owned.session_snapshot(&session_id)?;
        let artifacts = delivery
            .artifacts
            .iter()
            .filter_map(|artifact| serde_json::to_value(artifact).ok())
            .collect::<Vec<_>>();
        let invocation = crate::session::WorkflowPublicationInvocationEnvelope {
            publication_id: publication.id().to_string(),
            hook_id: Some(binding.id.clone()),
            invocation_id: delivery.delivery_id.clone(),
            transport: "event".to_string(),
            endpoint_id: binding.endpoint_id.clone(),
            queue_ref: binding.queue_ref.clone(),
            input: serde_json::json!({
                "event_type": &delivery.event_type,
                "event_type_version": delivery.event_type_version,
                "occurrence_id": &delivery.occurrence_id,
                "occurred_at": &delivery.occurred_at,
                "metadata": &delivery.metadata,
                "reply_context": &delivery.reply_context,
            }),
            artifacts,
            mode: None,
            caller: serde_json::json!({
                "kind": "event_delivery",
                "binding_id": binding.id,
            }),
        };
        let queued_prompt = match self
            .owned
            .session_store
            .write()
            .enqueue_workflow_prompt_with_publication_invocation(
                &session_id,
                publication.workflow_id(),
                &binding.endpoint_id,
                Some(delivery.prompt.clone()),
                binding.queue_ref.as_deref(),
                crate::session::WorkflowQueuedPromptSource::Event,
                None,
                Some(invocation),
            ) {
            Ok(prompt) => prompt,
            Err(error) => return Err(error),
        };
        if let Err(error) = self
            .owned
            .session_store
            .write()
            .record_workflow_event_delivery_receipt(
                &session_id,
                crate::session::WorkflowEventDeliveryReceipt {
                    delivery_id: delivery.delivery_id.clone(),
                    binding_id: delivery.binding_id.clone(),
                    occurrence_id: delivery.occurrence_id.clone(),
                    queued_prompt_id: queued_prompt.id().to_string(),
                    accepted_at_ms: now_ms,
                    expires_at_ms: delivery.expires_at_ms,
                },
            )
        {
            self.owned.session_store.write().restore_session(before);
            return Err(error);
        }
        if let Err(error) = self
            .owned
            .persist_workflow_runtime_session(&session_id, "workflow_event_delivery_accepted")
        {
            self.owned.session_store.write().restore_session(before);
            return Err(error);
        }

        let dispatches = match self
            .owned
            .workflow_start_next_queued_prompt_for_response(&session_id)
        {
            Ok(Some((_, dispatches))) => dispatches,
            Ok(None) => WorkflowPromptDispatches::default(),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "event prompt was persisted but could not be dispatched",
                    serde_json::json!({
                        "delivery_id": delivery.delivery_id,
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                WorkflowPromptDispatches::default()
            }
        };
        if !dispatches.is_empty() {
            let _ = self
                .owned
                .persist_workflow_runtime_session(&session_id, "workflow_event_dispatch_started");
            self.spawn_workflow_prompt_dispatches(dispatches);
        }
        Ok(AcceptedWorkflowEventDelivery {
            delivery_id: delivery.delivery_id,
            queued_prompt_id: queued_prompt.id().to_string(),
            duplicate: false,
            session: self.owned.session_snapshot(&session_id)?,
        })
    }
}

fn event_binding_effectively_active(
    session: &crate::session::RuntimeSession,
    binding: &crate::session::WorkflowEventBinding,
) -> bool {
    binding.active()
        && session
            .workflow_publications()
            .iter()
            .any(|publication| publication.id() == binding.publication_id && publication.enabled())
}
