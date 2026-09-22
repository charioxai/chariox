use super::*;

#[cfg(test)]
std::thread_local! {
    static BEFORE_EVENT_ACTIVITY_GATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_EVENT_ACTIVITY_GATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_event_activity_gate_hook(
    hook: &'static std::thread::LocalKey<std::cell::RefCell<Option<Box<dyn FnOnce()>>>>,
) {
    hook.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn set_event_activity_gate_hooks(
    before: impl FnOnce() + 'static,
    after: impl FnOnce() + 'static,
) {
    BEFORE_EVENT_ACTIVITY_GATE.with(|slot| *slot.borrow_mut() = Some(Box::new(before)));
    AFTER_EVENT_ACTIVITY_GATE.with(|slot| *slot.borrow_mut() = Some(Box::new(after)));
}

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
        #[cfg(test)]
        run_event_activity_gate_hook(&BEFORE_EVENT_ACTIVITY_GATE);
        let activity_mutation = self.owned.begin_managed_activity_mutation();
        #[cfg(test)]
        run_event_activity_gate_hook(&AFTER_EVENT_ACTIVITY_GATE);
        let existing_receipt = self
            .owned
            .session_store
            .get_session(&session_id)?
            .workflow_event_delivery_receipts()
            .get(&delivery.delivery_id)
            .cloned();
        if let Some(receipt) = existing_receipt {
            drop(activity_mutation);
            return Ok(AcceptedWorkflowEventDelivery {
                delivery_id: delivery.delivery_id,
                queued_prompt_id: receipt.queued_prompt_id,
                duplicate: true,
                session: self.owned.session_snapshot(&session_id)?,
            });
        }

        // Keep the rollback snapshot raw: session_snapshot publishes a projection and may
        // perform activity capture, which must not nest inside this admission boundary.
        let before = self.owned.session_store.get_session(&session_id)?;
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
            Err(error) => {
                self.owned.session_store.write().restore_session(before);
                return Err(error);
            }
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
            .persist_workflow_runtime_session_with_activity_mutation_and_rollback(
                &session_id,
                "workflow_event_delivery_accepted",
                activity_mutation,
                {
                    let session_store = self.owned.session_store.clone();
                    move || session_store.write().restore_session(before)
                },
            )
        {
            return Err(error);
        }

        let dispatches = match self
            .owned
            .workflow_start_next_queued_prompt_for_response(&session_id)
        {
            Ok((_, dispatches)) => dispatches,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::runtime::router::CommandRouter;
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;
    use tokio::sync::Mutex;

    fn runtime_with_event_binding() -> (
        KernelRuntimeState,
        String,
        crate::session::WorkflowEventBinding,
    ) {
        let mut config = DaemonConfig::for_tests();
        config.publication_control_state_root = Some(std::env::temp_dir().join(format!(
            "chariox-event-admission-control-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        )));
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-event-admission",
                "worktree-event-admission",
            ))
            .expect("event admission session should create");
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("event-admission".to_string()))
            .expect("event workflow should create");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("event workflow node should create");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("event-entry".to_string()),
            )
            .expect("event workflow endpoint should create");
        let publication = app
            .sessions_mut()
            .create_workflow_publication(
                session.id(),
                workflow.id(),
                endpoint.id(),
                Some("default".to_string()),
                Some("event-publication".to_string()),
                Some(crate::session::WORKFLOW_PUBLICATION_KIND_EVENT_BASED.to_string()),
                None,
                Vec::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            )
            .expect("event publication should create");
        let binding = app
            .sessions_mut()
            .create_workflow_event_binding(
                session.id(),
                publication.id(),
                "dev.chariox.test".to_string(),
                "1.0.0".to_string(),
                "event-admission-manifest".to_string(),
                "event-admission-connection".to_string(),
                "tenant:event-admission".to_string(),
                "test.event".to_string(),
                1,
                serde_json::json!({"scope": "event-admission"}),
                Some("event-admission-environment".to_string()),
                Some("default".to_string()),
                None,
                Vec::new(),
            )
            .expect("event binding should create");
        let runtime = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1)
            .runtime_state();
        (runtime, session.id().to_string(), binding)
    }

    fn delivery(
        binding: &crate::session::WorkflowEventBinding,
        delivery_id: &str,
    ) -> chariox_event_protocol::EventDeliveryEnvelope {
        chariox_event_protocol::EventDeliveryEnvelope {
            delivery_id: delivery_id.to_string(),
            binding_id: binding.id.clone(),
            event_type: binding.event_type.clone(),
            event_type_version: binding.event_type_version,
            occurrence_id: format!("occurrence-{delivery_id}"),
            occurred_at: chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            prompt: "Process the gated event.".to_string(),
            artifacts: Vec::new(),
            metadata: serde_json::json!({"test": "activity-admission"}),
            reply_context: None,
            expires_at_ms: u64::MAX,
        }
    }

    fn assert_no_event_work(
        runtime: &KernelRuntimeState,
        session_id: &str,
    ) {
        let session = runtime
            .owned
            .session_store
            .get_session(session_id)
            .expect("event session should remain available");
        assert!(session.workflow_queued_prompts().is_empty());
        assert!(session.workflow_event_delivery_receipts().is_empty());
        assert!(!session.has_active_workflow_run());
    }

    fn install_workflow_append_failure(runtime: &KernelRuntimeState) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(runtime.owned.durable_state_store.path())
            .expect("durable database should open for event failure injection");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_event_workflow_runtime_append
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'workflow.runtime.updated'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected event workflow append failure');
                 END;",
            )
            .expect("event workflow append failure trigger should install");
        connection
    }

    #[tokio::test]
    async fn event_delivery_blocks_at_activity_gate_before_enqueue_or_receipt() {
        let (runtime, session_id, binding) = runtime_with_event_binding();
        runtime
            .ensure_managed_activity_tracking("kernel-event-entry-gate")
            .expect("managed activity tracking should activate");

        let (locked_tx, locked_rx) = mpsc::sync_channel(0);
        let (before_tx, before_rx) = mpsc::sync_channel(0);
        let (after_tx, after_rx) = mpsc::sync_channel(0);
        let activity_lock = Arc::clone(&runtime.owned.managed_activity_mutation_lock);
        let observed_runtime = runtime.clone();
        let observed_session_id = session_id.clone();
        let holder = std::thread::spawn(move || {
            let guard = activity_lock
                .lock()
                .expect("managed activity mutation mutex should lock");
            locked_tx
                .send(())
                .expect("gate ownership should be observable");
            before_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("event admission should reach the gate");
            assert_no_event_work(&observed_runtime, &observed_session_id);
            assert!(matches!(
                after_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            assert_no_event_work(&observed_runtime, &observed_session_id);
            drop(guard);
            after_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("event admission should acquire the released gate");
        });
        locked_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test should own the activity gate");
        set_event_activity_gate_hooks(
            move || before_tx.send(()).expect("before-gate hook should be observed"),
            move || after_tx.send(()).expect("after-gate hook should be observed"),
        );

        let accepted = runtime
            .accept_workflow_event_delivery(delivery(&binding, "delivery-entry-gate"))
            .expect("event should admit after the gate is released");
        holder.join().expect("gate holder should finish cleanly");

        assert!(!accepted.duplicate);
        assert_eq!(
            runtime
                .owned
                .session_store
                .get_session(&session_id)
                .expect("event session should remain available")
                .workflow_event_delivery_receipts()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn concurrent_duplicate_event_delivery_enqueues_once_under_activity_gate() {
        let (runtime, session_id, binding) = runtime_with_event_binding();
        let barrier = Arc::new(Barrier::new(2));
        let envelope = delivery(&binding, "delivery-concurrent-duplicate");
        let mut threads = Vec::new();
        for _ in 0..2 {
            let runtime = runtime.clone();
            let barrier = Arc::clone(&barrier);
            let envelope = envelope.clone();
            threads.push(std::thread::spawn(move || {
                set_event_activity_gate_hooks(
                    move || {
                        barrier.wait();
                    },
                    || {},
                );
                runtime.accept_workflow_event_delivery(envelope)
            }));
        }
        let outcomes = threads
            .into_iter()
            .map(|thread| {
                thread
                    .join()
                    .expect("duplicate delivery thread should finish")
                    .expect("duplicate delivery should resolve")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.duplicate).count(),
            1
        );
        assert_eq!(outcomes[0].queued_prompt_id, outcomes[1].queued_prompt_id);
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("event session should remain available");
        assert_eq!(session.workflow_event_delivery_receipts().len(), 1);
        assert_eq!(session.workflow_queued_prompts().len(), 1);
        assert_eq!(
            session
                .workflow_event_delivery_receipts()
                .get("delivery-concurrent-duplicate")
                .expect("accepted delivery receipt should remain")
                .queued_prompt_id,
            session.workflow_queued_prompts()[0].id()
        );
    }

    #[tokio::test]
    async fn failed_event_delivery_append_rolls_back_before_activity_capture_and_retries() {
        let (runtime, session_id, binding) = runtime_with_event_binding();
        runtime
            .ensure_managed_activity_tracking("kernel-event-append-failure")
            .expect("managed activity tracking should activate");
        let (sequence_before, observation_before) = runtime
            .managed_activity_report_snapshot()
            .expect("initial idle activity should be durable");
        let connection = install_workflow_append_failure(&runtime);
        let envelope = delivery(&binding, "delivery-append-failure");

        let error = runtime
            .accept_workflow_event_delivery(envelope.clone())
            .expect_err("durable append failure should reject event admission");
        assert!(error
            .to_string()
            .contains("injected event workflow append failure"));
        assert_no_event_work(&runtime, &session_id);
        let (sequence_after, observation_after) = runtime
            .managed_activity_report_snapshot()
            .expect("rolled-back activity should remain readable");
        assert_eq!(sequence_after, sequence_before);
        assert_eq!(observation_after, observation_before);

        connection
            .execute_batch("DROP TRIGGER fail_event_workflow_runtime_append;")
            .expect("event workflow append failure trigger should be removed");
        let accepted = runtime
            .accept_workflow_event_delivery(envelope)
            .expect("rolled-back event delivery should retry as new");
        assert!(!accepted.duplicate);
    }
}
