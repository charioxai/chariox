//! Protocols 358–360: App inbox routes and connection grants on an owner's
//! event generator connection. Each is checked with the generator, then stored
//! by the App lane while the connection lane (and, for routes, the event
//! interest lock) is held. A workflow event binding can move to an App.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{
    AppInboxConnection, CreateAppInboxRouteRequest, LocalDaemonRequest, LocalDaemonResponse,
};

impl CommandRouter {
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    pub(super) async fn dispatch_app_event_request(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Result<Option<LocalDaemonResponse>, DaemonError> {
        match request {
            LocalDaemonRequest::CreateAppInboxRoute(route) => {
                let Some(connection) = &route.connection else {
                    return Ok(None);
                };
                let _connection_guard = self
                    .event_connection_lanes
                    .lock(caller_user_id, &connection.connection_id)
                    .await;
                self.check_app_route(caller_user_id, route, connection)
                    .await?;
                let _interest = self.event_interest_lock.lock().await;
                self.refuse_claimed_interest(caller_user_id, route, connection)?;
                Ok(self
                    .runtime_state
                    .execute_app_control_request(command, request)
                    .await)
            }
            LocalDaemonRequest::GrantAppConnection(grant) => {
                let _connection_guard = self
                    .event_connection_lanes
                    .lock(caller_user_id, &grant.connection_id)
                    .await;
                crate::runtime::event_catalog_control::validate_event_connection(
                    &self.runtime_state,
                    &self.config_projection,
                    &self.aegs_management_http_client,
                    caller_user_id,
                    &grant.generator_id,
                    &grant.connection_id,
                )
                .await?;
                Ok(self
                    .runtime_state
                    .execute_app_control_request(command, request)
                    .await)
            }
            LocalDaemonRequest::MoveEventBindingToApp(request) => self
                .move_event_binding_to_app(command, request, caller_user_id)
                .await
                .map(Some),
            _ => Ok(None),
        }
    }

    /// The generator's current catalog declares the event, and the owner's
    /// connection is ready and granted its scopes.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    async fn check_app_route(
        &self,
        caller_user_id: &str,
        route: &CreateAppInboxRouteRequest,
        connection: &AppInboxConnection,
    ) -> Result<(), DaemonError> {
        crate::runtime::event_catalog_control::validate_event_connection(
            &self.runtime_state,
            &self.config_projection,
            &self.aegs_management_http_client,
            caller_user_id,
            &connection.generator_id,
            &connection.connection_id,
        )
        .await?;
        crate::runtime::event_catalog_control::validate_app_route_event(
            &self.runtime_state,
            &self.config_projection,
            caller_user_id,
            &connection.generator_id,
            &connection.connection_id,
            &route.source_event_type,
            route.source_event_version,
        )
        .await
    }

