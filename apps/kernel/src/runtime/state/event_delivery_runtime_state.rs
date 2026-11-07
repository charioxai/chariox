use super::*;

/// What one AEGS reconciliation claims: a generator's subscriptions under one
/// event owner, named by the connection records that carry that owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EventClaimSet {
    pub(crate) generator_id: String,
    pub(crate) owner_id: String,
}

impl KernelRuntimeState {
    /// Err when the App routes could not be read: the caller skips this round
    /// rather than publish an authoritative set without them.
    pub(crate) fn event_generator_subscription_claims(
        &self,
        kernel_id: &str,
        environment_id: &str,
    ) -> Result<BTreeMap<EventClaimSet, Vec<chariox_event_protocol::AegsSubscriptionClaim>>, String>
    {
        // One claim set per generator and record owner, each reconciled under
        // that owner; a record no route uses still gets its (empty) set so a
        // removed route stops being delivered.
        let mut sets =
            BTreeMap::<EventClaimSet, Vec<chariox_event_protocol::AegsSubscriptionClaim>>::new();
        for (generator_id, owner_id) in self
            .event_connection_registry()
            .claimable_generator_owners()
            .map_err(|error| format!("event connection owners could not be read: {error}"))?
        {
            sets.entry(EventClaimSet {
                generator_id,
                owner_id,
            })
            .or_default();
        }
        for (route, event_owner_id) in self.bound_app_event_routes()? {
            let binding_id =
                delivery_binding_id(&route, &event_owner_id, kernel_id, environment_id);
            let Some(claim) = app_route_subscription(&route, binding_id) else {
                continue;
            };
            sets.entry(EventClaimSet {
                generator_id: claim.generator_id.clone(),
                owner_id: event_owner_id,
            })
            .or_default()
            .push(claim);
        }
        for claims in sets.values_mut() {
            claims.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        }
        Ok(sets)
    }

    /// App inbox routes fed by generator connections (protocol 358), of
    /// active installations only.
    fn app_event_routes(&self) -> Result<Vec<chariox_app_runtime::app_inbox::InboxRoute>, String> {
        self.owned
            .durable_state_store
            .app_generator_routes()
            .map_err(|error| format!("App event routes could not be read: {error}"))
    }

    /// The App event routes whose connection this kernel holds a record of,
    /// each with the owner id that record carries. A route whose record is
    /// gone has no owner to name it by, so it is not claimed: the user
    /// connects again.
    fn bound_app_event_routes(
        &self,
    ) -> Result<Vec<(chariox_app_runtime::app_inbox::InboxRoute, String)>, String> {
        let registry = self.event_connection_registry();
        let config = self.owned.config_projection.snapshot();
        let mut bound = Vec::new();
        for route in self.app_event_routes()? {
            let Some(source) = route.source.as_ref() else {
                continue;
            };
            let owner = registry
                .claimable_event_owner_id(
                    &crate::event_connection::event_connection_user_id(&config, &route.owner_id),
                    &source.connection_id,
                )
                .map_err(|error| format!("event connection owners could not be read: {error}"))?;
            if let Some(owner) = owner {
                bound.push((route, owner));
            }
        }
        Ok(bound)
    }

