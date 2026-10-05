//! Protocols 358–359: App inbox routes and connection grants on an owner's
//! event generator connection. Each is checked with the generator, then stored
//! by the App lane while the connection lane (and, for routes, the event
//! interest lock) is held.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{
    AppInboxConnection, CreateAppInboxRouteRequest, LocalDaemonRequest, LocalDaemonResponse,
};

impl CommandRouter {
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    // MD-4: select the App event branch before reserving its validation future.
    pub(super) fn dispatch_app_event_request<'a>(
        &'a self,
        command: &'a crate::runtime::command::KernelCommand,
        request: &'a LocalDaemonRequest,
        caller_user_id: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Option<LocalDaemonResponse>, DaemonError>>
                + Send
                + 'a,
        >,
    > {
        match request {
            LocalDaemonRequest::CreateAppInboxRoute(route) => Box::pin(async move {
                let Some(connection) = &route.connection else {
                    return Ok(None);
                };
                let _connection_guard = self
                    .event_connection_lanes
                    .lock(caller_user_id, &connection.connection_id)
                    .await;
                Box::pin(self.check_app_route(caller_user_id, route, connection)).await?;
                let _interest = self.event_interest_lock.lock().await;
                self.refuse_claimed_interest(caller_user_id, route, connection)?;
                Ok(self
                    .runtime_state
                    .execute_app_control_request(command, request)
                    .await)
            }),
            LocalDaemonRequest::GrantAppConnection(grant) => Box::pin(async move {
                let _connection_guard = self
                    .event_connection_lanes
                    .lock(caller_user_id, &grant.connection_id)
                    .await;
                Box::pin(
                    crate::runtime::event_catalog_control::validate_event_connection(
                        &self.runtime_state,
                        &self.config_projection,
                        &self.aegs_management_http_client,
                        caller_user_id,
                        &grant.generator_id,
                        &grant.connection_id,
                    ),
                )
                .await?;
                Ok(self
                    .runtime_state
                    .execute_app_control_request(command, request)
                    .await)
            }),
            _ => Box::pin(async { Ok(None) }),
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

    /// Another active App route already receives these events;
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
}

fn route_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "create App inbox route",
        message,
    }
}
