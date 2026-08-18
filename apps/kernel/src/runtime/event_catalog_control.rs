// Catalog control is a kernel runtime boundary and consistently returns the shared `DaemonError`.
// Boxing only this module's results would introduce a second error contract for its callers.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::local::{
    EventCatalogCategory, EventCatalogFacet, EventCatalogFacetValue, EventConnectionPage,
    EventGeneratorCatalogDetail, EventGeneratorCatalogPage, EventGeneratorCatalogSummary,
    EventGeneratorEventDefinition, EventGeneratorEventPage, EventGeneratorParty,
    LocalDaemonRequest, LocalDaemonResponse, WorkflowEventBindingDependency,
};
use crate::runtime::aegs_network_policy::is_globally_reachable_aegs_destination;
use crate::runtime::cloud_api_client::issue_event_generator_management_capability;
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

const CATALOG_CACHE_FRESH_TTL: Duration = Duration::from_secs(60);
const CATALOG_CACHE_STALE_TTL: Duration = Duration::from_secs(5 * 60);
const CATALOG_CACHE_MAX_ENTRIES: usize = 128;
const CATALOG_RESPONSE_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const BUILTIN_DUMMY_MANIFEST_DIGEST: &str =
    "sha256:f40b9a94c3319565975f9d23c65a1796869b7083cdfa9f7d8d2d86ed590a15f5";

#[derive(Clone)]
struct CatalogCacheEntry {
    response: LocalDaemonResponse,
    stored_at: Instant,
}

static CATALOG_CACHE: OnceLock<Mutex<BTreeMap<String, CatalogCacheEntry>>> = OnceLock::new();

type AegsAgentBuilderFactory =
    dyn Fn(&crate::config::EventGeneratorManagementTarget) -> ureq::AgentBuilder + Send + Sync;

#[derive(Clone)]
pub(crate) struct AegsManagementHttpClient {
    agent_builder: Arc<AegsAgentBuilderFactory>,
}

impl Default for AegsManagementHttpClient {
    fn default() -> Self {
        Self {
            agent_builder: Arc::new(aegs_management_agent_builder),
        }
    }
}

impl AegsManagementHttpClient {
    fn agent_builder(
        &self,
        target: &crate::config::EventGeneratorManagementTarget,
    ) -> ureq::AgentBuilder {
        (self.agent_builder)(target)
    }

    #[cfg(test)]
    pub(crate) fn with_agent_builder(
        agent_builder: impl Fn(&crate::config::EventGeneratorManagementTarget) -> ureq::AgentBuilder
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            agent_builder: Arc::new(agent_builder),
        }
    }
}

pub(crate) struct WorkflowEventBindingContract {
    pub(crate) generator_id: String,
    pub(crate) generator_version: String,
    pub(crate) manifest_digest: String,
    pub(crate) connection_id: String,
    pub(crate) event_type: String,
    pub(crate) event_type_version: u32,
    pub(crate) action_ids: Vec<String>,
    pub(crate) reply_mode: Option<String>,
}

impl From<&crate::local::CreateWorkflowEventBindingRequest> for WorkflowEventBindingContract {
    fn from(request: &crate::local::CreateWorkflowEventBindingRequest) -> Self {
        Self {
            generator_id: request.generator_id.clone(),
            generator_version: request.generator_version.clone(),
            manifest_digest: request.manifest_digest.clone(),
            connection_id: request.connection_id.clone(),
            event_type: request.event_type.clone(),
            event_type_version: request.event_type_version,
            action_ids: request.action_ids.clone(),
            reply_mode: request.reply_mode.clone(),
        }
    }
}

impl From<&crate::session::WorkflowEventBinding> for WorkflowEventBindingContract {
    fn from(binding: &crate::session::WorkflowEventBinding) -> Self {
        Self {
            generator_id: binding.generator_id.clone(),
            generator_version: binding.generator_version.clone(),
            manifest_digest: binding.manifest_digest.clone(),
            connection_id: binding.connection_id.clone(),
            event_type: binding.event_type.clone(),
            event_type_version: binding.event_type_version,
            action_ids: binding.action_ids.clone(),
            reply_mode: binding.reply_mode.clone(),
        }
    }
}

impl WorkflowEventBindingContract {
    pub(crate) async fn required_scopes(
        &self,
        config_projection: &DaemonConfigProjectionStore,
    ) -> Result<Vec<String>, DaemonError> {
        validate_event_binding_contract(config_projection, self).await
    }
}

pub(crate) async fn execute_event_catalog_request_with_client(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    management_client: &AegsManagementHttpClient,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = config_projection.snapshot();
    if matches!(request, LocalDaemonRequest::GetEventDeliveryStatus(_)) {
        return Ok(LocalDaemonResponse::EventDeliveryStatus {
            status: crate::transport::event_delivery_client::event_delivery_status(
                runtime_state,
                &config,
            ),
        });
    }
    if matches!(
        request,
        LocalDaemonRequest::ListEventConnections(_)
            | LocalDaemonRequest::GetEventConnection(_)
            | LocalDaemonRequest::ListEventConnectionDependencies(_)
    ) {
        return execute_event_connection_request(
            runtime_state,
            &BTreeMap::new(),
            management_client,
            &config.daemon_id,
            caller_user_id,
            request,
        )
        .await;
    }
    if matches!(
        request,
        LocalDaemonRequest::InstallEventConnection(_)
            | LocalDaemonRequest::ObserveEventConnectionAuthorization(_)
            | LocalDaemonRequest::RefreshEventConnection(_)
            | LocalDaemonRequest::TestEventConnection(_)
            | LocalDaemonRequest::ReconnectEventConnection(_)
            | LocalDaemonRequest::ListEventConnectionResources(_)
            | LocalDaemonRequest::RemoveEventConnection(_)
    ) {
        let management_targets = resolve_event_generator_management_targets(
            runtime_state,
            config_projection,
            &config,
            caller_user_id,
            &request,
        )
        .await?;
        return execute_event_connection_request(
            runtime_state,
            &management_targets,
            management_client,
            &config.daemon_id,
            caller_user_id,
            request,
        )
        .await;
    }
    if matches!(
        request,
        LocalDaemonRequest::StartEventGeneratorAuthorization(_)
            | LocalDaemonRequest::ListEventGeneratorResources(_)
    ) {
        let targets = resolve_event_generator_management_targets(
            runtime_state,
            config_projection,
            &config,
            caller_user_id,
            &request,
        )
        .await?;
        let owner_id = event_connection_owner_id(&config.daemon_id, caller_user_id);
        let management_client = management_client.clone();
        return tokio::task::spawn_blocking(move || {
            aegs_management_request(&management_client, &targets, &owner_id, &request)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "query event generator management service",
            message: error.to_string(),
        })?;
    }
    let registry_url = config.event_registry_url.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(registry_url) = registry_url {
            cached_remote_catalog_request(&registry_url, &request)
        } else {
            builtin_catalog_request(&request)
        }
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "query event generator catalog",
        message: error.to_string(),
    })?
}