    /// Active App inbox routes fed by an event generator; 0, with a warning,
    /// when they cannot be read.
    pub(crate) fn active_app_route_count(&self) -> usize {
        self.app_event_routes()
            .unwrap_or_else(|error| {
                crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "App event routes could not be read for the delivery status",
                    serde_json::json!({ "error": error }),
                );
                Vec::new()
            })
            .iter()
            .filter_map(|route| app_route_subscription(route, route.binding_id()))
            .filter(|claim| claim.active)
            .count()
    }

    /// The App inbox routes and connection grants that use one of the
    /// person's event connections. `owner_id` is the record owner; routes and
    /// grants keep the owner that created them, so both are compared mapped.
    pub(crate) fn event_connection_dependencies(
        &self,
        owner_id: &str,
        connection_id: &str,
    ) -> Result<Vec<crate::local::EventConnectionDependency>, String> {
        let config = self.owned.config_projection.snapshot();
        let is_record_owner = |app_owner: &str| {
            crate::event_connection::event_connection_user_id(&config, app_owner) == owner_id
        };
        let mut dependencies = self
            .app_event_routes()?
            .into_iter()
            .filter(|route| {
                is_record_owner(&route.owner_id)
                    && route
                        .source
                        .as_ref()
                        .is_some_and(|source| source.connection_id == connection_id)
            })
            .map(|route| crate::local::EventConnectionDependency {
                installation_id: route.installation_id,
                route_id: Some(route.route_id),
                active: route.active,
            })
            .collect::<Vec<_>>();
        let grantees = self
            .owned
            .durable_state_store
            .app_connection_grantees(connection_id)
            .map_err(|code| format!("App connection grants could not be read: {code}"))?;
        dependencies.extend(
            grantees
                .into_iter()
                .filter(|(grant_owner, _)| is_record_owner(grant_owner))
                .map(
                    |(_, installation_id)| crate::local::EventConnectionDependency {
                        installation_id,
                        route_id: None,
                        active: true,
                    },
                ),
        );
        Ok(dependencies)
    }

    /// App routes deliver in the kernel's default environment, the only one
    /// this kernel resumes.
    pub(crate) fn event_delivery_resumes(
        &self,
        kernel_id: &str,
        default_environment_id: &str,
    ) -> Result<Vec<chariox_event_protocol::KernelEnvironmentResume>, String> {
        let routes = self
            .bound_app_event_routes()?
            .iter()
            .filter_map(|(route, event_owner_id)| {
                app_route_claim(route, event_owner_id, kernel_id, default_environment_id)
            })
            .collect();
        Ok(vec![chariox_event_protocol::KernelEnvironmentResume {
            environment_id: default_environment_id.to_owned(),
            last_accepted_delivery_id: None,
            routes,
        }])
    }

    /// An App route has no status to mark; its owner sees no deliveries, and
    /// the log names the route that holds the interest.
    pub(crate) fn apply_event_route_conflicts(
        &self,
        conflicts: &[chariox_event_protocol::EventRouteConflict],
    ) {
        for conflict in conflicts {
            crate::logging::warn_with_fields(
                "daemon.event_delivery",
                "event route conflicts with another route",
                serde_json::json!({ "conflict": conflict }),
            );
        }
    }

    /// An AEDS delivery for an App inbox route. Protocol 365 retired direct
    /// workflow event bindings: a delivery for any other route can never land,
    /// so it is logged and acknowledged rather than retried.
    pub(crate) fn accept_event_delivery(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
        kernel_id: &str,
        environment_id: &str,
    ) -> Result<(), String> {
        if delivery.binding_id.starts_with("app-route-") {
            return self.accept_app_event_delivery(delivery, kernel_id, environment_id);
        }
        refused(&delivery, "direct workflow event bindings are retired")
    }

    /// Protocol 358: an occurrence for an App inbox route. Returns Ok when
    /// the source may be acknowledged: the occurrence is durably in the
    /// App's inbox (or already was), or it can never be (the route is gone,
    /// or the payload does not match the App's signed schema); Err leaves it
    /// unacknowledged so the event service retries.
    pub(crate) fn accept_app_event_delivery(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
        kernel_id: &str,
        environment_id: &str,
    ) -> Result<(), String> {
        delivery.validate(crate::session::unix_epoch_ms())?;
        let Some((route, _)) =
            self.bound_app_event_routes()?
                .into_iter()
                .find(|(route, event_owner_id)| {
                    delivery_binding_id(route, event_owner_id, kernel_id, environment_id)
                        == delivery.binding_id
                })
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
}

