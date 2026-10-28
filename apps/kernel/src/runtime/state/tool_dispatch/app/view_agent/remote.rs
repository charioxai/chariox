use super::*;

const REMOTE_NAME: &str = "chariox.app_views";

impl KernelRuntimeState {
    pub(crate) fn remote_app_view_tools(
        agent: &crate::agent::AgentInstance,
    ) -> Vec<RemoteExtensionTool> {
        if agent.archived()
            || !agent
                .extension_grants()
                .iter()
                .any(|grant| grant.kind == ExtensionKind::App)
        {
            return Vec::new();
        }
        specs()
            .into_iter()
            .map(|spec| RemoteExtensionTool {
                kind: ExtensionKind::App,
                name: REMOTE_NAME.into(),
                tool_name: spec.name,
                description: spec.description,
                input_schema: spec.input_schema,
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: Some(APPROVAL_TIMEOUT_SEC + 30),
                version_hash: Some("1".into()),
            })
            .collect()
    }

    pub(in crate::runtime::state::tool_dispatch::app) fn authorize_forwarded_app_view_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        hinted: &RemoteExtensionTool,
    ) -> Option<Result<RemoteExtensionTool, DaemonError>> {
        if !TOOLS.contains(&hinted.tool_name.as_str()) && hinted.name != REMOTE_NAME {
            return None;
        }
        Some((|| {
            let agent =
                super::super::super::home_extension_authorizer::HomeExtensionAuthorizationService::new(
                    self,
                )
                .authorize_invocation_context(context)?;
            Self::remote_app_view_tools(&agent)
                .into_iter()
                .find(|tool| tool == hinted)
                .ok_or_else(unavailable)
        })())
    }

    pub(in crate::runtime::state::tool_dispatch::app) async fn dispatch_home_app_view_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &RemoteExtensionTool,
        arguments: Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        let authorization = self.authorize_forwarded_app_view_tool(context, tool)?;
        Some(async {
            authorization?;
            let agent = super::super::super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
                .authorize_invocation_context(context)?;
            let run_id = crate::provider::projected_leased_provider_run_id(&context.leased_agent_id, &context.worker_provider_run_id);
            Ok(view_result(self.app_view_tool(&run_id, &agent, &tool.tool_name, arguments).await))
        }.await)
    }
}
