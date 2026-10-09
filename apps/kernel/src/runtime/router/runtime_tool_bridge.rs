use super::CommandRouter;
use crate::error::DaemonError;

// MP-08 / MP-10 / MP-11: callers retain one pointer rather than every tool branch future.
type AuthenticatedRuntimeToolFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError>,
            > + Send
            + 'a,
    >,
>;

impl CommandRouter {
    pub(crate) fn with_forwarded_response_authority<R>(
        &self,
        operation: impl FnOnce() -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        self.runtime_state
            .with_forwarded_binding_operation(operation)
    }

    pub(crate) async fn authorize_forwarded_peer_request(
        &self,
        request: &crate::transport::relay_peer::RelayPeerRequest,
    ) -> Result<Self, DaemonError> {
        let mut router = self.clone();
        router.runtime_state = self
            .runtime_state
            .prepare_forwarded_peer_request(request)
            .await?;
        Ok(router)
    }

    pub(crate) fn with_relay_peer_authority(
        &self,
        peer: crate::runtime::relay_peer_authority::RelayPeerAuthority,
    ) -> Self {
        let mut router = self.clone();
        router.runtime_state = router.runtime_state.with_relay_peer_authority(peer);
        router
    }

    /// Notification delivery cannot broaden token scope. A stream belongs to
    /// exactly one still-live run; normal tool authorization remains unchanged.
    pub(crate) fn runtime_mcp_catalog_run(
        &self,
        token: &str,
    ) -> Option<crate::provider::RuntimeProviderRun> {
        let runs = self
            .provider_run_projection
            .active_runs_by_runtime_mcp_auth_token(token);
        (runs.len() == 1).then(|| runs[0].clone())
    }

    pub(crate) fn runtime_mcp_catalog_changes(
        &self,
    ) -> crate::runtime::runtime_tool_catalog::RuntimeToolCatalogChanges {
        self.provider_run_projection.catalog_changes().clone()
    }

    pub(crate) fn runtime_tool_catalog_auth_tokens(&self) -> Vec<String> {
        self.runtime_state.runtime_tool_catalog_auth_tokens()
    }

    pub(crate) fn runtime_tool_catalog_changed_for_auth_token(&self, token: &str) {
        self.runtime_state
            .runtime_tool_catalog_changed_for_auth_token(token);
    }