pub(crate) async fn resolve_event_generator_management_targets(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    config: &crate::config::DaemonConfig,
    caller_user_id: &str,
    request: &LocalDaemonRequest,
) -> Result<BTreeMap<String, crate::config::EventGeneratorManagementTarget>, DaemonError> {
    let generator_ids =
        event_generator_ids_from_management_request(runtime_state, caller_user_id, request)?;
    let mut targets = config.event_generator_management_targets.clone();
    if generator_ids.is_empty() {
        // A generator-less list is limited to providers that already have a local connection.
        // The kernel must not page the entire Store and mint capabilities for every provider
        // merely to render an installed-connection list.
        return Ok(targets);
    }
    let Some(profile) = config.cloud_relay.as_ref() else {
        return Ok(targets);
    };
    let registry_url = config.event_registry_url.clone().ok_or_else(|| {
        catalog_error("event generator management bootstrap requires a registry URL".to_string())
    })?;
    let requested_owner_ids = [
        config.daemon_id.clone(),
        event_connection_owner_id(&config.daemon_id, caller_user_id),
    ];
    for generator_id in generator_ids {
        let refresh_before_ms = crate::session::unix_epoch_ms().saturating_add(30_000);
        if targets.get(&generator_id).is_some_and(|target| {
            requested_owner_ids.iter().all(|owner| {
                let scoped_valid = target
                    .owner_scoped
                    .as_ref()
                    .and_then(|scoped| scoped.get(owner))
                    .is_some_and(|credential| {
                        credential
                            .expires_at_ms
                            .is_none_or(|expires_at_ms| expires_at_ms > refresh_before_ms)
                    });
                let legacy_valid = target.owner_scoped.is_none()
                    && target
                        .owner_ids
                        .as_ref()
                        .is_none_or(|owners| owners.contains(owner))
                    && target
                        .expires_at_ms
                        .is_none_or(|expires_at_ms| expires_at_ms > refresh_before_ms);
                scoped_valid || legacy_valid
            })
        }) {
            continue;
        }
        let detail_request = LocalDaemonRequest::GetEventGeneratorDetail(
            crate::local::GetEventGeneratorDetailRequest {
                generator_id: generator_id.clone(),
                version: None,
            },
        );
        let detail_registry_url = registry_url.clone();
        let detail_response = tokio::task::spawn_blocking(move || {
            cached_remote_catalog_request(&detail_registry_url, &detail_request)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "query event generator detail for management bootstrap",
            message: error.to_string(),
        })??;
        let LocalDaemonResponse::EventGeneratorDetail { detail } = detail_response else {
            return Err(catalog_error(
                "event catalog returned an unexpected detail response for management bootstrap"
                    .to_string(),
            ));
        };
        let Some(management_url) = detail.summary.management_url.clone() else {
            continue;
        };
        let owner_ids = requested_owner_ids.to_vec();
        let capability = issue_event_generator_management_capability(
            profile,
            &config.daemon_id,
            &generator_id,
            &detail.summary.version,
            &detail.summary.manifest_digest,
            &management_url,
        )
        .await?;
        let expires_at_ms = chrono::DateTime::parse_from_rfc3339(&capability.expires_at)
            .map_err(|error| {
                catalog_error(format!(
                    "cloud returned an invalid management capability expiry: {error}"
                ))
            })?
            .timestamp_millis()
            .try_into()
            .map_err(|_| {
                catalog_error("management capability expiry is before the Unix epoch".to_string())
            })?;
        let credential = crate::config::EventGeneratorManagementTargetCredential {
            url: management_url.clone(),
            token: capability.token.clone(),
            expires_at_ms: Some(expires_at_ms),
        };
        let owner_scoped = requested_owner_ids
            .iter()
            .cloned()
            .map(|owner| (owner, credential.clone()))
            .collect();
        let resolved_target = crate::config::EventGeneratorManagementTarget {
            url: management_url,
            token: capability.token,
            expires_at_ms: Some(expires_at_ms),
            owner_ids: Some(owner_ids),
            owner_scoped: Some(owner_scoped),
        };
        config_projection.merge_event_generator_management_target(&generator_id, resolved_target);
        // Read the merged projection so concurrent owner/generator resolutions
        // are preserved in the map returned to this operation.
        targets = config_projection
            .snapshot()
            .event_generator_management_targets;
    }
    Ok(targets)
}

