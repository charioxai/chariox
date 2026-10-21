use super::*;

impl KernelRuntimeState {
    /// Err when the App routes could not be read: the caller skips this round
    /// rather than publish an authoritative set without them.
    pub(crate) fn event_generator_subscription_claims(
        &self,
    ) -> Result<BTreeMap<String, Vec<chariox_event_protocol::AegsSubscriptionClaim>>, String> {
        let mut generators =
            BTreeMap::<String, Vec<chariox_event_protocol::AegsSubscriptionClaim>>::new();
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
            .filter_map(app_route_subscription)
            .filter(|claim| claim.active)
            .count()
    }

    /// The App inbox routes and connection grants that use one of the
    /// owner's event connections.
    pub(crate) fn event_connection_dependencies(
        &self,
        owner_id: &str,
        connection_id: &str,
    ) -> Result<Vec<crate::local::EventConnectionDependency>, String> {
        let mut dependencies = self
            .app_event_routes()?
            .into_iter()
            .filter(|route| {
                route.owner_id == owner_id
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
            .app_connection_grantees(owner_id, connection_id)
            .map_err(|code| format!("App connection grants could not be read: {code}"))?;
        dependencies.extend(grantees.into_iter().map(|installation_id| {
            crate::local::EventConnectionDependency {
                installation_id,
                route_id: None,
                active: true,
            }
        }));
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
            .app_event_routes()?
            .iter()
            .filter_map(|route| app_route_claim(route, kernel_id, default_environment_id))
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
    ) -> Result<(), String> {
        if delivery.binding_id.starts_with("app-route-") {
            return self.accept_app_event_delivery(delivery);
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn app_routes_from_generators_are_claimed_and_unroutable_deliveries_are_acknowledged() {
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
            (
                routes[0].binding_id.as_str(),
                routes[0].endpoint_id.as_str()
            ),
            (binding_id.as_str(), "mentions")
        );
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
                    runtime
                        .accept_app_event_delivery(delivery("app-route-missing", "app.mentioned")),
                    runtime.accept_app_event_delivery(delivery(&binding_id, "reaction.added")),
                    runtime.accept_event_delivery(delivery("binding-retired", "app.mentioned")),
                )
            }
        })
        .await
        .unwrap();
        assert_eq!(accepted, (Ok(()), Ok(()), Ok(())));
    }
}
