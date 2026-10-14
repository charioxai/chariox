//! MP-08 / MP-10 / MP-11: owning-kernel routing; no provider or prompt authority
//! is delegated to the relay or the native client's loopback proxy.
use std::time::Duration;

use super::CommandRouter;
use crate::{
    app::RemoteLeaseRuntime,
    error::DaemonError,
    local::{
        NativeProviderTransportOperation, NativeProviderTransportRequest,
        NativeProviderTransportResult,
    },
    provider::{ProviderClientInterface, ProviderRunState},
    transport::{
        native_provider_socket::{error, SocketScope},
        relay_peer::{RelayPeerRequest, RelayPeerResponse},
    },
};

fn local_scope(app: &mut crate::app::DaemonApp, scope: &SocketScope) -> Result<(), DaemonError> {
    if let Some(leased_agent_id) = &scope.leased_agent_id {
        let binding = RemoteLeaseRuntime::new(app).native_transport_binding(leased_agent_id)?;
        if binding != (scope.session_id.clone(), scope.agent_id.clone()) {
            return Err(error("native lease binding changed"));
        }
    } else {
        let attachment = app.attachments().get_attachment(&scope.attachment_id)?;
        if attachment.session_id() != scope.session_id {
            return Err(error("native attachment left the session"));
        }
        let session = app.sessions.get_session(&scope.session_id)?;
        if !session.has_member(attachment.owner_user_id()) {
            return Err(error("native attachment owner left the session"));
        }
        if app
            .providers()
            .get_run(&scope.provider_run_id)?
            .owner_user_id()
            != attachment.owner_user_id()
        {
            return Err(error(
                "native owning provider account belongs to another user",
            ));
        }
    }
    let agent = app.agents().get_agent(&scope.agent_id)?;
    if agent.session_id() != scope.session_id { return Err(error("native agent left the session")); }
    let run = app
        .providers()
        .get_run_for_agent(&scope.session_id, &scope.agent_id)
        .ok_or_else(|| error("native owning run is no longer available"))?;
    validate_owning_run(&run, scope)
}

fn validate_owning_run(
    run: &crate::provider::RuntimeProviderRun,
    scope: &SocketScope,
) -> Result<(), DaemonError> {
    if run.id() != scope.provider_run_id
        || run.session_id() != scope.session_id
        || run.agent_instance_id() != Some(scope.agent_id.as_str())
        || run.provider() != "codex"
        || run.adapter_key() != "codex"
        || run.endpoint_mode() != crate::provider::AgentEndpointMode::Managed
        || run.state() != ProviderRunState::Running
        || run.client_interface() != ProviderClientInterface::NativeTui
        || run.structured_endpoint() != Some(scope.endpoint.as_str())
    {
        return Err(error("native owning run or endpoint changed; reattach"));
    }
    Ok(())
}