fn event_generator_ids_from_management_request(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    request: &LocalDaemonRequest,
) -> Result<Vec<String>, DaemonError> {
    let connection_generator = |connection_id: &str| {
        runtime_state
            .event_connection_registry()
            .get(caller_user_id, connection_id)
            .ok()
            .flatten()
            .map(|connection| connection.generator_id)
    };
    let authorization_generator = |authorization_id: &str| {
        runtime_state
            .event_connection_registry()
            .authorization(caller_user_id, authorization_id)
            .ok()
            .flatten()
            .map(|authorization| authorization.generator_id)
    };
    let generator_id = match request {
        LocalDaemonRequest::StartEventGeneratorAuthorization(request) => {
            Some(request.generator_id.clone())
        }
        LocalDaemonRequest::ListEventGeneratorResources(request) => {
            Some(request.generator_id.clone())
        }
        LocalDaemonRequest::ListEventConnections(request) => request.generator_id.clone(),
        LocalDaemonRequest::GetEventConnection(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::InstallEventConnection(request) => Some(request.generator_id.clone()),
        LocalDaemonRequest::ObserveEventConnectionAuthorization(request) => {
            authorization_generator(&request.authorization_id)
        }
        LocalDaemonRequest::RefreshEventConnection(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::TestEventConnection(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::ReconnectEventConnection(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::ListEventConnectionResources(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::ListEventConnectionDependencies(request) => {
            connection_generator(&request.connection_id)
        }
        LocalDaemonRequest::RemoveEventConnection(request) => {
            connection_generator(&request.connection_id)
        }
        _ => None,
    };
    if let Some(generator_id) = generator_id {
        return Ok(vec![generator_id]);
    }
    if matches!(
        request,
        LocalDaemonRequest::ListEventConnections(request) if request.generator_id.is_none()
    ) {
        return Ok(runtime_state
            .event_connection_registry()
            .list(caller_user_id, None)?
            .into_iter()
            .map(|connection| connection.generator_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect());
    }
    Ok(Vec::new())
}

pub(crate) async fn validate_event_connection(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    management_client: &AegsManagementHttpClient,
    caller_user_id: &str,
    generator_id: &str,
    connection_id: &str,
) -> Result<(), DaemonError> {
    let config = config_projection.snapshot();
    let owner_id = event_connection_owner_id(&config.daemon_id, caller_user_id);
    let target_request =
        LocalDaemonRequest::ListEventConnections(crate::local::ListEventConnectionsRequest {
            generator_id: Some(generator_id.to_string()),
            cursor: None,
            limit: 1,
        });
    let targets = resolve_event_generator_management_targets(
        runtime_state,
        config_projection,
        &config,
        caller_user_id,
        &target_request,
    )
    .await?;
    let generator_id = generator_id.to_string();
    let connection_id = connection_id.to_string();
    let expected_generator_id = generator_id.clone();
    let expected_connection_id = connection_id.clone();
    let management_client = management_client.clone();
    let page = match blocking_aegs(move || {
        query_aegs_connections(
            &management_client,
            &targets,
            &owner_id,
            &generator_id,
            Some(&connection_id),
        )
    })
    .await
    {
        Ok(page) => page,
        Err(error) => {
            mark_connection_unavailable_if_registered(
                runtime_state,
                caller_user_id,
                &expected_connection_id,
            )?;
            return Err(error);
        }
    };
    let summary = page.connections.into_iter().find(|connection| {
        connection.connection_id == expected_connection_id
            && connection.generator_id == expected_generator_id
    });
    let Some(summary) = summary else {
        mark_connection_unavailable_if_registered(
            runtime_state,
            caller_user_id,
            &expected_connection_id,
        )?;
        return Err(connection_error(
            "the selected event connection is not installed for this kernel user".to_string(),
        ));
    };
    if summary.status != crate::local::EventConnectionStatus::Ready {
        if runtime_state
            .event_connection_registry()
            .get(caller_user_id, &expected_connection_id)?
            .is_some()
        {
            runtime_state
                .event_connection_registry()
                .upsert(caller_user_id, summary.clone())?;
        }
        return Err(connection_error(format!(
            "event connection `{}` is {:?}; reconnect it before attaching",
            summary.connection_id, summary.status
        )));
    }
    runtime_state
        .event_connection_registry()
        .upsert(caller_user_id, summary)?;
    Ok(())
}

pub(crate) async fn validate_event_binding_contract(
    config_projection: &DaemonConfigProjectionStore,
    contract: &WorkflowEventBindingContract,
) -> Result<Vec<String>, DaemonError> {
    let registry_url = config_projection.snapshot().event_registry_url;
    let request =
        LocalDaemonRequest::GetEventGeneratorDetail(crate::local::GetEventGeneratorDetailRequest {
            generator_id: contract.generator_id.clone(),
            version: Some(contract.generator_version.clone()),
        });
    let response = tokio::task::spawn_blocking(move || {
        if let Some(registry_url) = registry_url {
            cached_remote_catalog_request(&registry_url, &request)
        } else {
            builtin_catalog_request(&request)
        }
    })
    .await
    .map_err(|error| catalog_error(error.to_string()))??;
    let LocalDaemonResponse::EventGeneratorDetail { detail } = response else {
        return Err(catalog_error(
            "event catalog returned an unexpected detail response".to_string(),
        ));
    };
    validate_event_binding_detail(&detail, contract)
}

fn validate_event_binding_detail(
    detail: &EventGeneratorCatalogDetail,
    contract: &WorkflowEventBindingContract,
) -> Result<Vec<String>, DaemonError> {
    let WorkflowEventBindingContract {
        generator_id,
        generator_version,
        manifest_digest,
        event_type,
        event_type_version,
        action_ids,
        reply_mode,
        ..
    } = contract;
    if detail.summary.generator_id != *generator_id || detail.summary.version != *generator_version
    {
        return Err(connection_error(format!(
            "event catalog returned `{}`@`{}` for requested `{generator_id}@{generator_version}`",
            detail.summary.generator_id, detail.summary.version
        )));
    }
    if detail.summary.manifest_digest != *manifest_digest {
        return Err(connection_error(format!(
            "event generator manifest changed; expected `{manifest_digest}`, catalog has `{}`",
            detail.summary.manifest_digest
        )));
    }
    let Some(event) = detail
        .events
        .iter()
        .find(|event| event.event_type == *event_type && event.version == *event_type_version)
    else {
        return Err(connection_error(format!(
            "event `{event_type}@{event_type_version}` is not declared by `{generator_id}@{generator_version}`"
        )));
    };
    let mut required_scopes = event
        .required_scopes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for action_id in action_ids {
        let Some(action) = detail
            .actions
            .iter()
            .find(|action| action.action_id == *action_id)
        else {
            return Err(connection_error(format!(
                "action `{action_id}` is not declared by `{generator_id}@{generator_version}`"
            )));
        };
        if action_id == "notification.reply"
            && !matches!(reply_mode.as_deref(), Some("thread" | "channel"))
        {
            return Err(connection_error(
                "notification.reply requires reply_mode thread or channel".to_string(),
            ));
        }
        required_scopes.extend(action.required_scopes.iter().cloned());
    }
    Ok(required_scopes.into_iter().collect())
}

pub(crate) fn validate_event_connection_scopes(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    connection_id: &str,
    required_scopes: &[String],
) -> Result<(), DaemonError> {
    if required_scopes.is_empty() {
        return Ok(());
    }
    let connection = runtime_state
        .event_connection_registry()
        .get(caller_user_id, connection_id)?
        .ok_or_else(|| connection_error("event connection was removed or is not installed"))?;
    validate_granted_event_connection_scopes(connection_id, &connection.scopes, required_scopes)
}

fn validate_granted_event_connection_scopes(
    connection_id: &str,
    connection_scopes: &[crate::local::EventConnectionScope],
    required_scopes: &[String],
) -> Result<(), DaemonError> {
    let granted_scopes = connection_scopes
        .iter()
        .filter(|scope| scope.granted)
        .map(|scope| scope.id.as_str())
        .collect::<BTreeSet<_>>();
    let missing_scopes = required_scopes
        .iter()
        .filter(|required| !granted_scopes.contains(required.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing_scopes.is_empty() {
        return Ok(());
    }
    Err(connection_error(format!(
        "event connection `{connection_id}` is missing required scopes: {}; reconnect it before attaching",
        missing_scopes.join(", ")
    )))
}

pub(crate) async fn validate_registered_event_connection(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    management_client: &AegsManagementHttpClient,
    caller_user_id: &str,
    generator_id: &str,
    connection_id: &str,
) -> Result<(), DaemonError> {
    let connection = runtime_state
        .event_connection_registry()
        .get(caller_user_id, connection_id)?
        .ok_or_else(|| connection_error("event connection was removed or is not installed"))?;
    if connection.generator_id != generator_id {
        return Err(connection_error(
            "event connection does not belong to the requested generator",
        ));
    }
    validate_event_connection(
        runtime_state,
        config_projection,
        management_client,
        caller_user_id,
        generator_id,
        connection_id,
    )
    .await
}

/// Ensure a workflow runtime action has a live management target before the
/// synchronous action path reads the projection. This is deliberately shared
/// by reply and event-context actions so registry-issued targets are resolved
/// after a kernel restart as well as during connection/binding setup.
pub(crate) async fn ensure_event_generator_management_target_for_workflow_run(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    session_id: &str,
    workflow_run_ref: &str,
) -> Result<(), DaemonError> {
    let (generator_id, caller_user_id) = {
        let session_store = runtime_state.session_store();
        let session = session_store.read().get_session(session_id)?;
        let workflow_run = session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)?;
        let invocation = workflow_run.publication_invocation().ok_or_else(|| {
            connection_error("event runtime action is missing its invocation".to_string())
        })?;
        let binding_id = invocation.hook_id.as_deref().ok_or_else(|| {
            connection_error("event runtime action is missing its binding identity".to_string())
        })?;
        let binding = session
            .workflow_event_bindings()
            .iter()
            .find(|binding| binding.id == binding_id)
            .ok_or_else(|| {
                connection_error(format!("event binding `{binding_id}` was not found"))
            })?;
        (
            binding.generator_id.clone(),
            session.owner_user_id().to_string(),
        )
    };
    let config = config_projection.snapshot();
    let request =
        LocalDaemonRequest::ListEventConnections(crate::local::ListEventConnectionsRequest {
            generator_id: Some(generator_id),
            cursor: None,
            limit: 1,
        });
    resolve_event_generator_management_targets(
        runtime_state,
        config_projection,
        &config,
        &caller_user_id,
        &request,
    )
    .await
    .map(|_| ())
}

async fn execute_event_connection_request(
    runtime_state: &KernelRuntimeState,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    management_client: &AegsManagementHttpClient,
    daemon_id: &str,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let registry = runtime_state.event_connection_registry();
    let owner_id = event_connection_owner_id(daemon_id, caller_user_id);
    match request {
        LocalDaemonRequest::ListEventConnections(request) => {
            let connections = registry
                .list(caller_user_id, request.generator_id.as_deref())?
                .into_iter()
                .map(|connection| {
                    project_event_connection_usage(
                        runtime_state,
                        registry,
                        caller_user_id,
                        connection,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let offset = decode_offset(request.cursor.as_deref())?;
            let limit = bounded_limit(request.limit) as usize;
            let page = connections
                .into_iter()
                .skip(offset)
                .take(limit.saturating_add(1))
                .collect::<Vec<_>>();
            let has_more = page.len() > limit;
            let connections = page.into_iter().take(limit).collect::<Vec<_>>();
            Ok(LocalDaemonResponse::EventConnectionsPage {
                page: EventConnectionPage {
                    connections,
                    next_cursor: has_more.then(|| encode_offset(offset + limit)),
                },
            })
        }
        LocalDaemonRequest::GetEventConnection(request) => {
            let connection = require_connection(registry, caller_user_id, &request.connection_id)?;
            let connection = project_event_connection_usage(
                runtime_state,
                registry,
                caller_user_id,
                connection,
            )?;
            Ok(LocalDaemonResponse::EventConnection { connection })
        }
        LocalDaemonRequest::InstallEventConnection(request) => {
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let flow = blocking_aegs(move || {
                start_aegs_authorization(
                    &management_client,
                    &targets,
                    &owner_id,
                    &request.generator_id,
                    request.return_url,
                )
            })
            .await?;
            let authorization = registry.start_authorization(caller_user_id, flow)?;
            Ok(LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization })
        }
        LocalDaemonRequest::ObserveEventConnectionAuthorization(request) => {
            let (authorization, observed) = observe_event_connection_authorization(
                runtime_state,
                targets,
                management_client,
                &owner_id,
                caller_user_id,
                &request.authorization_id,
            )
            .await?;
            Ok(LocalDaemonResponse::EventConnectionAuthorizationObserved {
                authorization,
                connection: observed,
            })
        }
        LocalDaemonRequest::RefreshEventConnection(request) => {
            let current = require_connection(registry, caller_user_id, &request.connection_id)?;
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let generator_id = current.generator_id.clone();
            let connection_id = current.connection_id.clone();
            let inspection = match blocking_aegs(move || {
                refresh_aegs_connection(
                    &management_client,
                    &targets,
                    &owner_id,
                    &generator_id,
                    &connection_id,
                )
            })
            .await
            {
                Ok(inspection) => inspection,
                Err(error) => {
                    registry.mark_status(
                        caller_user_id,
                        &current.connection_id,
                        crate::local::EventConnectionStatus::Unavailable,
                    )?;
                    return Err(error);
                }
            };
            let connection = registry.apply_inspection(caller_user_id, inspection)?;
            let connection = project_event_connection_usage(
                runtime_state,
                registry,
                caller_user_id,
                connection,
            )?;
            Ok(LocalDaemonResponse::EventConnection { connection })
        }
        LocalDaemonRequest::TestEventConnection(request) => {
            let connection = require_connection(registry, caller_user_id, &request.connection_id)?;
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let generator_id = connection.generator_id;
            let connection_id = connection.connection_id;
            let result = blocking_aegs(move || {
                test_aegs_connection(
                    &management_client,
                    &targets,
                    &owner_id,
                    &generator_id,
                    &connection_id,
                    request.event_type,
                )
            })
            .await?;
            Ok(LocalDaemonResponse::EventConnectionTested { result })
        }
        LocalDaemonRequest::ReconnectEventConnection(request) => {
            let connection = require_connection(registry, caller_user_id, &request.connection_id)?;
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let reconnect = chariox_event_protocol::AegsConnectionReconnectRequest {
                generator_id: connection.generator_id.clone(),
                owner_id,
                connection_id: connection.connection_id,
                return_url: request.return_url,
            };
            let generator_id = reconnect.generator_id.clone();
            let flow = match blocking_aegs(move || {
                post_aegs_json(
                    &management_client,
                    &targets,
                    &generator_id,
                    "/v1/connections/reconnect",
                    &reconnect,
                )
            })
            .await
            {
                Ok(flow) => flow,
                Err(error) => {
                    registry.mark_status(
                        caller_user_id,
                        &request.connection_id,
                        crate::local::EventConnectionStatus::Unavailable,
                    )?;
                    return Err(error);
                }
            };
            let authorization = registry.start_authorization(caller_user_id, flow)?;
            Ok(LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization })
        }
        LocalDaemonRequest::ListEventConnectionResources(request) => {
            let connection = require_connection(registry, caller_user_id, &request.connection_id)?;
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let generator_id = connection.generator_id.clone();
            let resource_query = chariox_event_protocol::AegsProviderResourceQuery {
                generator_id: generator_id.clone(),
                owner_id,
                connection_id: connection.connection_id.clone(),
                query: request.query,
                cursor: request.cursor,
                limit: request.limit,
            };
            let page = match blocking_aegs(move || {
                post_aegs_json(
                    &management_client,
                    &targets,
                    &generator_id,
                    "/v1/resources/query",
                    &resource_query,
                )
            })
            .await
            {
                Ok(page) => page,
                Err(error) => {
                    registry.mark_status(
                        caller_user_id,
                        &request.connection_id,
                        crate::local::EventConnectionStatus::Unavailable,
                    )?;
                    return Err(error);
                }
            };
            Ok(LocalDaemonResponse::EventConnectionResourcesPage { page })
        }
        LocalDaemonRequest::ListEventConnectionDependencies(request) => {
            require_connection(registry, caller_user_id, &request.connection_id)?;
            let dependencies = event_connection_dependencies(
                runtime_state,
                caller_user_id,
                &request.connection_id,
            );
            Ok(LocalDaemonResponse::EventConnectionDependencies {
                connection_id: request.connection_id,
                dependencies,
            })
        }
        LocalDaemonRequest::RemoveEventConnection(request) => {
            let connection = require_connection(registry, caller_user_id, &request.connection_id)?;
            let dependencies = event_connection_dependencies(
                runtime_state,
                caller_user_id,
                &request.connection_id,
            );
            let active_dependency_count = dependencies
                .iter()
                .filter(|dependency| {
                    dependency.status != crate::session::WorkflowEventBindingStatus::Tombstoned
                })
                .count();
            if !request.confirm {
                return Err(connection_error(format!(
                    "removing this connection requires confirm=true and will deactivate {active_dependency_count} workflow binding(s)"
                )));
            }
            if active_dependency_count != 0 {
                return Err(connection_error(
                    "dependent workflow bindings must be deactivated before connection removal"
                        .to_string(),
                ));
            }
            let targets = targets.clone();
            let management_client = management_client.clone();
            let owner_id = owner_id.clone();
            let generator_id = connection.generator_id.clone();
            let connection_id = connection.connection_id.clone();
            let revoked = match blocking_aegs(move || {
                revoke_aegs_connection(
                    &management_client,
                    &targets,
                    &owner_id,
                    &generator_id,
                    &connection_id,
                )
            })
            .await
            {
                Ok(revoked) => revoked,
                Err(error) => {
                    registry.mark_status(
                        caller_user_id,
                        &request.connection_id,
                        crate::local::EventConnectionStatus::Unavailable,
                    )?;
                    return Err(error);
                }
            };
            if !revoked.revoked {
                registry.mark_status(
                    caller_user_id,
                    &request.connection_id,
                    crate::local::EventConnectionStatus::Unavailable,
                )?;
                return Err(connection_error(
                    "event generator did not confirm connection revocation",
                ));
            }
            registry
                .remove_authorizations_for_connection(caller_user_id, &connection.connection_id)?;
            registry.remove(caller_user_id, &connection.connection_id)?;
            Ok(LocalDaemonResponse::EventConnectionRemoved {
                connection,
                deactivated_bindings: dependencies,
            })
        }
        _ => Err(connection_error(
            "request is not an event connection request".to_string(),
        )),
    }
}

async fn observe_event_connection_authorization(
    runtime_state: &KernelRuntimeState,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    management_client: &AegsManagementHttpClient,
    owner_id: &str,
    caller_user_id: &str,
    authorization_id: &str,
) -> Result<
    (
        crate::local::EventConnectionAuthorization,
        Option<crate::local::EventConnection>,
    ),
    DaemonError,
> {
    let registry = runtime_state.event_connection_registry();
    let mut authorization = registry
        .authorization(caller_user_id, authorization_id)?
        .ok_or_else(|| connection_error("event connection authorization was not found"))?;
    let query_targets = targets.clone();
    let inspection_targets = targets.clone();
    let query_client = management_client.clone();
    let inspection_client = management_client.clone();
    let owner_id = owner_id.to_string();
    let inspection_owner_id = owner_id.clone();
    let generator_id = authorization.generator_id.clone();
    let connection_id = authorization.connection_id.clone();
    let page = blocking_aegs(move || {
        query_aegs_connections(
            &query_client,
            &query_targets,
            &owner_id,
            &generator_id,
            connection_id.as_deref(),
        )
    })
    .await?;
    let mut observed = None;
    for summary in page.connections {
        if authorization.connection_id.as_deref() != Some(summary.connection_id.as_str()) {
            continue;
        }
        let generator_id = summary.generator_id.clone();
        let connection_id = summary.connection_id.clone();
        let mut connection = registry.upsert(caller_user_id, summary)?;
        let targets = inspection_targets.clone();
        let management_client = inspection_client.clone();
        let owner_id = inspection_owner_id.clone();
        if let Ok(inspection) = blocking_aegs(move || {
            inspect_aegs_connection(
                &management_client,
                &targets,
                &owner_id,
                &generator_id,
                &connection_id,
            )
        })
        .await
        {
            connection = registry.apply_inspection(caller_user_id, inspection)?;
        }
        connection =
            project_event_connection_usage(runtime_state, registry, caller_user_id, connection)?;
        observed = Some(connection);
    }
    if let Some(connection) = &observed {
        authorization.status = format!("{:?}", connection.status).to_ascii_lowercase();
        authorization = registry.update_authorization(caller_user_id, authorization)?;
    }
    Ok((authorization, observed))
}

async fn blocking_aegs<T, F>(operation: F) -> Result<T, DaemonError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, DaemonError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| connection_error(format!("AEGS management task failed: {error}")))?
}

fn require_connection(
    registry: &crate::event_connection::EventConnectionRegistry,
    caller_user_id: &str,
    connection_id: &str,
) -> Result<crate::local::EventConnection, DaemonError> {
    registry
        .get(caller_user_id, connection_id)?
        .ok_or_else(|| connection_error("event connection was not found".to_string()))
}

fn mark_connection_unavailable_if_registered(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    connection_id: &str,
) -> Result<(), DaemonError> {
    let registry = runtime_state.event_connection_registry();
    if registry.get(caller_user_id, connection_id)?.is_some() {
        registry.mark_status(
            caller_user_id,
            connection_id,
            crate::local::EventConnectionStatus::Unavailable,
        )?;
    }
    Ok(())
}

fn event_connection_dependencies(
    runtime_state: &KernelRuntimeState,
    caller_user_id: &str,
    connection_id: &str,
) -> Vec<WorkflowEventBindingDependency> {
    runtime_state
        .list_session_snapshots()
        .into_iter()
        .flat_map(|session| {
            let session_id = session.id().to_string();
            let owned_publication_ids = session
                .workflow_publications()
                .iter()
                .filter(|publication| publication.created_by_user_id() == caller_user_id)
                .map(|publication| publication.id().to_string())
                .collect::<std::collections::BTreeSet<_>>();
            session
                .workflow_event_bindings()
                .iter()
                .filter(|binding| {
                    binding.connection_id == connection_id
                        && owned_publication_ids.contains(&binding.publication_id)
                })
                .map(move |binding| WorkflowEventBindingDependency {
                    session_id: session_id.clone(),
                    publication_id: binding.publication_id.clone(),
                    binding_id: binding.id.clone(),
                    status: binding.status,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn project_event_connection_usage(
    runtime_state: &KernelRuntimeState,
    registry: &crate::event_connection::EventConnectionRegistry,
    caller_user_id: &str,
    connection: crate::local::EventConnection,
) -> Result<crate::local::EventConnection, DaemonError> {
    let attached_trigger_count =
        event_connection_dependencies(runtime_state, caller_user_id, &connection.connection_id)
            .into_iter()
            .filter(|dependency| {
                dependency.status != crate::session::WorkflowEventBindingStatus::Tombstoned
            })
            .count() as u64;
    registry.set_attached_trigger_count(
        caller_user_id,
        &connection.connection_id,
        attached_trigger_count,
    )
}

pub(crate) fn workflow_event_binding_contract(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    binding_id: &str,
) -> Option<WorkflowEventBindingContract> {
    runtime_state
        .list_session_snapshots()
        .into_iter()
        .find(|session| session.id() == session_id)
        .and_then(|session| {
            session
                .workflow_event_bindings()
                .iter()
                .find(|binding| binding.id == binding_id)
                .map(WorkflowEventBindingContract::from)
        })
}

fn decode_offset(cursor: Option<&str>) -> Result<usize, DaemonError> {
    let Some(cursor) = cursor else { return Ok(0) };
    cursor
        .strip_prefix("offset-")
        .and_then(|offset| offset.parse::<usize>().ok())
        .ok_or_else(|| connection_error("event connection cursor is invalid".to_string()))
}

fn encode_offset(offset: usize) -> String {
    format!("offset-{offset}")
}

/// Select the capability scoped to one request owner. Static administrator
/// targets have no owner scope and are returned unchanged; registry-issued
/// targets must have an exact owner entry so a concurrent user cannot reuse a
/// different user's short-lived token.
pub(crate) fn select_event_generator_management_target(
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    generator_id: &str,
    owner_id: &str,
) -> Result<crate::config::EventGeneratorManagementTarget, DaemonError> {
    let target = targets.get(generator_id).ok_or_else(|| {
        catalog_error(format!(
            "event generator `{generator_id}` has no configured management target"
        ))
    })?;
    if let Some(scoped) = target.owner_scoped.as_ref() {
        let credential = scoped.get(owner_id).ok_or_else(|| {
            catalog_error(format!(
                "event generator `{generator_id}` management capability is not authorized for owner `{owner_id}`"
            ))
        })?;
        return Ok(crate::config::EventGeneratorManagementTarget {
            url: credential.url.clone(),
            token: credential.token.clone(),
            expires_at_ms: credential.expires_at_ms,
            owner_ids: Some(vec![owner_id.to_string()]),
            owner_scoped: None,
        });
    }
    if target
        .owner_ids
        .as_ref()
        .is_some_and(|owners| !owners.iter().any(|owner| owner == owner_id))
    {
        return Err(catalog_error(format!(
            "event generator `{generator_id}` management capability is not authorized for owner `{owner_id}`"
        )));
    }
    Ok(target.clone())
}

fn aegs_management_request(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (generator_id, path, body) = match request {
        LocalDaemonRequest::StartEventGeneratorAuthorization(request) => (
            request.generator_id.as_str(),
            "/v1/authorizations",
            serde_json::to_string(&chariox_event_protocol::AegsAuthorizationStartRequest {
                generator_id: request.generator_id.clone(),
                owner_id: owner_id.to_string(),
                return_url: request.return_url.clone(),
            })
            .map_err(|error| catalog_error(error.to_string()))?,
        ),
        LocalDaemonRequest::ListEventGeneratorResources(request) => (
            request.generator_id.as_str(),
            "/v1/resources/query",
            serde_json::to_string(&chariox_event_protocol::AegsProviderResourceQuery {
                generator_id: request.generator_id.clone(),
                owner_id: owner_id.to_string(),
                connection_id: request.connection_id.clone(),
                query: request.query.clone(),
                cursor: request.cursor.clone(),
                limit: request.limit,
            })
            .map_err(|error| catalog_error(error.to_string()))?,
        ),
        _ => {
            return Err(catalog_error(
                "request is not an event generator management request".to_string(),
            ))
        }
    };
    let target = select_event_generator_management_target(targets, generator_id, owner_id)?;
    let url = format!("{}{path}", target.url);
    let agent = management_client
        .agent_builder(&target)
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .build();
    let response = agent
        .post(&url)
        .set("authorization", &format!("Bearer {}", target.token))
        .set("x-chariox-owner-id", owner_id)
        .set("content-type", "application/json")
        .send_string(&body)
        .map_err(|error| catalog_error(format!("AEGS {generator_id} request failed: {error}")))?;
    let response = response
        .into_string()
        .map_err(|error| catalog_error(error.to_string()))?;
    match request {
        LocalDaemonRequest::StartEventGeneratorAuthorization(_) => {
            let flow = serde_json::from_str(&response)
                .map_err(|error| catalog_error(format!("AEGS response is invalid: {error}")))?;
            Ok(LocalDaemonResponse::EventGeneratorAuthorizationStarted { flow })
        }
        LocalDaemonRequest::ListEventGeneratorResources(_) => {
            let page = serde_json::from_str(&response)
                .map_err(|error| catalog_error(format!("AEGS response is invalid: {error}")))?;
            Ok(LocalDaemonResponse::EventGeneratorResourcesPage { page })
        }
        _ => unreachable!("event generator management request was matched above"),
    }
}

fn start_aegs_authorization(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    return_url: Option<String>,
) -> Result<chariox_event_protocol::AegsAuthorizationFlow, DaemonError> {
    let request = chariox_event_protocol::AegsAuthorizationStartRequest {
        generator_id: generator_id.to_string(),
        owner_id: owner_id.to_string(),
        return_url,
    };
    post_aegs_json(
        management_client,
        targets,
        generator_id,
        "/v1/authorizations",
        &request,
    )
}

fn query_aegs_connections(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    connection_id: Option<&str>,
) -> Result<chariox_event_protocol::AegsConnectionPage, DaemonError> {
    let request = chariox_event_protocol::AegsConnectionQuery {
        generator_id: generator_id.to_string(),
        owner_id: owner_id.to_string(),
        connection_id: connection_id.map(str::to_string),
        cursor: None,
        limit: 100,
    };
    post_aegs_json(
        management_client,
        targets,
        generator_id,
        "/v1/connections/query",
        &request,
    )
}

fn refresh_aegs_connection(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    connection_id: &str,
) -> Result<chariox_event_protocol::AegsConnectionInspection, DaemonError> {
    post_aegs_json(
        management_client,
        targets,
        generator_id,
        "/v1/connections/refresh",
        &chariox_event_protocol::AegsConnectionRefreshRequest {
            generator_id: generator_id.to_string(),
            owner_id: owner_id.to_string(),
            connection_id: connection_id.to_string(),
        },
    )
}

fn inspect_aegs_connection(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    connection_id: &str,
) -> Result<chariox_event_protocol::AegsConnectionInspection, DaemonError> {
    post_aegs_json(
        management_client,
        targets,
        generator_id,
        "/v1/connections/inspect",
        &chariox_event_protocol::AegsConnectionInspectionRequest {
            generator_id: generator_id.to_string(),
            owner_id: owner_id.to_string(),
            connection_id: connection_id.to_string(),
        },
    )
}

fn test_aegs_connection(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    connection_id: &str,
    event_type: Option<String>,
) -> Result<chariox_event_protocol::AegsConnectionTestEventResponse, DaemonError> {
    post_aegs_json(
        management_client,
        targets,
        generator_id,
        "/v1/connections/test-event",
        &chariox_event_protocol::AegsConnectionTestEventRequest {
            generator_id: generator_id.to_string(),
            owner_id: owner_id.to_string(),
            connection_id: connection_id.to_string(),
            event_type,
        },
    )
}

pub(crate) fn invoke_aegs_action(
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    request: &chariox_event_protocol::AegsProviderActionRequest,
) -> Result<chariox_event_protocol::AegsProviderActionResponse, DaemonError> {
    post_aegs_json(
        &AegsManagementHttpClient::default(),
        targets,
        &request.generator_id,
        "/v1/actions",
        request,
    )
}

fn revoke_aegs_connection(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    owner_id: &str,
    generator_id: &str,
    connection_id: &str,
) -> Result<chariox_event_protocol::AegsConnectionRevokeResponse, DaemonError> {
    let request = chariox_event_protocol::AegsConnectionRevokeRequest {
        generator_id: generator_id.to_string(),
        owner_id: owner_id.to_string(),
        connection_id: connection_id.to_string(),
    };
    post_aegs_json::<_, chariox_event_protocol::AegsConnectionRevokeResponse>(
        management_client,
        targets,
        generator_id,
        "/v1/connections/revoke",
        &request,
    )
}

fn post_aegs_json<T: serde::Serialize, R: serde::de::DeserializeOwned>(
    management_client: &AegsManagementHttpClient,
    targets: &BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    generator_id: &str,
    path: &str,
    request: &T,
) -> Result<R, DaemonError> {
    let owner_id = serde_json::to_value(request)
        .ok()
        .and_then(|value| {
            value
                .get("owner_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| catalog_error("AEGS request is missing owner_id".to_string()))?;
    let target = select_event_generator_management_target(targets, generator_id, &owner_id)?;
    let body = serde_json::to_string(request).map_err(|error| catalog_error(error.to_string()))?;
    let mut http_request = management_client
        .agent_builder(&target)
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .build()
        .post(&format!("{}{path}", target.url))
        .set("authorization", &format!("Bearer {}", target.token));
    http_request = http_request.set("x-chariox-owner-id", &owner_id);
    let response = http_request
        .set("content-type", "application/json")
        .send_string(&body)
        .map_err(|error| catalog_error(format!("AEGS {generator_id} request failed: {error}")))?
        .into_string()
        .map_err(|error| catalog_error(error.to_string()))?;
    serde_json::from_str(&response)
        .map_err(|error| catalog_error(format!("AEGS response is invalid: {error}")))
}

pub(crate) fn event_connection_owner_id(daemon_id: &str, caller_user_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(format!("{daemon_id}\0{caller_user_id}").as_bytes());
    format!("kernel-user-{digest:x}")
}

/// Registry-issued management targets must resolve only to public addresses.
/// Static targets are an explicit administrator action and retain loopback or
/// private-network support for local/self-hosted deployments.
pub(crate) fn aegs_management_agent_builder(
    target: &crate::config::EventGeneratorManagementTarget,
) -> ureq::AgentBuilder {
    aegs_management_agent_builder_with_resolver(target, resolve_public_aegs_addresses)
}

pub(crate) fn aegs_management_agent_builder_with_resolver<R>(
    target: &crate::config::EventGeneratorManagementTarget,
    resolver: R,
) -> ureq::AgentBuilder
where
    R: ureq::Resolver + 'static,
{
    let builder = ureq::AgentBuilder::new();
    if is_registry_issued_management_target(target) {
        builder.https_only(true).resolver(resolver)
    } else {
        builder
    }
}

fn is_registry_issued_management_target(
    target: &crate::config::EventGeneratorManagementTarget,
) -> bool {
    target.expires_at_ms.is_some() && target.owner_ids.is_some()
}

fn resolve_public_aegs_addresses(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    let addresses = netloc.to_socket_addrs()?.collect::<Vec<_>>();
    validate_public_aegs_addresses(netloc, addresses)
}

fn validate_public_aegs_addresses(
    netloc: &str,
    addresses: Vec<SocketAddr>,
) -> io::Result<Vec<SocketAddr>> {
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("AEGS management target `{netloc}` did not resolve"),
        ));
    }
    if let Some(address) = addresses
        .iter()
        .find(|address| !is_globally_reachable_aegs_destination(address.ip()))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "registry-issued AEGS management target `{netloc}` resolved to non-public address {}",
                address.ip()
            ),
        ));
    }
    Ok(addresses)
}

fn connection_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "event_connection.control",
        message: message.into(),
    }
}

fn cached_remote_catalog_request(
    registry_url: &str,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let key = format!(
        "{}|protocol={}|{}",
        registry_url,
        chariox_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION,
        serde_json::to_string(request).map_err(|error| catalog_error(error.to_string()))?
    );
    let cache = CATALOG_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(response) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
        .filter(|entry| entry.stored_at.elapsed() <= CATALOG_CACHE_FRESH_TTL)
        .map(|entry| entry.response)
    {
        return Ok(response);
    }
    match remote_catalog_request(registry_url, request) {
        Ok(response) => {
            if let Ok(mut cache) = cache.lock() {
                if cache.len() >= CATALOG_CACHE_MAX_ENTRIES {
                    if let Some(oldest) = cache
                        .iter()
                        .min_by_key(|(_, entry)| entry.stored_at)
                        .map(|(key, _)| key.clone())
                    {
                        cache.remove(&oldest);
                    }
                }
                cache.insert(
                    key,
                    CatalogCacheEntry {
                        response: response.clone(),
                        stored_at: Instant::now(),
                    },
                );
            }
            Ok(response)
        }
        Err(error) => {
            let cached = cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key).cloned())
                .filter(|entry| entry.stored_at.elapsed() <= CATALOG_CACHE_STALE_TTL);
            cached
                .map(|entry| mark_catalog_response_stale(entry.response))
                .ok_or(error)
        }
    }
}

fn mark_catalog_response_stale(response: LocalDaemonResponse) -> LocalDaemonResponse {
    match response {
        LocalDaemonResponse::EventGeneratorCatalogPage { mut page } => {
            page.stale = true;
            LocalDaemonResponse::EventGeneratorCatalogPage { page }
        }
        response => response,
    }
}

fn remote_catalog_request(
    registry_url: &str,
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut url =
        url::Url::parse(registry_url).map_err(|error| catalog_error(error.to_string()))?;
    match request {
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(request) => {
            url.set_path("/v1/event-generators");
            url.query_pairs_mut()
                .append_pair("view", "landing")
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            fetch_page(url)
        }
        LocalDaemonRequest::SearchEventGeneratorCatalog(request) => {
            url.set_path("/v1/event-generators");
            let mut query = url.query_pairs_mut();
            query
                .append_pair("q", request.query.trim())
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            if let Some(category) = request.category.as_deref() {
                query.append_pair("category", category);
            }
            if let Some(verification) = request.verification.as_deref() {
                query.append_pair("verification", verification);
            }
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            fetch_page(url)
        }
        LocalDaemonRequest::BrowseEventGeneratorCategory(request) => {
            url.set_path("/v1/event-generators");
            let mut query = url.query_pairs_mut();
            query
                .append_pair("category", request.category.trim())
                .append_pair("limit", &bounded_limit(request.limit).to_string());
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            fetch_page(url)
        }
        LocalDaemonRequest::GetEventGeneratorDetail(request) => {
            url.set_path(&format!(
                "/v1/event-generators/{}",
                percent_encode_path_segment(&request.generator_id)
            ));
            if let Some(version) = request.version.as_deref() {
                url.query_pairs_mut().append_pair("version", version);
            }
            let detail = fetch_json::<EventGeneratorCatalogDetail>(url)?;
            Ok(LocalDaemonResponse::EventGeneratorDetail { detail })
        }
        LocalDaemonRequest::BrowseEventGeneratorEvents(request) => {
            url.set_path(&format!(
                "/v1/event-generators/{}/events",
                percent_encode_path_segment(&request.generator_id)
            ));
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &bounded_event_limit(request.limit).to_string());
            if let Some(search) = request.query.as_deref() {
                query.append_pair("query", search);
            }
            if let Some(cursor) = request.cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
            drop(query);
            let page = fetch_json::<EventGeneratorEventPage>(url)?;
            Ok(LocalDaemonResponse::EventGeneratorEventsPage { page })
        }
        _ => Err(catalog_error(
            "request is not an event catalog request".to_string(),
        )),
    }
}

fn builtin_catalog_request(
    request: &LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::GetEventGeneratorCatalogLanding(request) => {
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page("", None, None, 0, bounded_limit(request.limit)),
            })
        }
        LocalDaemonRequest::SearchEventGeneratorCatalog(request) => {
            let cursor = decode_cursor(request.cursor.as_deref())?;
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page(
                    &request.query,
                    request.category.as_deref(),
                    request.verification.as_deref(),
                    cursor,
                    bounded_limit(request.limit),
                ),
            })
        }
        LocalDaemonRequest::BrowseEventGeneratorCategory(request) => {
            let cursor = decode_cursor(request.cursor.as_deref())?;
            Ok(LocalDaemonResponse::EventGeneratorCatalogPage {
                page: builtin_page(
                    "",
                    Some(&request.category),
                    None,
                    cursor,
                    bounded_limit(request.limit),
                ),
            })
        }
        LocalDaemonRequest::GetEventGeneratorDetail(request) => {
            let detail = builtin_detail(&request.generator_id).ok_or_else(|| {
                catalog_error(format!(
                    "event generator `{}` was not found",
                    request.generator_id
                ))
            })?;
            Ok(LocalDaemonResponse::EventGeneratorDetail { detail })
        }
        LocalDaemonRequest::BrowseEventGeneratorEvents(request) => {
            let detail = builtin_detail(&request.generator_id).ok_or_else(|| {
                catalog_error(format!(
                    "event generator `{}` was not found",
                    request.generator_id
                ))
            })?;
            let search = request
                .query
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let offset = decode_cursor(request.cursor.as_deref())?;
            let events = detail
                .events
                .into_iter()
                .filter(|event| {
                    search.is_empty()
                        || event.event_type.to_ascii_lowercase().contains(&search)
                        || event.name.to_ascii_lowercase().contains(&search)
                })
                .collect::<Vec<_>>();
            let end = (offset + bounded_event_limit(request.limit) as usize).min(events.len());
            let page_events = if offset >= events.len() {
                Vec::new()
            } else {
                events[offset..end].to_vec()
            };
            Ok(LocalDaemonResponse::EventGeneratorEventsPage {
                page: EventGeneratorEventPage {
                    events: page_events,
                    next_cursor: (end < events.len()).then(|| format!("offset:{end}")),
                },
            })
        }
        _ => Err(catalog_error(
            "request is not an event catalog request".to_string(),
        )),
    }
}

