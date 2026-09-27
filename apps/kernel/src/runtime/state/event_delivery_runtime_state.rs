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
fn set_event_activity_gate_hooks(before: impl FnOnce() + 'static, after: impl FnOnce() + 'static) {
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
    /// Err when the App routes could not be read: the caller skips this round
    /// rather than publish an authoritative set without them.
    pub(crate) fn event_generator_subscription_claims(
        &self,
    ) -> Result<BTreeMap<String, Vec<chariox_event_protocol::AegsSubscriptionClaim>>, String> {
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
        for route in self.app_event_routes()? {
            let Some(claim) = app_route_subscription(&route) else {
                continue;
            };
            generators
                .entry(claim.generator_id.clone())
                .or_default()
                .push(claim);
        }
        for claims in generators.values_mut() {
            claims.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        }
        Ok(generators)
    }

    /// App inbox routes fed by generator connections (protocol 358), of
    /// active installations only.
    fn app_event_routes(&self) -> Result<Vec<chariox_app_runtime::app_inbox::InboxRoute>, String> {
        self.owned
            .durable_state_store
            .app_generator_routes()
            .map_err(|error| format!("App event routes could not be read: {error}"))
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
    ) -> Result<Vec<chariox_event_protocol::KernelEnvironmentResume>, String> {
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
        // App routes deliver in the kernel's default environment.
        let app_routes = self
            .app_event_routes()?
            .iter()
            .filter_map(|route| app_route_claim(route, kernel_id, default_environment_id))
            .collect::<Vec<_>>();
        environments
            .get_mut(default_environment_id)
            .expect("default environment")
            .0
            .extend(app_routes);
        Ok(environments
            .into_iter()
            .map(|(environment_id, (routes, last_delivery))| {
                chariox_event_protocol::KernelEnvironmentResume {
                    environment_id,
                    last_accepted_delivery_id: last_delivery.map(|(_, delivery_id)| delivery_id),
                    routes,
                }
            })
            .collect())
    }

    /// The active route, workflow binding or App route, that already claims
    /// `event_interest_key` in `environment_id` under another binding. The
    /// event service keeps only one route per interest.
    pub(crate) fn event_interest_claimed_by(
        &self,
        kernel_id: &str,
        environment_id: &str,
        binding_id: &str,
        event_interest_key: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .event_delivery_resumes(kernel_id, environment_id)?
            .into_iter()
            .filter(|resume| resume.environment_id == environment_id)
            .flat_map(|resume| resume.routes)
            .find(|claim| {
                claim.active
                    && claim.event_interest_key == event_interest_key
                    && claim.binding_id != binding_id
            })
            .map(|claim| claim.binding_id))
    }

    /// The active App route that receives `event_interest_key` (App routes
    /// live in the kernel's default environment).
    pub(crate) fn app_route_claiming(
        &self,
        event_interest_key: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .app_event_routes()?
            .iter()
            .filter_map(app_route_subscription)
            .find(|claim| claim.active && claim.event_interest_key == event_interest_key)
            .map(|claim| claim.binding_id))
    }

    pub(crate) fn apply_event_route_conflicts(
        &self,
        conflicts: &[chariox_event_protocol::EventRouteConflict],
    ) {
        let mut changed_session_ids = BTreeSet::new();
        for conflict in conflicts {
            if conflict.requested_binding_id.starts_with("app-route-") {
                // An App route has no status to mark; its owner sees no
                // deliveries, and the log names the route that holds the
                // interest.
                crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "App inbox route conflicts with another route",
                    serde_json::json!({ "conflict": conflict }),
                );
                continue;
            }
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

    /// An AEDS delivery, to the App route or workflow binding it names. A
    /// binding whose events moved to an App (paused while an App route
    /// receives its interest) takes no more deliveries: one it accepted before
    /// is a replay after a lost acknowledgement, and one it never accepted
    /// goes to the App's inbox, which keeps one copy per occurrence.
    pub(crate) fn accept_event_delivery(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
    ) -> Result<(), String> {
        if delivery.binding_id.starts_with("app-route-") {
            return self.accept_app_event_delivery(delivery);
        }
        let paused = self
            .owned
            .session_store
            .read()
            .find_workflow_event_binding(&delivery.binding_id)
            .filter(|(_, binding)| {
                binding.status == crate::session::WorkflowEventBindingStatus::Paused
            });
        if let Some((session_id, binding)) = paused {
            if let Some(route) = self.app_route_claiming(&binding.event_interest_key)? {
                let accepted = self
                    .owned
                    .session_store
                    .read()
                    .workflow_event_delivery_was_accepted(&session_id, &delivery.delivery_id)
                    .map_err(|error| error.to_string())?;
                if accepted {
                    return Ok(());
                }
                return self.accept_app_event_delivery(
                    chariox_event_protocol::EventDeliveryEnvelope {
                        binding_id: route,
                        ..delivery
                    },
                );
            }
        }
        self.accept_workflow_event_delivery(delivery)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Protocol 358: an occurrence for an App inbox route. Returns Ok when
    /// the source may be acknowledged: the occurrence is durably in the
    /// App's inbox (or already was), or it can never be (the route is gone,
    /// or the payload does not match the App's signed schema); Err leaves it
    /// unacknowledged so the event service retries.
    pub(crate) fn accept_app_event_delivery(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
    ) -> Result<(), String> {
        delivery.validate(crate::session::unix_epoch_ms())?;
        let Some(route) = self
            .owned
            .durable_state_store
            .app_route_by_binding(&delivery.binding_id)
            .map_err(|error| error.to_string())?
        else {
            return refused(&delivery, "the App inbox route no longer exists");
        };
        let Some(source) = route.source.as_ref() else {
            return refused(&delivery, "the App inbox route has no event source");
        };
        if route.source_event_type != delivery.event_type
            || route.source_event_version != delivery.event_type_version
        {
            return refused(&delivery, "the occurrence is not the route's event type");
        }
        let payload = serde_json::json!({
            "source": {
                "generator_id": source.generator_id,
                "connection_id": source.connection_id,
                "event_type": delivery.event_type,
                "event_type_version": delivery.event_type_version,
            },
            "occurred_at": delivery.occurred_at,
            "text": delivery.prompt,
            "metadata": delivery.metadata,
            "artifacts": delivery.artifacts,
            "reply_context": delivery.reply_context,
        });
        let accepted =
            tokio::runtime::Handle::current().block_on(self.accept_app_inbox_occurrence(
                &route.owner_id,
                &route.installation_id,
                &route.route_id,
                &delivery.occurrence_id,
                payload,
            ));
        use crate::local::AppRequestErrorCode::*;
        match accepted {
            Ok(_) => Ok(()),
            // Each code has more than one cause; the reason names them all.
            Err(NotFound) => refused(
                &delivery,
                "the App inbox route or its installation is no longer active",
            ),
            Err(InvalidRequest) => refused(
                &delivery,
                "the occurrence does not fit the App's incoming schema or inbox limits",
            ),
            Err(Conflict) => refused(
                &delivery,
                "the occurrence differs from an earlier one, or the App's release cannot be verified",
            ),
            Err(code) => Err(format!("App inbox did not accept the occurrence: {code:?}")),
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
        // Drop the write guard before a rejected mutation attempts rollback.
        let enqueue_result = self
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
            );
        let queued_prompt = match enqueue_result {
            Ok(prompt) => prompt,
            Err(error) => {
                self.owned.session_store.write().restore_session(before);
                return Err(error);
            }
        };
        let receipt_result = self
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
            );
        if let Err(error) = receipt_result {
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
                    move || {
                        session_store.write().restore_session(before);
                    }
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

fn app_route_subscription(
    route: &chariox_app_runtime::app_inbox::InboxRoute,
) -> Option<chariox_event_protocol::AegsSubscriptionClaim> {
    let source = route.source.as_ref()?;
    let filter: serde_json::Value = serde_json::from_str(&source.filter_json).ok()?;
    Some(chariox_event_protocol::AegsSubscriptionClaim {
        binding_id: route.binding_id(),
        generator_id: source.generator_id.clone(),
        connection_id: source.connection_id.clone(),
        connection_scope: source.connection_scope.clone(),
        event_interest_key: chariox_event_protocol::event_interest_key(
            &source.generator_id,
            &route.source_event_type,
            route.source_event_version,
            &source.connection_scope,
            &filter,
        )
        .ok()?,
        event_type: route.source_event_type.clone(),
        event_type_version: route.source_event_version,
        filter,
        revision: 1,
        active: route.active,
    })
}

/// AEDS routes by binding: the App route's installation and route stand in
/// for a workflow's publication and endpoint.
fn app_route_claim(
    route: &chariox_app_runtime::app_inbox::InboxRoute,
    kernel_id: &str,
    environment_id: &str,
) -> Option<chariox_event_protocol::EnvironmentRouteClaim> {
    let subscription = app_route_subscription(route)?;
    Some(chariox_event_protocol::EnvironmentRouteClaim {
        environment_id: environment_id.to_owned(),
        event_interest_key: subscription.event_interest_key,
        kernel_id: kernel_id.to_owned(),
        publication_id: format!("app-installation-{}", route.installation_id),
        binding_id: subscription.binding_id,
        endpoint_id: route.route_id.clone(),
        queue_ref: None,
        binding_revision: 1,
        active: route.active,
    })
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
            .create_workflow_publication_idempotent(
                session.id(),
                workflow.id(),
                endpoint.id(),
                None,
                None,
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
                vec![agent.clone()],
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
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
        (runtime, session.id().to_string(), binding)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn app_routes_from_generators_join_both_claim_sets_and_unroutable_deliveries_are_acknowledged(
    ) {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
        let route = chariox_app_runtime::app_inbox::InboxRoute {
            route_id: "mentions".into(),
            owner_id: "local".into(),
            installation_id: "app_slack".into(),
            event_name: "mentioned".into(),
            source_event_type: "app.mentioned".into(),
            source_event_version: 1,
            active: true,
            source: Some(chariox_app_runtime::app_inbox::InboxSource {
                generator_id: "dev.chariox.slack".into(),
                connection_id: "connection-1".into(),
                connection_scope: "team:T1".into(),
                filter_json: "null".into(),
            }),
        };
        runtime
            .owned
            .durable_state_store
            .app_inbox(
                crate::durable_state::app_inbox::AppInboxOperation::CreateRoute {
                    route: route.clone(),
                    now_ms: 1,
                },
            )
            .unwrap();
        let binding_id = route.binding_id();
        // Only an active installation's routes are claimed.
        assert!(runtime
            .event_generator_subscription_claims()
            .unwrap()
            .is_empty());
        rusqlite::Connection::open(runtime.owned.durable_state_store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('app_slack','dev.chariox.slack-app','local',1,1,'{}')",
            )
            .unwrap();
        let claims = runtime.event_generator_subscription_claims().unwrap();
        let claim = &claims["dev.chariox.slack"][0];
        assert_eq!(
            (
                claim.binding_id.as_str(),
                claim.connection_id.as_str(),
                claim.event_type.as_str(),
                claim.active
            ),
            (binding_id.as_str(), "connection-1", "app.mentioned", true)
        );
        let resumes = runtime
            .event_delivery_resumes("kernel-1", "default")
            .unwrap();
        let routes = &resumes
            .iter()
            .find(|resume| resume.environment_id == "default")
            .unwrap()
            .routes;
        assert_eq!(routes.len(), 1);
        assert_eq!(
            (routes[0].binding_id.as_str(), routes[0].endpoint_id.as_str()),
            (binding_id.as_str(), "mentions")
        );
        assert_eq!(routes[0].event_interest_key, claim.event_interest_key);
        // A second route for the same interest is told which route holds it;
        // the route itself is not a conflict with itself.
        let key = claim.event_interest_key.as_str();
        assert_eq!(
            runtime
                .event_interest_claimed_by("kernel-1", "default", "app-route-other", key)
                .unwrap(),
            Some(binding_id.clone())
        );
        assert_eq!(
            runtime
                .event_interest_claimed_by("kernel-1", "default", &binding_id, key)
                .unwrap(),
            None
        );
        // A delivery that can never land (unknown route, wrong event type) is
        // acknowledged, not retried forever; the App is never reached.
        let delivery = |binding_id: &str, event_type: &str| {
            chariox_event_protocol::EventDeliveryEnvelope {
                delivery_id: format!("delivery-{event_type}"),
                binding_id: binding_id.into(),
                event_type: event_type.into(),
                event_type_version: 1,
                occurrence_id: "occurrence-1".into(),
                occurred_at: "2026-09-26T00:00:00.000Z".into(),
                prompt: "hello".into(),
                artifacts: Vec::new(),
                metadata: serde_json::Value::Null,
                reply_context: None,
                expires_at_ms: u64::MAX,
            }
        };
        let accepted = tokio::task::spawn_blocking({
            let runtime = runtime.clone();
            move || {
                (
                    runtime.accept_app_event_delivery(delivery("app-route-missing", "app.mentioned")),
                    runtime.accept_app_event_delivery(delivery(&binding_id, "reaction.added")),
                )
            }
        })
        .await
        .unwrap();
        assert_eq!(accepted, (Ok(()), Ok(())));
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
            occurred_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            prompt: "Process the gated event.".to_string(),
            artifacts: Vec::new(),
            metadata: serde_json::json!({"test": "activity-admission"}),
            reply_context: None,
            expires_at_ms: u64::MAX,
        }
    }

    fn assert_no_event_work(runtime: &KernelRuntimeState, session_id: &str) {
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
            move || {
                before_tx
                    .send(())
                    .expect("before-gate hook should be observed")
            },
            move || {
                after_tx
                    .send(())
                    .expect("after-gate hook should be observed")
            },
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
    async fn rejected_event_enqueue_releases_session_guard_before_rollback() {
        let (runtime, session_id, binding) = runtime_with_event_binding();
        let mut session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .expect("event session should exist");
        session
            .workflow_event_binding_mut(&binding.id)
            .expect("event binding should exist")
            .queue_ref = Some("removed-event-queue".to_string());
        runtime.owned.session_store.restore_session(session);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let outcome = runtime
                .accept_workflow_event_delivery(delivery(&binding, "delivery-rejected-queue"));
            assert_no_event_work(&runtime, &session_id);
            result_tx
                .send(outcome)
                .expect("test should receive rejection");
        });
        let outcome = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("rejected enqueue must return without re-locking its session guard");
        worker
            .join()
            .expect("rejected delivery worker should finish");
        assert!(
            matches!(
                outcome,
                Err(DaemonError::InvalidWorkflowGraphReference { reference, .. })
                    if reference == "removed-event-queue"
            ),
            "removed queue must reject event admission"
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

/// An occurrence no retry can deliver: logged, then acknowledged.
fn refused(
    delivery: &chariox_event_protocol::EventDeliveryEnvelope,
    reason: &str,
) -> Result<(), String> {
    crate::logging::warn_with_fields(
        "daemon.event_delivery",
        "App event delivery refused",
        serde_json::json!({
            "delivery_id": delivery.delivery_id,
            "binding_id": delivery.binding_id,
            "reason": reason,
        }),
    );
    Ok(())
}
