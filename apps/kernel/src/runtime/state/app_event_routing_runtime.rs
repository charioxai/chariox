//! Protocol 1011: an App with `capabilities.eventRouting` configures the
//! existing delivery path for itself. The owner's `GrantAppConnection` names
//! the one connection it may route through; the App's signed manifest names
//! the generator, the incoming events and the most routes. Nothing here takes
//! a connection, token, installation or Room from the App.
//!
//! Routes are the ordinary inbox routes (`create_app_inbox_route`), marked as
//! the App's own, and targets are the ordinary agent triggers and App
//! automations: this module only checks who may create them and for what.
use super::{
    app_inbox_runtime::{RouteContract, RouteOrigin},
    KernelRuntimeState,
};
use crate::durable_state::app_inbox::{AppInboxOperation, AppInboxOutcome};
use crate::local::{
    AppAutomationStatus, AppAutomationSummary, AppInboxConnection, AppRequestErrorCode,
    ConfigureAppAutomationRequest, CreateAgentTriggerRequest, CreateAppInboxRouteRequest,
    DisableAppAutomationRequest, EventGeneratorResource, InstallEventConnectionRequest,
    ListEventConnectionResourcesRequest, LocalDaemonRequest, LocalDaemonResponse,
};
use crate::runtime::event_catalog_control::AegsManagementHttpClient;
use crate::session::AgentTriggerNotificationSource;
use chariox_app_package::EventRouting;
use chariox_app_runtime::app_inbox::{InboxCounts, InboxRoute};
use serde_json::Value;

/// Targets (agent triggers and workflow automations) one App may hold.
pub(crate) const MAX_TARGETS: usize = 200;
/// Pages of the generator's resource list searched for one repository.
const RESOURCE_PAGES: usize = 5;
const RESOURCE_PAGE_SIZE: u32 = 100;

/// What the kernel knows about the calling App; nothing in it comes from the
/// App's own requests.
pub(crate) struct RoutingScope {
    pub owner: String,
    pub installation: String,
    pub declared: EventRouting,
}

/// Why a seam call failed. Each variant is one wire code the App can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutingError {
    InvalidArgument,
    EventNotDeclared,
    ConnectionNotGranted,
    ConnectionAmbiguous,
    ConnectionUnavailable,
    RepositoryNotInConnection,
    RouteLimit,
    RouteNotFound,
    RouteExists,
    NotAllowed,
    TargetLimit,
    TargetNotFound,
    Busy,
    StorageUnavailable,
}

impl RoutingError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::EventNotDeclared => "EVENT_NOT_DECLARED",
            Self::ConnectionNotGranted => "CONNECTION_NOT_GRANTED",
            Self::ConnectionAmbiguous => "CONNECTION_AMBIGUOUS",
            Self::ConnectionUnavailable => "CONNECTION_UNAVAILABLE",
            Self::RepositoryNotInConnection => "REPOSITORY_NOT_IN_CONNECTION",
            Self::RouteLimit => "ROUTE_LIMIT",
            Self::RouteNotFound => "ROUTE_NOT_FOUND",
            Self::RouteExists => "ROUTE_EXISTS",
            Self::NotAllowed => "NOT_ALLOWED",
            Self::TargetLimit => "TARGET_LIMIT",
            Self::TargetNotFound => "TARGET_NOT_FOUND",
            Self::Busy => "APP_BUSY",
            Self::StorageUnavailable => "STORAGE_UNAVAILABLE",
        }
    }

    pub(crate) fn retryable(self) -> bool {
        matches!(
            self,
            Self::Busy | Self::StorageUnavailable | Self::ConnectionUnavailable
        )
    }
}

impl From<AppRequestErrorCode> for RoutingError {
    fn from(code: AppRequestErrorCode) -> Self {
        match code {
            AppRequestErrorCode::NotFound => Self::RouteNotFound,
            AppRequestErrorCode::Conflict => Self::RouteExists,
            AppRequestErrorCode::LimitExceeded => Self::RouteLimit,
            AppRequestErrorCode::Busy => Self::Busy,
            AppRequestErrorCode::StorageUnavailable => Self::StorageUnavailable,
            _ => Self::InvalidArgument,
        }
    }
}