fn builtin_page(
    query: &str,
    category: Option<&str>,
    verification: Option<&str>,
    offset: usize,
    limit: u32,
) -> EventGeneratorCatalogPage {
    let query = query.trim().to_ascii_lowercase();
    let mut services = builtin_summaries()
        .into_iter()
        .filter(|service| {
            (query.is_empty()
                || [
                    service.name.as_str(),
                    service.summary.as_str(),
                    service.provider.as_str(),
                    service.generator_id.as_str(),
                ]
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&query)))
                && category.is_none_or(|category| {
                    service
                        .categories
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(category))
                })
                && verification.is_none_or(|verification| service.verification == verification)
        })
        .collect::<Vec<_>>();
    services.sort_by_key(|service| {
        (
            !service.recommended,
            std::cmp::Reverse(service.installed_count),
        )
    });
    let total = services.len();
    let end = (offset + limit as usize).min(total);
    let services = if offset >= total {
        Vec::new()
    } else {
        services[offset..end].to_vec()
    };
    EventGeneratorCatalogPage {
        services,
        next_cursor: (end < total).then(|| format!("offset:{end}")),
        categories: builtin_categories(),
        facets: vec![EventCatalogFacet {
            id: "verification".to_string(),
            values: vec![EventCatalogFacetValue {
                value: "chariox".to_string(),
                count: 1,
            }],
        }],
        stale: false,
    }
}

