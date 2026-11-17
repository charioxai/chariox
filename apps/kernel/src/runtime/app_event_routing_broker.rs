//! Protocol 1011 SDK calls (`routing.*`): an App whose signed manifest declares
//! `capabilities.eventRouting` manages its own inbox routes and event targets
//! inside the one connection its owner granted. This file only parses the
//! call, names whom it acts for from the kernel's own caller record, and maps
//! errors; the checks and effects are `KernelRuntimeState::app_routing_*`.
//!
//! - `routing.ensure_route {routeId, event, repository, sourceEventType, sourceEventVersion}`
//! - `routing.remove_route {routeId}`, `routing.set_route_active {routeId, active}`, `routing.list_routes {}`
//! - `routing.connection_status {}`, `routing.connection_resources {query?, cursor?, limit}`,
//!   `routing.start_authorization {returnUrl?}`
//! - `routing.bind_agent {event, agentId?}`, `routing.bind_workflow {event, sessionId,
//!   publicationRef, queueRef?}`, `routing.unbind_target {automationId}`, `routing.list_targets {}`
use super::app_operation_budget::AppOperationBudget;
use super::event_catalog_control::AegsManagementHttpClient;
use super::state::{
    Acting, ConnectionView, KernelRuntimeState, RouteSpec, RouteView, RoutingError, RoutingScope,
    TargetView,
};
use chariox_app_package::EventRouting;
use chariox_app_runtime::{wire::RemoteError, worker_peer::BrokerRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// What the broker reaches the kernel through. A router owns its host; the
/// App lifecycle, which lives inside the app the host's state also owns, keeps
/// only weak handles, so a stopped kernel is still released.
pub(crate) struct RoutingHost {
    pub(crate) state: KernelRuntimeState,
    pub(crate) management: AegsManagementHttpClient,
}

#[derive(Clone, Default)]
pub(crate) struct RoutingHostSlot(Arc<Mutex<Vec<Weak<RoutingHost>>>>);

impl RoutingHostSlot {
    pub(crate) fn attach(&self, host: &Arc<RoutingHost>) {
        let mut hosts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        hosts.retain(|weak| weak.strong_count() > 0);
        hosts.push(Arc::downgrade(host));
    }

    /// Any live router's host: they all act on the same kernel.
    fn get(&self) -> Option<Arc<RoutingHost>> {
        let hosts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        hosts.iter().find_map(Weak::upgrade)
    }
}

#[derive(Clone)]
pub(crate) struct AppEventRoutingBroker {
    owner: String,
    installation: String,
    declared: Option<EventRouting>,
    host: RoutingHostSlot,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnsureRoute {
    route_id: String,
    event: String,
    repository: String,
    source_event_type: String,
    source_event_version: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteId {
    route_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetRouteActive {
    route_id: String,
    active: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resources {
    query: Option<String>,
    cursor: Option<String>,
    limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Authorization {
    return_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindAgent {
    event: String,
    agent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindWorkflow {
    event: String,
    session_id: String,
    publication_ref: String,
    queue_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Unbind {
    automation_id: String,
}

impl AppEventRoutingBroker {
    pub(crate) fn new(
        owner: String,
        installation: String,
        declared: Option<EventRouting>,
        host: RoutingHostSlot,
    ) -> Self {
        Self {
            owner,
            installation,
            declared,
            host,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        budget
            .check()
            .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
        // An App that did not declare the capability has no such import; the
        // call can only come from a forged peer.
        let declared = self
            .declared
            .clone()
            .ok_or_else(|| error("METHOD_UNAVAILABLE", false))?;
        let host = self
            .host
            .get()
            .ok_or_else(|| error("CONNECTION_UNAVAILABLE", true))?;
        let scope = RoutingScope {
            owner: self.owner.clone(),
            installation: self.installation.clone(),
            declared,
        };
        let deadline = request.deadline;
        let callers = request.callers;
        let run = call(&host, &scope, &request.method, request.params, &callers);
        match tokio::time::timeout_at(deadline, run).await {
            Ok(result) => result.map_err(|failure| error(failure.code(), failure.retryable())),
            Err(_) => Err(error("APP_OPERATION_STOPPED", false)),
        }
    }
}

async fn call(
    host: &RoutingHost,
    scope: &RoutingScope,
    method: &str,
    params: Value,
    callers: &[Value],
) -> Result<Value, RoutingError> {
    let state = &host.state;
    match method {
        "routing.ensure_route" => {
            let input: EnsureRoute = input(params)?;
            let view = state
                .app_routing_ensure_route(
                    &host.management,
                    scope,
                    RouteSpec {
                        route_id: input.route_id,
                        event: input.event,
                        repository: input.repository,
                        source_event_type: input.source_event_type,
                        source_event_version: input.source_event_version,
                    },
                )
                .await?;
            Ok(route(&view))
        }
        "routing.remove_route" => {
            let input: RouteId = input(params)?;
            state
                .app_routing_remove_route(scope, &input.route_id)
                .await?;
            Ok(json!({}))
        }
        "routing.set_route_active" => {
            let input: SetRouteActive = input(params)?;
            let view = state
                .app_routing_set_route_active(scope, &input.route_id, input.active)
                .await?;
            Ok(route(&view))
        }
        "routing.list_routes" => {
            let routes = state.app_routing_list_routes(scope).await?;
            Ok(json!({"routes": routes.iter().map(route).collect::<Vec<_>>()}))
        }
        "routing.connection_status" => Ok(connection(
            &state.app_routing_connection_status(scope).await?,
        )),
        "routing.connection_resources" => {
            let input: Resources = input(params)?;
            let page = state
                .app_routing_connection_resources(
                    &host.management,
                    scope,
                    input.query,
                    input.cursor,
                    input.limit,
                )
                .await?;
            Ok(json!({"page": {
                "resources": page.resources.iter().map(|resource| json!({
                    "id": resource.id,
                    "name": resource.name,
                    "kind": resource.kind,
                    "connectionScope": resource.connection_scope,
                })).collect::<Vec<_>>(),
                "nextCursor": page.next_cursor,
            }}))
        }
        "routing.start_authorization" => {
            let input: Authorization = input(params)?;
            let (authorization_id, url) = state
                .app_routing_start_authorization(&host.management, scope, input.return_url)
                .await?;
            Ok(json!({"authorizationId": authorization_id, "url": url}))
        }
        "routing.bind_agent" => {
            let input: BindAgent = input(params)?;
            let target = state
                .app_routing_bind_agent(scope, &Acting::of(callers)?, input.event, input.agent_id)
                .await?;
            Ok(target_json(&target))
        }
        "routing.bind_workflow" => {
            let input: BindWorkflow = input(params)?;
            let target = state
                .app_routing_bind_workflow(
                    scope,
                    &Acting::of(callers)?,
                    input.event,
                    input.session_id,
                    input.publication_ref,
                    input.queue_ref,
                )
                .await?;
            Ok(target_json(&target))
        }
        "routing.unbind_target" => {
            let input: Unbind = input(params)?;
            state
                .app_routing_unbind_target(scope, &Acting::of(callers)?, &input.automation_id)
                .await?;
            Ok(json!({}))
        }
        "routing.list_targets" => {
            let targets = state.app_routing_list_targets(scope).await?;
            Ok(json!({"targets": targets.iter().map(target_json).collect::<Vec<_>>()}))
        }
        _ => Err(RoutingError::InvalidArgument),
    }
}

fn input<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, RoutingError> {
    serde_json::from_value(params).map_err(|_| RoutingError::InvalidArgument)
}

fn route(view: &RouteView) -> Value {
    json!({
        "routeId": view.route_id,
        "event": view.event,
        "repository": view.repository,
        "sourceEventType": view.source_event_type,
        "sourceEventVersion": view.source_event_version,
        "active": view.active,
        "pending": view.counts.pending,
        "delivered": view.counts.delivered,
        "failed": view.counts.failed,
        "expired": view.counts.expired,
    })
}

fn connection(view: &ConnectionView) -> Value {
    json!({
        "generatorId": view.generator_id,
        "connectionId": view.connection_id,
        "status": view.status,
        "maxRoutes": view.max_routes,
        "routes": view.routes,
    })
}

fn target_json(view: &TargetView) -> Value {
    json!({
        "automationId": view.automation_id,
        "event": view.event,
        "kind": view.kind,
        "agentId": view.agent_id,
        "state": view.state,
    })
}

fn error(code: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: code.to_ascii_lowercase().replace('_', " "),
        retryable: Some(retryable),
    }
}