    /// Another active route (App or workflow) already receives these events;
    /// the event service keeps one route per interest. Called with the
    /// interest lock held.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    fn refuse_claimed_interest(
        &self,
        caller_user_id: &str,
        route: &CreateAppInboxRouteRequest,
        connection: &AppInboxConnection,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let event_interest_key = chariox_event_protocol::event_interest_key(
            &connection.generator_id,
            &route.source_event_type,
            route.source_event_version,
            &connection.connection_scope,
            &connection.filter,
        )
        .map_err(|error| route_error(format!("the event filter is invalid: {error}")))?;
        match self
            .runtime_state
            .event_interest_claimed_by(
                &config.daemon_id,
                &config.event_delivery_environment_id,
                &chariox_app_runtime::app_inbox::route_binding_id(
                    caller_user_id,
                    &route.installation_id,
                    &route.route_id,
                ),
                &event_interest_key,
            )
            .map_err(route_error)?
        {
            None => Ok(()),
            Some(existing) => Err(route_error(format!(
                "another route (`{existing}`) already receives these events; remove it or use a different filter"
            ))),
        }
    }

    /// Protocol 360: the binding's events go to an App instead. The binding is
    /// paused first, so the event service never has two routes for them; a
    /// refusal undoes what was done and reactivates it, and when an undo step
    /// fails the binding stays paused and the answer says so.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    async fn move_event_binding_to_app(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &crate::local::MoveEventBindingToAppRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let first = self.read_binding(command, request).await?;
        let first_connection = first.connection_id.clone();
        let _connection_guard = self
            .event_connection_lanes
            .lock(caller_user_id, &first.connection_id)
            .await;
        let binding = first;
        let config = self.config_projection.snapshot();
        if binding.environment_id != config.event_delivery_environment_id {
            return Err(move_error(format!(
                "App routes receive events in this kernel's event environment, not `{}`",
                binding.environment_id
            )));
        }
        let connection = AppInboxConnection {
            generator_id: binding.generator_id.clone(),
            connection_id: binding.connection_id.clone(),
            connection_scope: binding.connection_scope.clone(),
            filter: binding.filter.clone(),
        };
        let route = CreateAppInboxRouteRequest {
            installation_id: request.installation_id.clone(),
            route_id: request.route_id.clone(),
            event_name: request.event_name.clone(),
            source_event_type: binding.event_type.clone(),
            source_event_version: binding.event_type_version,
            connection: Some(connection.clone()),
        };
        self.check_app_route(caller_user_id, &route, &connection)
            .await?;
        let _interest = self.event_interest_lock.lock().await;
        // Read again under the locks: the owner may have changed it meanwhile.
        let binding = self.read_binding(command, request).await?;
        if binding.connection_id != first_connection {
            return Err(move_error(
                "the event binding changed during the move; try again",
            ));
        }
        let was_active = binding.active();
        if was_active {
            self.set_binding_status(
                command,
                request,
                crate::session::WorkflowEventBindingStatus::Paused,
            )
            .await?;
        }
        let refused = match self
            .replace_binding(
                command,
                request,
                &binding,
                route,
                &connection,
                caller_user_id,
            )
            .await
        {
            Ok(Replaced::Moved(response)) => return Ok(response),
            Ok(Replaced::Refused { code, undone: true }) => Ok(code),
            Ok(Replaced::Refused {
                code,
                undone: false,
            }) => {
                return Err(move_error(format!(
                    "the App refused the move ({code:?}) and it could not be fully undone; the binding stays paused. Check `app inbox list`, `app connection list` and `app automation list`, then resume the binding or move it again"
                )));
            }
            Err(error) => Err(error),
        };
        if was_active {
            self.set_binding_status(
                command,
                request,
                crate::session::WorkflowEventBindingStatus::Active,
            )
            .await
            .map_err(|error| {
                move_error(format!(
                    "the move was refused and undone, but the binding could not be resumed and stays paused: {error}"
                ))
            })?;
        }
        refused.map(|code| LocalDaemonResponse::AppRequestFailed { code })
    }

    /// The caller's binding, read through the workflow lane.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    async fn read_binding(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &crate::local::MoveEventBindingToAppRequest,
    ) -> Result<crate::session::WorkflowEventBinding, DaemonError> {
        let listed = self
            .workflow_runtime
            .dispatch_workflow_command(
                command.clone(),
                LocalDaemonRequest::ListWorkflowEventBindings(
                    crate::local::ListWorkflowEventBindingsRequest {
                        session_id: request.session_id.clone(),
                        publication_ref: None,
                    },
                ),
            )
            .await?;
        let LocalDaemonResponse::WorkflowEventBindingsListed { bindings } = listed else {
            return Err(move_error("the session's event bindings could not be read"));
        };
        bindings
            .into_iter()
            .find(|binding| binding.id == request.binding_id)
            .filter(|binding| {
                binding.status != crate::session::WorkflowEventBindingStatus::Tombstoned
            })
            .ok_or_else(|| {
                move_error(format!(
                    "event binding `{}` was not found",
                    request.binding_id
                ))
            })
    }

    /// Route, grant, then the automation last so it never needs undoing (a
    /// disabled automation of the same id is taken over at its revision). A
    /// refusal undoes the route and a grant this move made. `Err` means
    /// nothing was done.
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    async fn replace_binding(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &crate::local::MoveEventBindingToAppRequest,
        binding: &crate::session::WorkflowEventBinding,
        route: CreateAppInboxRouteRequest,
        connection: &AppInboxConnection,
        caller_user_id: &str,
    ) -> Result<Replaced, DaemonError> {
        use crate::local::{AppAutomationStatus, AppRequestErrorCode, AppWorkerRequest};
        self.refuse_claimed_interest(caller_user_id, &route, connection)?;
        let installation = request.installation_id.clone();
        let mut undo = Vec::new();
        let app = |request: LocalDaemonRequest| async move {
            match self
                .runtime_state
                .execute_app_control_request(command, &request)
                .await
            {
                Some(LocalDaemonResponse::AppRequestFailed { code }) => Err(code),
                Some(response) => Ok(response),
                None => Err(AppRequestErrorCode::InvalidRequest),
            }
        };
        let result = async {
            let route_id = route.route_id.clone();
            let LocalDaemonResponse::AppInboxRoutes { routes, .. } =
                app(LocalDaemonRequest::CreateAppInboxRoute(route)).await?
            else {
                return Err(AppRequestErrorCode::StorageUnavailable);
            };
            undo.push(LocalDaemonRequest::RemoveAppInboxRoute(
                crate::local::AppInboxRouteRequest {
                    installation_id: installation.clone(),
                    route_id: route_id.clone(),
                },
            ));
            let route = routes
                .into_iter()
                .find(|route| route.route_id == route_id)
                .ok_or(AppRequestErrorCode::StorageUnavailable)?;
            // The binding's actions become the App's to take through the same
            // connection, when its signed manifest declares that generator.
            let connection = if binding.action_ids.is_empty() {
                None
            } else {
                let LocalDaemonResponse::AppConnections {
                    connections: before,
                    ..
                } = app(LocalDaemonRequest::ListAppConnections(AppWorkerRequest {
                    installation_id: installation.clone(),
                }))
                .await?
                else {
                    return Err(AppRequestErrorCode::StorageUnavailable);
                };
                let held = before
                    .iter()
                    .any(|granted| granted.connection_id == binding.connection_id);
                match app(LocalDaemonRequest::GrantAppConnection(
                    crate::local::GrantAppConnectionRequest {
                        installation_id: installation.clone(),
                        generator_id: binding.generator_id.clone(),
                        connection_id: binding.connection_id.clone(),
                    },
                ))
                .await
                {
                    Ok(LocalDaemonResponse::AppConnections { connections, .. }) => {
                        if !held {
                            undo.push(LocalDaemonRequest::RevokeAppConnection(
                                crate::local::RevokeAppConnectionRequest {
                                    installation_id: installation.clone(),
                                    connection_id: binding.connection_id.clone(),
                                },
                            ));
                        }
                        connections
                            .into_iter()
                            .find(|granted| granted.connection_id == binding.connection_id)
                    }
                    Err(AppRequestErrorCode::InvalidRequest) => None,
                    Ok(_) => return Err(AppRequestErrorCode::StorageUnavailable),
                    Err(code) => return Err(code),
                }
            };
            let automation = match &request.automation {
                None => None,
                Some(automation) => {
                    let LocalDaemonResponse::AppAutomations { automations, .. } =
                        app(LocalDaemonRequest::ListAppAutomations(AppWorkerRequest {
                            installation_id: installation.clone(),
                        }))
                        .await?
                    else {
                        return Err(AppRequestErrorCode::StorageUnavailable);
                    };
                    let expected_revision = automations
                        .iter()
                        .find(|existing| {
                            existing.automation_id == automation.automation_id
                                && existing.status == AppAutomationStatus::Disabled
                        })
                        .map_or(0, |existing| existing.revision);
                    let LocalDaemonResponse::AppAutomation { automation, .. } =
                        app(LocalDaemonRequest::ConfigureAppAutomation(
                            crate::local::ConfigureAppAutomationRequest {
                                installation_id: installation.clone(),
                                automation_id: automation.automation_id.clone(),
                                expected_revision,
                                event_name: automation.event_name.clone(),
                                session_id: request.session_id.clone(),
                                publication_ref: binding.publication_id.clone(),
                                queue_ref: binding.queue_ref.clone(),
                                scheduled: false,
                            },
                        ))
                        .await?
                    else {
                        return Err(AppRequestErrorCode::StorageUnavailable);
                    };
                    Some(automation)
                }
            };
            Ok(LocalDaemonResponse::EventBindingMovedToApp {
                binding_id: binding.id.clone(),
                installation_id: installation.clone(),
                route,
                connection,
                automation,
            })
        }
        .await;
        match result {
            Ok(response) => Ok(Replaced::Moved(response)),
            Err(code) => {
                let mut undone = true;
                for request in undo.into_iter().rev() {
                    undone &= matches!(
                        app(request).await,
                        Ok(_) | Err(AppRequestErrorCode::NotFound)
                    );
                }
                Ok(Replaced::Refused { code, undone })
            }
        }
    }

    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    async fn set_binding_status(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &crate::local::MoveEventBindingToAppRequest,
        status: crate::session::WorkflowEventBindingStatus,
    ) -> Result<(), DaemonError> {
        self.workflow_runtime
            .dispatch_workflow_command(
                command.clone(),
                LocalDaemonRequest::SetWorkflowEventBindingStatus(
                    crate::local::SetWorkflowEventBindingStatusRequest {
                        session_id: request.session_id.clone(),
                        binding_id: request.binding_id.clone(),
                        status,
                    },
                ),
            )
            .await
            .map(|_| ())
    }

    /// A workflow binding may not take an interest an App route already
    /// receives; other workflow bindings are checked where they are stored.
    pub(super) fn refuse_app_route_interest(
        &self,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let stored = |session_id: &str, binding_id: &str| {
            self.runtime_state
                .list_session_snapshots()
                .into_iter()
                .find(|session| session.id() == session_id)
                .and_then(|session| {
                    session
                        .workflow_event_bindings()
                        .iter()
                        .find(|binding| binding.id == binding_id)
                        .map(|binding| {
                            (
                                binding.environment_id.clone(),
                                binding.event_interest_key.clone(),
                            )
                        })
                })
        };
        // The interest the binding will claim once active: a new binding's,
        // or the stored one a reactivation or transfer makes active again.
        let (environment, key) = match request {
            LocalDaemonRequest::CreateWorkflowEventBinding(binding) => {
                let environment = binding
                    .environment_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(&config.event_delivery_environment_id)
                    .to_owned();
                let Ok(key) = chariox_event_protocol::event_interest_key(
                    &binding.generator_id,
                    &binding.event_type,
                    binding.event_type_version,
                    &binding.connection_scope,
                    &binding.filter,
                ) else {
                    // The binding's own validation reports the filter.
                    return Ok(());
                };
                (environment, key)
            }
            LocalDaemonRequest::SetWorkflowEventBindingStatus(change) => {
                match stored(&change.session_id, &change.binding_id) {
                    Some(interest) => interest,
                    None => return Ok(()),
                }
            }
            LocalDaemonRequest::TransferWorkflowEventBinding(transfer) => {
                match stored(&transfer.source_session_id, &transfer.binding_id) {
                    Some(interest) => interest,
                    None => return Ok(()),
                }
            }
            _ => return Ok(()),
        };
        // App routes live in this kernel's event environment.
        if environment != config.event_delivery_environment_id {
            return Ok(());
        }
        let refused = |message: String| DaemonError::LocalTransport {
            operation: "workflow event binding",
            message,
        };
        match self.runtime_state.app_route_claiming(&key).map_err(refused)? {
            None => Ok(()),
            Some(existing) => Err(refused(format!(
                "an App inbox route (`{existing}`) already receives these events; remove it or use a different filter"
            ))),
        }
    }
}

fn route_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "create App inbox route",
        message,
    }
}

fn move_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "move event binding to App",
        message: message.into(),
    }
}

/// What `replace_binding` did.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
enum Replaced {
    Moved(LocalDaemonResponse),
    /// The App refused a step; `undone` when every earlier step was undone.
    Refused {
        code: crate::local::AppRequestErrorCode,
        undone: bool,
    },
}