fn builtin_summaries() -> Vec<EventGeneratorCatalogSummary> {
    vec![EventGeneratorCatalogSummary {
        schema_version: 1,
        generator_id: "dev.chariox.dummy".to_string(),
        version: "1.0.0".to_string(),
        name: "Dummy Events".to_string(),
        summary: "Deterministic event source for local and deployment validation.".to_string(),
        provider: "Chariox test harness".to_string(),
        publisher: EventGeneratorParty {
            id: "dev.chariox".to_string(),
            name: "Chariox".to_string(),
            url: Some("https://chariox.com".to_string()),
        },
        operator: EventGeneratorParty {
            id: "local".to_string(),
            name: "Local operator".to_string(),
            url: None,
        },
        verification: "chariox".to_string(),
        manifest_digest: BUILTIN_DUMMY_MANIFEST_DIGEST.to_string(),
        protocol_version: chariox_event_protocol::AEGS_MANAGEMENT_PROTOCOL_VERSION,
        categories: vec!["Developer tools".to_string(), "Testing".to_string()],
        installed_count: 0,
        recommended: true,
        availability: "available".to_string(),
        management_url: None,
    }]
}

fn builtin_detail(generator_id: &str) -> Option<EventGeneratorCatalogDetail> {
    let summary = builtin_summaries()
        .into_iter()
        .find(|summary| summary.generator_id == generator_id)?;
    Some(EventGeneratorCatalogDetail {
        summary,
        authorization: serde_json::json!({
            "kind": "none",
            "disclosure": "This deterministic fixture does not contact an external provider."
        }),
        events: vec![EventGeneratorEventDefinition {
            event_type: "dummy.test".to_string(),
            version: 1,
            name: "Test event".to_string(),
            description: "Emits a user-supplied prompt and optional artifact references."
                .to_string(),
            filter_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "channel": {"type": "string", "default": "default"}
                }
            }),
            required_scopes: Vec::new(),
        }],
        actions: Vec::new(),
        signature: serde_json::json!({
            "key_id": "dev.chariox.fixture.2026-08-v3",
            "algorithm": "ed25519",
            "digest": BUILTIN_DUMMY_MANIFEST_DIGEST,
            "value": "SO4kz3m5fiQcEYCIkaFoxnpeiiUuAgetPd9CpFBD2vUnQnYjAH/5orIH4HwayQ4oX4mdCW6b3a0spbgsh4l/Dw=="
        }),
        deprecation: None,
    })
}