    pub(crate) fn runtime_mcp_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.runtime_mcp_host, config.runtime_mcp_port)
    }

    pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
        &self,
        auth_token: &str,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        self.runtime_state
            .dispatch_authenticated_mcp_proxy_call(
                &self.provider_run_projection,
                auth_token,
                name,
                payload,
            )
            .await
    }

    #[inline(never)]
    pub(crate) fn dispatch_authenticated_runtime_tool_call<'a>(
        &'a self,
        auth_token: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> AuthenticatedRuntimeToolFuture<'a> {
        Box::pin(async move {
            if crate::transport::runtime_tools::canonical_room_tool_name(tool_name).is_some()
                && !self.runtime_state.room_agent_tools_enabled()
            {
                return Err(crate::runtime::room_tool_admission::denied(
                    "room agent tools are disabled",
                ));
            }
            // A Claude run is silent while it waits on a runtime tool call, and a
            // person's decision (a permission prompt, a popup, an App binding
            // approval, also through Meta `run_command`) can take minutes: its turn
            // stall watchdog must not end the turn meanwhile.
            let _claude_waits = self
                .runtime_state
                .begin_claude_runtime_tool_waits(auth_token);
            if let Some(canonical) =
                crate::transport::runtime_tools::canonical_meta_tool_name(tool_name)
            {
                if let Some(run) = self.runtime_mcp_catalog_run(auth_token) {
                    if let Some(result) = self
                        .runtime_state
                        .try_dispatch_remote_meta_runtime_tool_call(
                            &run,
                            canonical,
                            arguments.clone(),
                        )
                        .await?
                    {
                        return Ok(result);
                    }
                }
            }
            if tool_name == "chariox_kernel_request" {
                let turn = self.runtime_state.sudo_for_auth_token(auth_token)?;
                let request: crate::local::LocalDaemonRequest =
                    serde_json::from_value(arguments.get("request").cloned().ok_or_else(|| {
                        crate::runtime::kernel_access::error("kernel_request needs request")
                    })?)
                    .map_err(|_| crate::runtime::kernel_access::error("invalid kernel request"))?;
                self.runtime_state
                    .authorize_sudo_request(&turn.entry_id, &request)?;
                let mut command = crate::runtime::command::KernelCommand::from_local_request(
                    format!("{}:{}", turn.entry_id, rand::random::<u64>()),
                    turn.prompt_id.clone(),
                    Some(turn.entry_id.clone()),
                    &request,
                );
                command.caller = crate::runtime::command::KernelCaller::for_source(
                    &crate::runtime::command::KernelCommandSource::LocalIpc,
                )
                .with_connection_class(crate::local::KernelConnectionClass::KernelAgent);
                command.caller.caller_id = turn.entry_id;
                command.caller.user_id = Some(turn.owner_user_id);
                // MP-08 / MP-11: the exact live sudo turn is the authority here.
                // Ordinary room-agent restrictions must not narrow this host grant;
                // dispatch still rechecks its forbidden operations and revocation.
                let response = Box::pin(self.dispatch(command, request)).await?;
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(response).map_err(|_| {
                        crate::runtime::kernel_access::error("kernel response serialization failed")
                    })?,
                });
            }
            if crate::transport::runtime_tools::canonical_meta_tool_name(tool_name)
                == Some(crate::transport::runtime_tools::META_RUN_COMMAND_TOOL)
            {
                let mut router = self.clone();
                if let Some(run) = self.runtime_mcp_catalog_run(auth_token) {
                    router.runtime_state = self
                        .runtime_state
                        .with_room_provider_origin(run.agent_instance_id(), Some(run.id()));
                }
                return Box::pin(router.dispatch_meta_run_command(auth_token, arguments)).await;
            }
            Box::pin(
                self.runtime_state
                    .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments),
            )
            .await
        })
    }

    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        self.runtime_state
            .runtime_tool_specs_for_auth_token(auth_token)
    }

    pub(crate) async fn runtime_tool_specs_for_auth_token_async(
        &self,
        auth_token: String,
    ) -> Result<Vec<crate::transport::runtime_tools::RuntimeToolSpec>, DaemonError> {
        self.runtime_state
            .runtime_tool_specs_for_auth_token_async(auth_token)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_workspace_live_sync_runtime_tool_call(
                context,
                metadata,
                tool_name,
                arguments,
                artifact_states,
            )
            .await
    }

    pub(crate) async fn finalize_forwarded_workspace_live_sync_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        metadata: crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        initial_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
        final_artifact_states: Vec<
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState,
        >,
    ) -> Result<(), DaemonError> {
        self.runtime_state
            .finalize_forwarded_workspace_live_sync_runtime_tool_call(
                context,
                metadata,
                tool_name,
                arguments,
                initial_artifact_states,
                final_artifact_states,
            )
            .await
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::CharioxSkillPackage>,
            crate::extension::RemoteExtensionManifest,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_meta_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .authorize_forwarded_workspace_context(&context)?;
        if crate::transport::runtime_tools::canonical_meta_tool_name(&tool_name)
            == Some(crate::transport::runtime_tools::META_RUN_COMMAND_TOOL)
        {
            return self
                .dispatch_forwarded_meta_run_command(context, arguments)
                .await;
        }
        self.runtime_state
            .dispatch_forwarded_meta_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_room_browser_runtime_tool_call(
        &self,
        from_worker_kernel_id: &str,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        call: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolCall,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_room_browser_runtime_tool_call(from_worker_kernel_id, context, call)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_extension_tool_call(context, metadata, tool, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_mcp_proxy_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        tool: crate::extension::RemoteExtensionTool,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_mcp_proxy_call(context, metadata, name, tool, payload)
            .await
    }

    pub(crate) async fn cancel_forwarded_home_extension_invocation(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<bool, DaemonError> {
        self.runtime_state
            .cancel_forwarded_home_extension_invocation(context, metadata)
            .await
    }

    pub(crate) async fn dispatch_forwarded_home_credential_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_home_credential_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn resolve_forwarded_home_credential_secret(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        credential_id: String,
        injection: crate::transport::relay_peer::RemoteCredentialSecretInjection,
    ) -> Result<(String, String), DaemonError> {
        self.runtime_state
            .resolve_forwarded_home_credential_secret(context, credential_id, injection)
            .await
    }
}