/// Whom an App call acts for, from the callers the kernel saw when the call
/// arrived (`BrokerRequest::callers`), never from a request field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Acting {
    /// Only people: a window action of the owner.
    Person,
    /// One agent, alone or beside a person's call. A person's call that
    /// overlaps an agent's must not lend it the person's authority.
    Agent(String),
}

impl Acting {
    pub(crate) fn of(callers: &[Value]) -> Result<Self, RoutingError> {
        let mut agents = std::collections::BTreeSet::new();
        let mut person = false;
        for actor in callers {
            let id = actor["id"].as_str().filter(|id| !id.is_empty());
            match (actor["kind"].as_str(), id) {
                (Some("agent"), Some(id)) => {
                    agents.insert(id.to_owned());
                }
                (Some("human"), Some(_)) => person = true,
                _ => {}
            }
        }
        match (agents.len(), person) {
            (0, true) => Ok(Self::Person),
            (1, _) => Ok(Self::Agent(agents.into_iter().next().unwrap_or_default())),
            _ => Err(RoutingError::NotAllowed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouteSpec {
    pub route_id: String,
    pub event: String,
    pub repository: String,
    pub source_event_type: String,
    pub source_event_version: u32,
}

impl RouteSpec {
    fn validate(&self) -> Result<(), RoutingError> {
        let text = |value: &str, max: usize| {
            !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        if text(&self.route_id, 128)
            && text(&self.event, 64)
            && text(&self.repository, 200)
            && text(&self.source_event_type, 128)
            && self.source_event_version > 0
        {
            Ok(())
        } else {
            Err(RoutingError::InvalidArgument)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouteView {
    pub route_id: String,
    pub event: String,
    pub repository: String,
    pub source_event_type: String,
    pub source_event_version: u32,
    /// Takes occurrences: not paused and not handed over.
    pub active: bool,
    pub counts: InboxCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionView {
    pub generator_id: String,
    pub connection_id: Option<String>,
    pub status: String,
    pub max_routes: u32,
    pub routes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetView {
    pub automation_id: String,
    pub event: String,
    pub kind: &'static str,
    pub agent_id: Option<String>,
    pub state: &'static str,
}

fn route_view((route, counts): &(InboxRoute, InboxCounts)) -> RouteView {
    RouteView {
        route_id: route.route_id.clone(),
        event: route.event_name.clone(),
        repository: route
            .source
            .as_ref()
            .map(|source| source.connection_scope.clone())
            .unwrap_or_default(),
        source_event_type: route.source_event_type.clone(),
        source_event_version: route.source_event_version,
        active: route.admits(),
        counts: *counts,
    }
}

impl KernelRuntimeState {
    /// The App's own routes, with their counts.
    async fn routing_routes(
        &self,
        scope: &RoutingScope,
    ) -> Result<Vec<(InboxRoute, InboxCounts)>, RoutingError> {
        match self
            .inbox(AppInboxOperation::Routes {
                owner: scope.owner.clone(),
                installation: scope.installation.clone(),
            })
            .await?
        {
            AppInboxOutcome::Routes(routes) => Ok(routes),
            _ => Err(RoutingError::StorageUnavailable),
        }
    }

    fn routing_own(routes: Vec<(InboxRoute, InboxCounts)>) -> Vec<(InboxRoute, InboxCounts)> {
        routes
            .into_iter()
            .filter(|(route, _)| route.app_managed)
            .collect()
    }

    /// The one connection the owner granted for the manifest's generator.
    fn routing_grant(
        &self,
        scope: &RoutingScope,
    ) -> Result<crate::durable_state::app_connections::ConnectionGrant, RoutingError> {
        let mut grants = self
            .owned
            .durable_state_store
            .app_connection_grants(&scope.owner, &scope.installation)
            .map_err(|_| RoutingError::StorageUnavailable)?
            .into_iter()
            .filter(|grant| grant.generator_id == scope.declared.generator);
        let grant = grants.next().ok_or(RoutingError::ConnectionNotGranted)?;
        if grants.next().is_some() {
            return Err(RoutingError::ConnectionAmbiguous);
        }
        Ok(grant)
    }

    fn routing_event_user(&self, owner: &str) -> String {
        crate::event_connection::event_connection_user_id(
            &self.owned.config_projection.snapshot(),
            owner,
        )
    }

    pub(crate) async fn app_routing_list_routes(
        &self,
        scope: &RoutingScope,
    ) -> Result<Vec<RouteView>, RoutingError> {
        let routes = Self::routing_own(self.routing_routes(scope).await?);
        Ok(routes.iter().map(route_view).collect())
    }

    pub(crate) async fn app_routing_ensure_route(
        &self,
        client: &AegsManagementHttpClient,
        scope: &RoutingScope,
        spec: RouteSpec,
    ) -> Result<RouteView, RoutingError> {
        spec.validate()?;
        if !scope.declared.incoming.contains(&spec.event) {
            return Err(RoutingError::EventNotDeclared);
        }
        let grant = self.routing_grant(scope)?;
        let all = self.routing_routes(scope).await?;
        if let Some(existing) = all
            .iter()
            .find(|(route, _)| route.route_id == spec.route_id)
        {
            let (route, _) = existing;
            let same = route.app_managed
                && route.event_name == spec.event
                && route.source_event_type == spec.source_event_type
                && route.source_event_version == spec.source_event_version
                && route.source.as_ref().is_some_and(|source| {
                    source.connection_id == grant.connection_id
                        && source.connection_scope == spec.repository
                });
            return if same {
                Ok(route_view(existing))
            } else {
                Err(RoutingError::RouteExists)
            };
        }
        let own = all.iter().filter(|(route, _)| route.app_managed).count();
        if own >= scope.declared.max_routes as usize {
            return Err(RoutingError::RouteLimit);
        }
        let resource = self
            .routing_resource(client, scope, &grant.connection_id, &spec.repository)
            .await?;
        let event_user = self.routing_event_user(&scope.owner);
        crate::runtime::event_catalog_control::validate_event_connection(
            self,
            &self.owned.config_projection,
            client,
            &event_user,
            &grant.generator_id,
            &grant.connection_id,
        )
        .await
        .map_err(|_| RoutingError::ConnectionUnavailable)?;
        crate::runtime::event_catalog_control::validate_app_route_event(
            self,
            &self.owned.config_projection,
            &event_user,
            &grant.generator_id,
            &grant.connection_id,
            &spec.source_event_type,
            spec.source_event_version,
        )
        .await
        .map_err(|_| RoutingError::InvalidArgument)?;
        self.create_app_inbox_route(
            &scope.owner,
            &scope.installation,
            CreateAppInboxRouteRequest {
                installation_id: scope.installation.clone(),
                route_id: spec.route_id.clone(),
                event_name: spec.event,
                source_event_type: spec.source_event_type,
                source_event_version: spec.source_event_version,
                connection: Some(AppInboxConnection {
                    generator_id: grant.generator_id,
                    connection_id: grant.connection_id,
                    connection_scope: resource.connection_scope,
                    filter: resource.filter.unwrap_or(Value::Null),
                }),
            },
            RouteContract::Catalog,
            RouteOrigin::App,
        )
        .await?;
        let created = Self::routing_own(self.routing_routes(scope).await?);
        created
            .iter()
            .find(|(route, _)| route.route_id == spec.route_id)
            .map(route_view)
            .ok_or(RoutingError::StorageUnavailable)
    }

    /// The generator's own record of one repository of the granted connection,
    /// so the route's scope and filter come from the generator, not the App.
    async fn routing_resource(
        &self,
        client: &AegsManagementHttpClient,
        scope: &RoutingScope,
        connection_id: &str,
        repository: &str,
    ) -> Result<EventGeneratorResource, RoutingError> {
        let mut cursor = None;
        for _ in 0..RESOURCE_PAGES {
            let page = self
                .routing_resources_page(
                    client,
                    &scope.owner,
                    connection_id,
                    Some(repository.to_owned()),
                    cursor,
                    RESOURCE_PAGE_SIZE,
                )
                .await?;
            if let Some(found) = page
                .resources
                .into_iter()
                .find(|resource| resource.connection_scope == repository)
            {
                return Ok(found);
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Err(RoutingError::RepositoryNotInConnection)
    }

    async fn routing_resources_page(
        &self,
        client: &AegsManagementHttpClient,
        owner: &str,
        connection_id: &str,
        query: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<crate::local::EventGeneratorResourcePage, RoutingError> {
        let response =
            crate::runtime::event_catalog_control::execute_event_catalog_request_with_client(
                self,
                &self.owned.config_projection,
                client,
                owner,
                LocalDaemonRequest::ListEventConnectionResources(
                    ListEventConnectionResourcesRequest {
                        connection_id: connection_id.to_owned(),
                        query,
                        cursor,
                        limit: limit.clamp(1, RESOURCE_PAGE_SIZE),
                    },
                ),
            )
            .await
            .map_err(|_| RoutingError::ConnectionUnavailable)?;
        match response {
            LocalDaemonResponse::EventConnectionResourcesPage { page } => Ok(page),
            _ => Err(RoutingError::ConnectionUnavailable),
        }
    }

    pub(crate) async fn app_routing_remove_route(
        &self,
        scope: &RoutingScope,
        route_id: &str,
    ) -> Result<(), RoutingError> {
        self.routing_own_route(scope, route_id).await?;
        self.inbox(AppInboxOperation::RemoveRoute {
            owner: scope.owner.clone(),
            installation: scope.installation.clone(),
            route_id: route_id.to_owned(),
        })
        .await?;
        Ok(())
    }

    pub(crate) async fn app_routing_set_route_active(
        &self,
        scope: &RoutingScope,
        route_id: &str,
        active: bool,
    ) -> Result<RouteView, RoutingError> {
        self.routing_own_route(scope, route_id).await?;
        self.inbox(AppInboxOperation::SetRoutePaused {
            owner: scope.owner.clone(),
            installation: scope.installation.clone(),
            route_id: route_id.to_owned(),
            paused: !active,
        })
        .await?;
        let routes = Self::routing_own(self.routing_routes(scope).await?);
        routes
            .iter()
            .find(|(route, _)| route.route_id == route_id)
            .map(route_view)
            .ok_or(RoutingError::RouteNotFound)
    }

    /// An owner's hand-made route is not the App's to change.
    async fn routing_own_route(
        &self,
        scope: &RoutingScope,
        route_id: &str,
    ) -> Result<(), RoutingError> {
        match self
            .inbox(AppInboxOperation::Route {
                owner: scope.owner.clone(),
                installation: scope.installation.clone(),
                route_id: route_id.to_owned(),
            })
            .await?
        {
            AppInboxOutcome::Route(Some(route)) if route.app_managed => Ok(()),
            _ => Err(RoutingError::RouteNotFound),
        }
    }

    pub(crate) async fn app_routing_connection_status(
        &self,
        scope: &RoutingScope,
    ) -> Result<ConnectionView, RoutingError> {
        let routes = Self::routing_own(self.routing_routes(scope).await?);
        let granted = match self.routing_grant(scope) {
            Ok(grant) => Some(grant),
            Err(RoutingError::ConnectionNotGranted) => None,
            Err(error) => return Err(error),
        };
        let status = match &granted {
            None => "not_granted".to_owned(),
            Some(grant) => {
                let event_user = self.routing_event_user(&scope.owner);
                match self
                    .event_connection_registry()
                    .get(&event_user, &grant.connection_id)
                    .map_err(|_| RoutingError::StorageUnavailable)?
                {
                    Some(connection) => serde_json::to_value(connection.status)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_else(|| "unavailable".into()),
                    None => "removed".to_owned(),
                }
            }
        };
        Ok(ConnectionView {
            generator_id: scope.declared.generator.clone(),
            connection_id: granted.map(|grant| grant.connection_id),
            status,
            max_routes: scope.declared.max_routes,
            routes: routes.len() as u32,
        })
    }

    pub(crate) async fn app_routing_connection_resources(
        &self,
        client: &AegsManagementHttpClient,
        scope: &RoutingScope,
        query: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<crate::local::EventGeneratorResourcePage, RoutingError> {
        let grant = self.routing_grant(scope)?;
        self.routing_resources_page(
            client,
            &scope.owner,
            &grant.connection_id,
            query,
            cursor,
            limit,
        )
        .await
    }

    /// Starts the generator's authorization (consent happens on the provider's
    /// page). It grants the App nothing: the owner still grants the new
    /// connection with `GrantAppConnection`.
    pub(crate) async fn app_routing_start_authorization(
        &self,
        client: &AegsManagementHttpClient,
        scope: &RoutingScope,
        return_url: Option<String>,
    ) -> Result<(String, String), RoutingError> {
        let response =
            crate::runtime::event_catalog_control::execute_event_catalog_request_with_client(
                self,
                &self.owned.config_projection,
                client,
                &scope.owner,
                LocalDaemonRequest::InstallEventConnection(InstallEventConnectionRequest {
                    generator_id: scope.declared.generator.clone(),
                    return_url,
                }),
            )
            .await
            .map_err(|_| RoutingError::ConnectionUnavailable)?;
        match response {
            LocalDaemonResponse::EventConnectionAuthorizationStarted { authorization } => {
                let url = authorization
                    .authorization_url
                    .ok_or(RoutingError::ConnectionUnavailable)?;
                Ok((authorization.authorization_id, url))
            }
            _ => Err(RoutingError::ConnectionUnavailable),
        }
    }

    // ---- targets --------------------------------------------------------

    async fn routing_automations(
        &self,
        scope: &RoutingScope,
    ) -> Result<Vec<AppAutomationSummary>, RoutingError> {
        match self
            .app_control_response(
                scope.owner.clone(),
                scope.owner.clone(),
                scope.installation.clone(),
                LocalDaemonRequest::ListAppAutomations(crate::local::ListAppAutomationsRequest {
                    installation_id: scope.installation.clone(),
                    session_id: None,
                }),
            )
            .await?
        {
            LocalDaemonResponse::AppAutomations { automations, .. } => Ok(automations),
            _ => Err(RoutingError::StorageUnavailable),
        }
    }

    fn target_view(automation: &AppAutomationSummary) -> TargetView {
        let agent = automation.target_kind == crate::local::WorkflowNotificationTargetKind::Agent;
        TargetView {
            automation_id: automation.automation_id.clone(),
            event: automation.event_name.clone(),
            kind: if agent { "agent" } else { "workflow" },
            agent_id: agent.then(|| automation.endpoint_id.clone()),
            state: match automation.status {
                AppAutomationStatus::Active => "active",
                AppAutomationStatus::Paused => "paused",
                AppAutomationStatus::Broken => "broken",
                AppAutomationStatus::Disabled => "disabled",
            },
        }
    }

    pub(crate) async fn app_routing_list_targets(
        &self,
        scope: &RoutingScope,
    ) -> Result<Vec<TargetView>, RoutingError> {
        Ok(self
            .routing_automations(scope)
            .await?
            .iter()
            .filter(|automation| automation.status != AppAutomationStatus::Disabled)
            .map(Self::target_view)
            .collect())
    }

    /// Delivers one outgoing event to an agent. An agent binds only itself; a
    /// person may name any agent of their Rooms.
    pub(crate) async fn app_routing_bind_agent(
        &self,
        scope: &RoutingScope,
        acting: &Acting,
        event: String,
        agent_id: Option<String>,
    ) -> Result<TargetView, RoutingError> {
        let agent_id = match (acting, agent_id) {
            (Acting::Agent(own), None) => own.clone(),
            (Acting::Agent(own), Some(named)) if *own == named => named,
            (Acting::Agent(_), Some(_)) => return Err(RoutingError::NotAllowed),
            (Acting::Person, Some(named)) => named,
            (Acting::Person, None) => return Err(RoutingError::InvalidArgument),
        };
        self.routing_require_outgoing(scope, &event).await?;
        let agent = self
            .owned
            .agent_store
            .get_agent(&agent_id)
            .map_err(|_| RoutingError::TargetNotFound)?;
        let room = agent.session_id().to_owned();
        let source = AgentTriggerNotificationSource::App {
            installation_id: scope.installation.clone(),
            event_name: event.clone(),
        };
        // One trigger per agent and event: binding again answers the same target.
        let existing = self
            .owned
            .session_store
            .get_session(&room)
            .map_err(|_| RoutingError::TargetNotFound)?
            .agent_triggers()
            .iter()
            .find_map(|trigger| match trigger.source() {
                crate::session::AgentTriggerSource::Notification {
                    source: found,
                    subscription_id,
                    ..
                } if trigger.agent_id() == agent_id && *found == source => {
                    Some(subscription_id.clone())
                }
                _ => None,
            });
        let automation_id = match existing {
            Some(id) => id,
            None => {
                if self
                    .routing_automations(scope)
                    .await?
                    .iter()
                    .filter(|a| a.status != AppAutomationStatus::Disabled)
                    .count()
                    >= MAX_TARGETS
                {
                    return Err(RoutingError::TargetLimit);
                }
                let created = self
                    .create_agent_trigger(
                        CreateAgentTriggerRequest {
                            session_id: room,
                            agent_id: agent_id.clone(),
                            source: crate::local::AgentTriggerSourceRequest::Notification {
                                source,
                            },
                            busy_policy: None,
                            prompt: None,
                        },
                        &scope.owner,
                    )
                    .await
                    .map_err(|_| RoutingError::TargetNotFound)?;
                let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = created else {
                    return Err(RoutingError::StorageUnavailable);
                };
                match trigger.source() {
                    crate::session::AgentTriggerSource::Notification {
                        subscription_id, ..
                    } => subscription_id.clone(),
                    _ => return Err(RoutingError::StorageUnavailable),
                }
            }
        };
        Ok(TargetView {
            automation_id,
            event,
            kind: "agent",
            agent_id: Some(agent_id),
            state: "active",
        })
    }

    /// Delivers one outgoing event to a workflow. Only a person's window may
    /// choose a workflow: an agent never sends the App's events there.
    pub(crate) async fn app_routing_bind_workflow(
        &self,
        scope: &RoutingScope,
        acting: &Acting,
        event: String,
        session_id: String,
        publication_ref: String,
        queue_ref: Option<String>,
    ) -> Result<TargetView, RoutingError> {
        if *acting != Acting::Person {
            return Err(RoutingError::NotAllowed);
        }
        self.routing_require_outgoing(scope, &event).await?;
        let live = self.routing_automations(scope).await?;
        if live
            .iter()
            .filter(|automation| automation.status != AppAutomationStatus::Disabled)
            .count()
            >= MAX_TARGETS
        {
            return Err(RoutingError::TargetLimit);
        }
        let automation_id = format!("wf_{:032x}", rand::random::<u128>());
        let response = self
            .app_control_response(
                scope.owner.clone(),
                scope.owner.clone(),
                scope.installation.clone(),
                LocalDaemonRequest::ConfigureAppAutomation(ConfigureAppAutomationRequest {
                    installation_id: scope.installation.clone(),
                    automation_id,
                    expected_revision: 0,
                    event_name: event,
                    session_id,
                    publication_ref,
                    queue_ref,
                    scheduled: false,
                }),
            )
            .await?;
        match response {
            LocalDaemonResponse::AppAutomation { automation, .. } => {
                Ok(Self::target_view(&automation))
            }
            _ => Err(RoutingError::StorageUnavailable),
        }
    }

    /// Ends one target. An agent ends only its own; a person any.
    pub(crate) async fn app_routing_unbind_target(
        &self,
        scope: &RoutingScope,
        acting: &Acting,
        automation_id: &str,
    ) -> Result<(), RoutingError> {
        let automations = self.routing_automations(scope).await?;
        let found = automations
            .iter()
            .find(|automation| {
                automation.automation_id == automation_id
                    && automation.status != AppAutomationStatus::Disabled
            })
            .ok_or(RoutingError::TargetNotFound)?;
        let agent = found.target_kind == crate::local::WorkflowNotificationTargetKind::Agent;
        match acting {
            Acting::Agent(own) if !(agent && found.endpoint_id == *own) => {
                return Err(RoutingError::NotAllowed)
            }
            _ => {}
        }
        if agent {
            // Deleting the trigger detaches (disables) its automation.
            let room = found.session_id.clone();
            let trigger_id = found.publication_id.clone();
            self.delete_agent_trigger(
                crate::local::DeleteAgentTriggerRequest {
                    session_id: room,
                    trigger_id,
                },
                &scope.owner,
            )
            .await
            .map_err(|_| RoutingError::TargetNotFound)?;
        } else {
            self.app_control_response(
                scope.owner.clone(),
                scope.owner.clone(),
                scope.installation.clone(),
                LocalDaemonRequest::DisableAppAutomation(DisableAppAutomationRequest {
                    session_id: None,
                    installation_id: scope.installation.clone(),
                    automation_id: automation_id.to_owned(),
                    expected_revision: found.revision,
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn routing_require_outgoing(
        &self,
        scope: &RoutingScope,
        event: &str,
    ) -> Result<(), RoutingError> {
        let catalog = self.app_catalog(&scope.owner, &scope.installation).await?;
        if catalog.outgoing_events().any(|(name, _)| name == event) {
            Ok(())
        } else {
            Err(RoutingError::EventNotDeclared)
        }
    }
}