fn builtin_categories() -> Vec<EventCatalogCategory> {
    [
        ("developer-tools", "Developer tools", 1),
        ("testing", "Testing", 1),
    ]
    .into_iter()
    .map(|(id, name, service_count)| EventCatalogCategory {
        id: id.to_string(),
        name: name.to_string(),
        service_count,
    })
    .collect()
}

fn fetch_page(url: url::Url) -> Result<LocalDaemonResponse, DaemonError> {
    let page = fetch_json::<EventGeneratorCatalogPage>(url)?;
    Ok(LocalDaemonResponse::EventGeneratorCatalogPage { page })
}

fn fetch_json<T: serde::de::DeserializeOwned>(url: url::Url) -> Result<T, DaemonError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();
    let response = agent
        .get(url.as_str())
        .set(
            "x-chariox-event-protocol-version",
            &chariox_event_protocol::EVENT_DELIVERY_PROTOCOL_VERSION.to_string(),
        )
        .call()
        .map_err(|error| catalog_error(format!("registry request failed: {error}")))?;
    let mut body = String::new();
    response
        .into_reader()
        .take(CATALOG_RESPONSE_MAX_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| catalog_error(error.to_string()))?;
    if body.len() as u64 > CATALOG_RESPONSE_MAX_BYTES {
        return Err(catalog_error(
            "registry response exceeded 2 MiB".to_string(),
        ));
    }
    serde_json::from_str(&body).map_err(|error| catalog_error(error.to_string()))
}