impl CommandRouter {
    pub(super) async fn native_provider_transport(
        &self,
        request: NativeProviderTransportRequest,
        caller_user_id: &str,
    ) -> Result<NativeProviderTransportResult, DaemonError> {
        self.runtime_state.authorize_current_external_command()?;
        let (scope, remote) = {
            let mut app = self.app.lock().await;
            let attachment = app.attachments().get_attachment(&request.attachment_id)?;
            if attachment.session_id() != request.session_id
                || attachment.owner_user_id() != caller_user_id
            {
                return Err(error("native attachment session or owner mismatch"));
            }
            let agent = app.agents().get_agent(&request.agent_id)?;
            if agent.owner_user_id() != caller_user_id {
                return Err(error("native agent belongs to another user"));
            }
            if agent.session_id() != request.session_id {
                return Err(error("native agent is not in the session"));
            }
            if let Some(binding) = agent.remote_execution().cloned() {
                app.ensure_remote_agent_binding_protocol(&binding)?;
                let run = self
                    .provider_run_projection
                    .get_for_agent(&request.session_id, &request.agent_id)
                    .ok_or_else(|| error("native projected run not found"))?;
                let worker_run_id =
                    crate::provider::worker_provider_run_id_from_projected_leased_id(
                        &binding.leased_agent_id,
                        &request.provider_run_id,
                    )
                    .ok_or_else(|| error("native projected run is not lease bound"))?;
                if run.id() != request.provider_run_id
                    || run.session_id() != request.session_id
                    || run.agent_instance_id() != Some(request.agent_id.as_str())
                    || run.owner_user_id() != caller_user_id
                    || run.provider() != "codex"
                    || run.adapter_key() != "codex"
                    || run.endpoint_mode() != crate::provider::AgentEndpointMode::Managed
                    || run.state() != ProviderRunState::Running
                    || run.client_interface() != ProviderClientInterface::NativeTui
                    || binding.active_worker_provider_run_id.as_deref()
                        != Some(worker_run_id.as_str())
                {
                    return Err(error("native projected run is stale or not the owning run"));
                }
                let config = app.relay_config_for_remote_execution(&binding);
                (
                    None,
                    Some((
                        config,
                        binding.worker_kernel_id,
                        binding.leased_agent_id,
                        worker_run_id,
                    )),
                )
            } else {
                let run = app.providers().get_run(&request.provider_run_id)?;
                let scope = SocketScope {
                    session_id: request.session_id.clone(),
                    agent_id: request.agent_id.clone(),
                    attachment_id: request.attachment_id.clone(),
                    provider_run_id: request.provider_run_id.clone(),
                    leased_agent_id: None,
                    endpoint: run.structured_endpoint().unwrap_or_default().into(),
                };
                local_scope(&mut app, &scope)?;
                (Some(scope), None)
            }
        };
        if let Some((config, worker_id, leased_agent_id, provider_run_id)) = remote {
            let target = chariox_relay::protocol::ClientTarget {
                daemon_id: Some(worker_id),
                daemon_alias: None,
            };
            let peer_request = RelayPeerRequest::NativeProviderTransport {
                leased_agent_id,
                provider_run_id,
                attachment_id: request.attachment_id,
                operation: request.operation,
            };
            let timeout = Duration::from_millis(config.relay_request_timeout_ms);
            let response = match self.runtime_state.connected_relay_state_for_config(&config).await {
                Some(state) => crate::transport::relay_client::send_peer_request_via_connected_relay_authorized(&config, &state, target, peer_request, timeout, || self.runtime_state.authorize_current_external_command()).await?,
                None => crate::transport::relay_client::send_peer_request_via_temporary_connection_authorized(&config, target, peer_request, timeout, || self.runtime_state.authorize_current_external_command()).await?,
            };
            return match response {
                RelayPeerResponse::NativeProviderTransport { result } => Ok(result),
                _ => Err(error(
                    "worker requires native provider transport protocol 134; update and rebind",
                )),
            };
        }
        self.execute_native_socket(scope.expect("local native scope"), request.operation)
            .await
    }

    pub(crate) async fn relay_native_provider_transport(
        &self,
        leased_agent_id: String,
        provider_run_id: String,
        attachment_id: String,
        operation: NativeProviderTransportOperation,
    ) -> Result<NativeProviderTransportResult, DaemonError> {
        let scope = {
            let mut app = self.app.lock().await;
            let (session_id, agent_id) = {
                let mut lease = RemoteLeaseRuntime::new(&mut app);
                lease.consume_leased_agent_authorization(&leased_agent_id)?;
                lease.native_transport_binding(&leased_agent_id)?
            };
            let run = app.providers().get_run(&provider_run_id)?;
            let scope = SocketScope {
                session_id,
                agent_id,
                attachment_id,
                provider_run_id,
                leased_agent_id: Some(leased_agent_id),
                endpoint: run.structured_endpoint().unwrap_or_default().into(),
            };
            local_scope(&mut app, &scope)?;
            scope
        };
        self.execute_native_socket(scope, operation).await
    }

    async fn execute_native_socket(
        &self,
        scope: SocketScope,
        operation: NativeProviderTransportOperation,
    ) -> Result<NativeProviderTransportResult, DaemonError> {
        let app = self.app.clone();
        let guard_scope = scope.clone();
        self.native_provider_sockets
            .execute(scope, operation, move || {
                let app = app.clone();
                let scope = guard_scope.clone();
                async move { local_scope(&mut *app.lock().await, &scope) }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_native_transport_rejects_other_session_agent_run_and_endpoint() {
        use crate::provider::{
            AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
        };
        let request = LaunchProviderRequest::new("room", "codex", "codex", "profile", "gpt-6-luna")
            .with_agent_id("agent")
            .with_client_interface(ProviderClientInterface::NativeTui);
        let mut run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "official-codex".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("ws://127.0.0.1:1234".into()),
            },
        );
        let scope = SocketScope {
            session_id: "room".into(),
            agent_id: "agent".into(),
            attachment_id: "attachment".into(),
            provider_run_id: "run".into(),
            leased_agent_id: None,
            endpoint: "ws://127.0.0.1:1234".into(),
        };
        assert!(validate_owning_run(&run, &scope).is_err());
        run.mark_running();
        assert!(validate_owning_run(&run, &scope).is_ok());
        for field in 0..4 {
            let mut foreign = scope.clone();
            match field {
                0 => foreign.session_id = "other".into(),
                1 => foreign.agent_id = "other".into(),
                2 => foreign.provider_run_id = "stale".into(),
                _ => foreign.endpoint = "ws://127.0.0.1:4321".into(),
            }
            assert!(validate_owning_run(&run, &foreign).is_err());
        }
    }
}