fn app_route_subscription(
    route: &chariox_app_runtime::app_inbox::InboxRoute,
    binding_id: String,
) -> Option<chariox_event_protocol::AegsSubscriptionClaim> {
    let source = route.source.as_ref()?;
    let filter: serde_json::Value = serde_json::from_str(&source.filter_json).ok()?;
    Some(chariox_event_protocol::AegsSubscriptionClaim {
        binding_id,
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

/// The binding id AEGS subscriptions and AEDS routes name this kernel's copy of
/// an App route by. AEDS keys routes by binding id alone and refuses a second
/// kernel's claim of the same one, so the id includes the kernel and
/// environment: kernels that hold copies of one route, with the same owner,
/// installation and route ids, each get their own delivery of an occurrence.
/// It also includes the owner id of the route's connection, which only the
/// kernels holding that connection's record know: a kernel that learns the
/// binding ids of another user's routes still cannot compute one.
fn delivery_binding_id(
    route: &chariox_app_runtime::app_inbox::InboxRoute,
    event_owner_id: &str,
    kernel_id: &str,
    environment_id: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(
        format!(
            "{}\0{event_owner_id}\0{kernel_id}\0{environment_id}",
            route.binding_id()
        )
        .as_bytes(),
    );
    format!("app-route-{}", &format!("{digest:x}")[..40])
}

/// AEDS routes by binding: the App route's installation and route stand in
/// for a workflow's publication and endpoint.
fn app_route_claim(
    route: &chariox_app_runtime::app_inbox::InboxRoute,
    event_owner_id: &str,
    kernel_id: &str,
    environment_id: &str,
) -> Option<chariox_event_protocol::EnvironmentRouteClaim> {
    let binding_id = delivery_binding_id(route, event_owner_id, kernel_id, environment_id);
    let subscription = app_route_subscription(route, binding_id.clone())?;
    Some(chariox_event_protocol::EnvironmentRouteClaim {
        environment_id: environment_id.to_owned(),
        event_interest_key: subscription.event_interest_key,
        kernel_id: kernel_id.to_owned(),
        publication_id: format!("app-installation-{}", route.installation_id),
        binding_id,
        endpoint_id: route.route_id.clone(),
        queue_ref: None,
        binding_revision: 1,
        active: route.active,
    })
}

/// An occurrence no retry can deliver: logged, then acknowledged.
fn refused(
    delivery: &chariox_event_protocol::EventDeliveryEnvelope,
    reason: &str,
) -> Result<(), String> {
    crate::logging::warn_with_fields(
        "daemon.event_delivery",
        "event delivery refused",
        serde_json::json!({
            "delivery_id": delivery.delivery_id,
            "binding_id": delivery.binding_id,
            "reason": reason,
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::runtime::router::CommandRouter;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn account_event_routes_have_distinct_bindings_per_environment_and_kernel() {
        let route = chariox_app_runtime::app_inbox::InboxRoute {
            route_id: "same-route".into(),
            owner_id: "same-user".into(),
            installation_id: "same-installation".into(),
            event_name: "received".into(),
            source_event_type: "dummy.test".into(),
            source_event_version: 1,
            active: true,
            source: Some(chariox_app_runtime::app_inbox::InboxSource {
                generator_id: "dev.chariox.dummy".into(),
                connection_id: "shared-connection".into(),
                connection_scope: "default".into(),
                filter_json: "null".into(),
            }),
        };
        let first = app_route_claim(&route, "owner-1", "kernel-a", "env-a").unwrap();
        let second = app_route_claim(&route, "owner-1", "kernel-b", "env-b").unwrap();
        assert_eq!(
            first.binding_id,
            "app-route-116662a2520eea96003ee8ac8fddaa195de53a0c"
        );
        assert_eq!(first.event_interest_key, second.event_interest_key);
        assert_ne!(first.binding_id, second.binding_id);
        assert_ne!(
            first.binding_id,
            app_route_claim(&route, "owner-1", "kernel-a", "env-b")
                .unwrap()
                .binding_id
        );
        assert_ne!(
            first.binding_id,
            app_route_claim(&route, "owner-1", "kernel-b", "env-a")
                .unwrap()
                .binding_id
        );
    }

    fn mention_route() -> chariox_app_runtime::app_inbox::InboxRoute {
        chariox_app_runtime::app_inbox::InboxRoute {
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
        }
    }

    const OWNER: &str = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";

    fn activate_mention_installation(runtime: &KernelRuntimeState) {
        rusqlite::Connection::open(runtime.owned.durable_state_store.path())
            .unwrap()
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('app_slack','dev.chariox.slack-app','local',1,1,'{}')",
            )
            .unwrap();
    }

    /// A kernel holding the mention route of an installation (not yet active) and,
    /// when given, the record of its connection under that event owner.
    fn runtime_with_mention_route(event_owner_id: Option<&str>) -> KernelRuntimeState {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
        runtime
            .owned
            .durable_state_store
            .app_inbox(
                crate::durable_state::app_inbox::AppInboxOperation::CreateRoute {
                    route: mention_route(),
                    now_ms: 1,
                },
            )
            .unwrap();
        if let Some(event_owner_id) = event_owner_id {
            runtime
                .event_connection_registry()
                .upsert(
                    "local",
                    event_owner_id,
                    chariox_event_protocol::AegsConnectionSummary {
                        generator_id: "dev.chariox.slack".into(),
                        connection_id: "connection-1".into(),
                        status: chariox_event_protocol::AegsConnectionStatus::Ready,
                        metadata: serde_json::json!({}),
                        expires_at_ms: None,
                        updated_at_ms: 1,
                    },
                )
                .unwrap();
        }
        runtime
    }

    /// The claims of one generator across the owners that carry them.
    fn claims_of<'a>(
        sets: &'a BTreeMap<EventClaimSet, Vec<chariox_event_protocol::AegsSubscriptionClaim>>,
        generator_id: &str,
    ) -> Vec<&'a chariox_event_protocol::AegsSubscriptionClaim> {
        sets.iter()
            .filter(|(set, _)| set.generator_id == generator_id)
            .flat_map(|(_, claims)| claims)
            .collect()
    }

    fn only_binding_id(runtime: &KernelRuntimeState, kernel_id: &str) -> String {
        let claims = runtime
            .event_generator_subscription_claims(kernel_id, "default")
            .unwrap();
        let claims = claims_of(&claims, "dev.chariox.slack");
        assert_eq!(claims.len(), 1);
        claims[0].binding_id.clone()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_binding_id_depends_on_the_connections_secret_owner_id() {
        let first = runtime_with_mention_route(Some(OWNER));
        let second = runtime_with_mention_route(Some("a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1"));
        activate_mention_installation(&first);
        activate_mention_installation(&second);
        // Same user, installation, route, kernel and environment: only the
        // owner the connection record carries differs. A kernel that knows
        // every other id cannot name this binding.
        assert_ne!(
            only_binding_id(&first, "kernel-1"),
            only_binding_id(&second, "kernel-1")
        );
        // A copy of the record on another kernel names its own binding.
        assert_ne!(
            only_binding_id(&first, "kernel-1"),
            only_binding_id(&first, "kernel-2")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_route_whose_connection_record_is_gone_is_not_claimed() {
        let runtime = runtime_with_mention_route(None);
        activate_mention_installation(&runtime);
        assert!(runtime
            .event_generator_subscription_claims("kernel-1", "default")
            .unwrap()
            .is_empty());
        let resumes = runtime
            .event_delivery_resumes("kernel-1", "default")
            .unwrap();
        assert!(resumes[0].routes.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn app_routes_from_generators_are_claimed_and_unroutable_deliveries_are_acknowledged() {
        let runtime = runtime_with_mention_route(Some(OWNER));
        let route = mention_route();
        let binding_id = delivery_binding_id(&route, OWNER, "kernel-1", "default");
        // Only an active installation's routes are claimed.
        let before = runtime
            .event_generator_subscription_claims("kernel-1", "default")
            .unwrap();
        assert!(claims_of(&before, "dev.chariox.slack").is_empty());
        activate_mention_installation(&runtime);
        let claims = runtime
            .event_generator_subscription_claims("kernel-1", "default")
            .unwrap();
        let claim = claims_of(&claims, "dev.chariox.slack")[0];
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
            (
                routes[0].binding_id.as_str(),
                routes[0].endpoint_id.as_str()
            ),
            (
                delivery_binding_id(&route, OWNER, "kernel-1", "default").as_str(),
                "mentions"
            )
        );
        let claim_binding_id = routes[0].binding_id.clone();
        assert_eq!(routes[0].event_interest_key, claim.event_interest_key);
        assert_eq!(runtime.active_app_route_count(), 1);
        // A delivery that can never land (unknown route, wrong event type, a
        // retired direct workflow binding) is acknowledged, not retried
        // forever; the App is never reached.
        let delivery =
            |binding_id: &str, event_type: &str| chariox_event_protocol::EventDeliveryEnvelope {
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
            };
        let accepted = tokio::task::spawn_blocking({
            let runtime = runtime.clone();
            move || {
                (
                    runtime.accept_app_event_delivery(
                        delivery("app-route-missing", "app.mentioned"),
                        "kernel-1",
                        "default",
                    ),
                    runtime.accept_app_event_delivery(
                        delivery(&claim_binding_id, "reaction.added"),
                        "kernel-1",
                        "default",
                    ),
                    runtime.accept_event_delivery(
                        delivery("binding-retired", "app.mentioned"),
                        "kernel-1",
                        "default",
                    ),
                )
            }
        })
        .await
        .unwrap();
        assert_eq!(accepted, (Ok(()), Ok(()), Ok(())));
    }
}

#[cfg(test)]
mod owner_mapping_tests {
    use super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app;
    use super::*;

    // The connection record of the person operating the kernel is kept under `local`, whoever
    // connected (a Cloud client or the CLI); the App routes and grants keep the owner that
    // created them. Counting a connection's dependencies must see them all.
    fn cloud_owner_runtime() -> (KernelRuntimeState, String) {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            user_id: "cloud-owner".to_string(),
            ..Default::default()
        });
        let app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let store = app.durable_state_store();
        let state = runtime_state_from_app(app);
        let record_owner =
            crate::event_connection::event_connection_user_id(&config, "cloud-owner");
        for (installation, owner) in [
            ("app", "cloud-owner"),
            ("local-app", "local"),
            ("other-app", "someone-else"),
        ] {
            rusqlite::Connection::open(store.path())
                .unwrap()
                .execute(
                    "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                     VALUES(?1,'dev.chariox.app',?2,1,1,'{}')",
                    rusqlite::params![installation, owner],
                )
                .unwrap();
        }
        state
            .event_connection_registry()
            .upsert(
                &record_owner,
                "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.up".into(),
                    connection_id: "connection-1".into(),
                    status: chariox_event_protocol::AegsConnectionStatus::Ready,
                    metadata: serde_json::json!({}),
                    expires_at_ms: None,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
        (state, record_owner)
    }

    fn create_route(state: &KernelRuntimeState, route_id: &str, owner: &str, installation: &str) {
        state
            .owned
            .durable_state_store
            .app_inbox(
                crate::durable_state::app_inbox::AppInboxOperation::CreateRoute {
                    route: chariox_app_runtime::app_inbox::InboxRoute {
                        route_id: route_id.into(),
                        owner_id: owner.into(),
                        installation_id: installation.into(),
                        event_name: "received".into(),
                        source_event_type: "thing.happened".into(),
                        source_event_version: 1,
                        active: true,
                        source: Some(chariox_app_runtime::app_inbox::InboxSource {
                            generator_id: "dev.chariox.up".into(),
                            connection_id: "connection-1".into(),
                            connection_scope: "scope".into(),
                            filter_json: "null".into(),
                        }),
                    },
                    now_ms: 1,
                },
            )
            .unwrap();
    }

    fn grant(state: &KernelRuntimeState, owner: &str, installation: &str) {
        state
            .owned
            .durable_state_store
            .app_connection_grant(
                crate::durable_state::app_connections::ConnectionGrantCommand::Grant {
                    owner: owner.into(),
                    installation: installation.into(),
                    generator_id: "dev.chariox.up".into(),
                    connection_id: "connection-1".into(),
                    event_owner_id: "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f".into(),
                    now_ms: 1,
                },
            )
            .unwrap();
    }

    fn dependency_installations(state: &KernelRuntimeState, record_owner: &str) -> Vec<String> {
        let mut installations = state
            .event_connection_dependencies(record_owner, "connection-1")
            .unwrap()
            .into_iter()
            .map(|dependency| dependency.installation_id)
            .collect::<Vec<_>>();
        installations.sort();
        installations
    }

    #[tokio::test]
    async fn a_route_of_the_cloud_owner_is_a_dependency_of_the_connection_the_cli_sees() {
        let (state, record_owner) = cloud_owner_runtime();
        create_route(&state, "route-1", "cloud-owner", "app");
        // The claim set sees the route under the record's owner ...
        assert_eq!(state.bound_app_event_routes().unwrap().len(), 1);
        // ... and removing the connection must see it too.
        let dependencies = dependency_installations(&state, &record_owner);
        assert!(
            dependencies.len() == 1,
            "the connection is claimed for the route but reports {} dependencies",
            dependencies.len()
        );
    }

    #[tokio::test]
    async fn a_grant_of_the_cloud_owner_is_a_dependency_of_the_connection_the_cli_sees() {
        let (state, record_owner) = cloud_owner_runtime();
        grant(&state, "cloud-owner", "app");
        assert_eq!(dependency_installations(&state, &record_owner), ["app"]);
    }

    #[tokio::test]
    async fn the_routes_and_grants_of_both_spellings_count_and_other_users_do_not() {
        let (state, record_owner) = cloud_owner_runtime();
        create_route(&state, "route-1", "cloud-owner", "app");
        create_route(&state, "route-2", "local", "local-app");
        grant(&state, "local", "local-app");
        grant(&state, "cloud-owner", "app");
        create_route(&state, "route-3", "someone-else", "other-app");
        grant(&state, "someone-else", "other-app");
        assert_eq!(
            dependency_installations(&state, &record_owner),
            ["app", "app", "local-app", "local-app"]
        );
    }

    async fn catalog_request(
        state: &KernelRuntimeState,
        caller: &str,
        request: crate::local::LocalDaemonRequest,
    ) -> Result<crate::local::LocalDaemonResponse, crate::error::DaemonError> {
        crate::runtime::event_catalog_control::execute_event_catalog_request_with_client(
            state,
            &state.owned.config_projection,
            &crate::runtime::event_catalog_control::AegsManagementHttpClient::default(),
            caller,
            request,
        )
        .await
    }

    // The CLI and the Cloud owner reach the same connection; the dependency list and the removal
    // guard answer the same for both.
    #[tokio::test]
    async fn the_cloud_owner_lists_and_is_guarded_by_the_dependencies_of_the_connection() {
        let (state, _) = cloud_owner_runtime();
        create_route(&state, "route-1", "cloud-owner", "app");
        grant(&state, "cloud-owner", "app");
        for caller in ["local", "cloud-owner"] {
            let listed = catalog_request(
                &state,
                caller,
                crate::local::LocalDaemonRequest::ListEventConnectionDependencies(
                    crate::local::ListEventConnectionDependenciesRequest {
                        connection_id: "connection-1".into(),
                    },
                ),
            )
            .await
            .unwrap();
            let crate::local::LocalDaemonResponse::EventConnectionDependencies {
                dependencies, ..
            } = listed
            else {
                panic!("unexpected response: {listed:?}");
            };
            assert!(
                dependencies.len() == 2,
                "{caller} sees {} dependencies",
                dependencies.len()
            );
            let removal = catalog_request(
                &state,
                caller,
                crate::local::LocalDaemonRequest::RemoveEventConnection(
                    crate::local::RemoveEventConnectionRequest {
                        connection_id: "connection-1".into(),
                        confirm: true,
                    },
                ),
            )
            .await
            .expect_err("an App still uses the connection");
            assert!(
                removal.to_string().contains("remove the App inbox routes"),
                "{caller}: {removal}"
            );
        }
    }

    // A grant or route request of the Cloud owner finds the connection the CLI made: the
    // record is looked up under the mapped owner, so the request goes on to ask the generator
    // instead of being told the connection does not exist.
    #[test]
    fn the_cloud_owner_finds_the_connection_when_granting_an_app() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            user_id: "cloud-owner".to_string(),
            ..Default::default()
        });
        let harness =
            crate::local::test_support::LocalRouterTestHarness::with_config(config.clone());
        harness
            .runtime_state()
            .event_connection_registry()
            .upsert(
                &crate::event_connection::event_connection_user_id(&config, "cloud-owner"),
                "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f",
                chariox_event_protocol::AegsConnectionSummary {
                    generator_id: "dev.chariox.up".into(),
                    connection_id: "connection-1".into(),
                    status: chariox_event_protocol::AegsConnectionStatus::Ready,
                    metadata: serde_json::json!({}),
                    expires_at_ms: None,
                    updated_at_ms: 1,
                },
            )
            .unwrap();
        let error = harness
            .dispatch_as_user(
                "cloud-owner",
                crate::local::LocalDaemonRequest::GrantAppConnection(
                    crate::local::GrantAppConnectionRequest {
                        installation_id: "app".into(),
                        generator_id: "dev.chariox.up".into(),
                        connection_id: "connection-1".into(),
                    },
                ),
            )
            .expect_err("no generator answers in this test");
        assert!(
            !error.to_string().contains("was not found"),
            "the connection of the person operating the kernel was not found: {error}"
        );
    }
}