fn bounded_limit(limit: u32) -> u32 {
    limit.clamp(1, 50)
}

fn bounded_event_limit(limit: u32) -> u32 {
    limit.clamp(1, 100)
}

fn decode_cursor(cursor: Option<&str>) -> Result<usize, DaemonError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    cursor
        .strip_prefix("offset:")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| catalog_error("invalid event catalog cursor".to_string()))
}

fn percent_encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn catalog_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "query event generator catalog",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding_contract(
        detail: &EventGeneratorCatalogDetail,
        generator_id: &str,
        event_type: &str,
        action_ids: Vec<String>,
    ) -> WorkflowEventBindingContract {
        WorkflowEventBindingContract {
            generator_id: generator_id.to_string(),
            generator_version: detail.summary.version.clone(),
            manifest_digest: detail.summary.manifest_digest.clone(),
            connection_id: "connection-1".to_string(),
            event_type: event_type.to_string(),
            event_type_version: 1,
            action_ids,
            reply_mode: None,
        }
    }

    #[test]
    fn builtin_dummy_catalog_matches_publisher_manifest_fixture() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/fixtures/event-generators/dummy/manifest.json"
        ))
        .expect("dummy manifest fixture must be valid JSON");
        let detail = builtin_detail("dev.chariox.dummy").expect("dummy catalog detail");

        assert_eq!(
            detail.summary.manifest_digest,
            manifest
                .pointer("/signature/digest")
                .and_then(serde_json::Value::as_str)
                .expect("signed manifest digest")
        );
        assert_eq!(detail.signature, manifest["signature"]);
        assert!(manifest.get("operator").is_none());
        assert!(manifest.get("verification").is_none());
    }

    #[test]
    fn event_binding_contract_rejects_undeclared_event_type() {
        let detail = builtin_detail("dev.chariox.dummy").expect("dummy catalog detail");
        let contract =
            binding_contract(&detail, &detail.summary.generator_id, "dummy_typo", vec![]);
        let error = validate_event_binding_detail(&detail, &contract)
            .expect_err("undeclared event type must be rejected");
        assert!(error.to_string().contains("is not declared"));
    }

    #[test]
    fn event_binding_contract_accepts_declared_event_type() {
        let detail = builtin_detail("dev.chariox.dummy").expect("dummy catalog detail");
        let contract =
            binding_contract(&detail, &detail.summary.generator_id, "dummy.test", vec![]);
        validate_event_binding_detail(&detail, &contract)
            .expect("declared event type should be accepted");
    }

    #[test]
    fn event_binding_contract_returns_unique_event_and_action_scopes() {
        let mut detail = builtin_detail("dev.chariox.dummy").expect("dummy catalog detail");
        detail.events[0].required_scopes = vec!["events:read".to_string()];
        detail.actions = vec![crate::local::EventGeneratorActionDefinition {
            action_id: "dummy.acknowledge".to_string(),
            name: "Acknowledge".to_string(),
            description: "Acknowledge the event".to_string(),
            required_scopes: vec!["actions:write".to_string(), "events:read".to_string()],
            target: "originating_resource".to_string(),
            mutation: true,
            idempotent: true,
            input_schema: serde_json::json!({"type": "object"}),
        }];

        let contract = binding_contract(
            &detail,
            &detail.summary.generator_id,
            "dummy.test",
            vec!["dummy.acknowledge".to_string()],
        );
        let required_scopes = validate_event_binding_detail(&detail, &contract)
            .expect("declared action should be accepted");

        assert_eq!(required_scopes, vec!["actions:write", "events:read"]);
    }

    #[test]
    fn event_binding_scope_validation_fails_closed_for_omitted_grants() {
        let required_scopes = vec!["actions:write".to_string()];
        let error = validate_granted_event_connection_scopes("connection-1", &[], &required_scopes)
            .expect_err("omitted scopes must not grant a scope-bearing action");
        assert!(error
            .to_string()
            .contains("missing required scopes: actions:write"));

        validate_granted_event_connection_scopes("connection-1", &[], &[])
            .expect("actions without required scopes remain available");
    }

    #[test]
    fn registry_issued_aegs_resolver_rejects_non_public_destinations() {
        for destination in [
            "127.0.0.1:443",
            "10.0.0.1:443",
            "169.254.169.254:443",
            "192.168.1.1:443",
            "[::1]:443",
            "[::ffff:127.0.0.1]:443",
            "[fc00::1]:443",
            "[fe80::1]:443",
        ] {
            let error = resolve_public_aegs_addresses(destination)
                .expect_err("non-public management destination must be rejected");
            assert_eq!(
                error.kind(),
                io::ErrorKind::PermissionDenied,
                "{destination}"
            );
        }
    }

    #[test]
    fn registry_issued_aegs_resolver_accepts_public_destinations() {
        assert_eq!(
            resolve_public_aegs_addresses("8.8.8.8:443").expect("public IPv4 destination"),
            vec!["8.8.8.8:443".parse::<SocketAddr>().expect("socket address")]
        );
        assert_eq!(
            resolve_public_aegs_addresses("[2606:4700:4700::1111]:443")
                .expect("public IPv6 destination"),
            vec!["[2606:4700:4700::1111]:443"
                .parse::<SocketAddr>()
                .expect("socket address")]
        );
    }

    #[test]
    fn registry_issued_aegs_resolver_rejects_mixed_dns_answers() {
        let error = validate_public_aegs_addresses(
            "aegs.example.test:443",
            vec![
                "8.8.8.8:443".parse().expect("public socket address"),
                "127.0.0.1:443".parse().expect("private socket address"),
            ],
        )
        .expect_err("one private DNS answer must reject the entire target");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn static_management_targets_retain_self_hosted_network_access() {
        let static_target = crate::config::EventGeneratorManagementTarget {
            url: "http://127.0.0.1:43132".to_string(),
            token: "operator-token".to_string(),
            expires_at_ms: None,
            owner_ids: None,
            owner_scoped: None,
        };
        assert!(!is_registry_issued_management_target(&static_target));

        let registry_target = crate::config::EventGeneratorManagementTarget {
            url: "https://aegs.example.test".to_string(),
            token: "signed-capability".to_string(),
            expires_at_ms: Some(1_700_000_000_000),
            owner_ids: Some(vec!["kernel-1".to_string()]),
            owner_scoped: None,
        };
        assert!(is_registry_issued_management_target(&registry_target));
    }

    #[test]
    fn event_binding_contract_rejects_mismatched_generator_identity() {
        let detail = builtin_detail("dev.chariox.dummy").expect("dummy catalog detail");
        let contract = binding_contract(&detail, "dev.chariox.other", "dummy.test", vec![]);
        let error = validate_event_binding_detail(&detail, &contract)
            .expect_err("mismatched catalog identity must be rejected");
        assert!(error.to_string().contains("event catalog returned"));
    }
}
