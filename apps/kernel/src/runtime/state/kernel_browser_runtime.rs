//! MD-2/MD-3: routing and focused-agent MCP adapter for the host service.
use super::*;
use crate::local::KernelBrowserCommand;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

const LOADER: &str = "chariox.load_kernel_browser";
pub(super) const PASTE: &str = "chariox.kernel_browser_paste_secret";
const BROWSER: &str = "chariox.kernel_browser";

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn kernel_browser_profile_root(&self, user: &str) -> std::path::PathBuf {
        self.owned.kernel_browser_host.profile_root(user)
    }

    pub(crate) async fn kernel_browser_request(
        &self,
        user: String,
        command: KernelBrowserCommand,
    ) -> Result<serde_json::Value, DaemonError> {
        let user = self.provider_account_authority_owner_user_id(&user);
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if matches!(command, KernelBrowserCommand::Stop) {
            for view in self.app_control().user_views().browser_views(&user) {
                self.forget_user_app_view(&user, &view.view_id);
            }
        }
        self.kernel_browser_operation(
            &user,
            None,
            "host.browser",
            serde_json::to_value(command)
                .map_err(|_| host_error("MD-2: invalid command".into()))?,
        )
        .await
    }
    /// MD-2/MD-5: appviews retains admission; observations use the same protection.
    pub(crate) async fn kernel_browser_app_view(
        &self,
        user: String,
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
        generation: Option<u64>,
    ) -> Result<serde_json::Value, DaemonError> {
        let user = self.provider_account_authority_owner_user_id(&user);
        let mut params = request.params();
        if let Some(generation) = generation {
            params["_host_generation"] = generation.into();
        }
        self.kernel_browser_operation(&user, None, request.method(), params)
            .await
    }
    fn kernel_browser_agent(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Option<crate::agent::AgentInstance> {
        let id = run.agent_instance_id()?;
        let agent = self.owned.agent_store.get_agent(id).ok()?;
        if agent.remote_execution().is_some()
            || self
                .owned
                .provider_run_projection
                .is_leased_provider_run(run.id())
            || self.slice_kernel_id().is_some()
        {
            return None;
        }
        // Session focus also protects destruction/move and other existing focus paths.
        let session = self.owned.session_snapshot(run.session_id()).ok()?;
        (session.focused_agent_id() == Some(id)
            && self.owned.kernel_browser_host.is_focused(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
                id,
            ))
        .then_some(agent)
    }
    pub(super) fn kernel_browser_tool_specs(
        &self,
        runs: &[crate::provider::RuntimeProviderRun],
    ) -> Vec<RuntimeToolSpec> {
        let [run] = runs else {
            return Vec::new();
        };
        let Some(agent) = self.kernel_browser_agent(run) else {
            return Vec::new();
        };
        let mut tools = vec![RuntimeToolSpec { name: LOADER.into(),
            description: "MD-3: load user-domain browser tools on demand. Only the user's focused local agent has access.".into(),
            input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}), }];
        if self.owned.kernel_browser_host.is_loaded(
            &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            agent.id(),
        ) {
            tools.push(super::kernel_browser_secret_runtime::paste_spec());
            tools.push(RuntimeToolSpec { name: BROWSER.into(),
                description: "MD-3: control the user's kernel browser outside sessions/slices. Read state to obtain tab IDs and generation; input accepts click/text/key/scroll. Vault input is separate.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"command":{"type":"object","properties":{"op":{"type":"string","enum":["start","state","stop","open","close","navigate","snapshot","input","screenshot","subscribe","poll","unsubscribe"]},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"url":{"type":"string"},"subscription_id":{"type":"string"},"input":{"type":"object","properties":{"kind":{"type":"string","enum":["click","text","key","scroll"]},"x":{"type":"integer","minimum":0,"maximum":1279},"y":{"type":"integer","minimum":0,"maximum":799},"text":{"type":"string","maxLength":16384},"key":{"type":"string","enum":["Tab","Enter","Escape","Backspace","Delete","ArrowLeft","ArrowRight","ArrowUp","ArrowDown","Home","End"]},"delta_x":{"type":"integer","minimum":-10000,"maximum":10000},"delta_y":{"type":"integer","minimum":-10000,"maximum":10000}},"required":["kind"],"additionalProperties":false}},"required":["op"],"additionalProperties":false}},"required":["command"],"additionalProperties":false}), });
        }
        tools
    }
    pub(super) async fn try_kernel_browser_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        if ![LOADER, BROWSER, PASTE].contains(&name) {
            return None;
        }
        Some(self.kernel_browser_tool(token, name, arguments).await)
    }
    async fn kernel_browser_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(host_error(
                "MD-3: admitted local provider run required".into(),
            ));
        };
        let agent = self
            .kernel_browser_agent(run)
            .ok_or_else(|| host_error("MD-3: current local focus required".into()))?;
        let host = self.owned.kernel_browser_host.clone();
        if name == LOADER {
            if arguments
                .as_object()
                .is_none_or(|object| !object.is_empty())
            {
                return Err(host_error("MD-3: loader accepts no arguments".into()));
            }
            host.load(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
                agent.id(),
            )
            .map_err(host_error)?;
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"loaded": BROWSER}),
            });
        }
        if name == PASTE {
            let scope = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            );
            return self
                .kernel_browser_paste_secret(run.session_id(), &agent, arguments)
                .await
                .map_err(|error| {
                    self.owned
                        .kernel_browser_secret_observations
                        .scrub_error(&scope, error)
                });
        }
        let request: crate::local::KernelBrowserRequest = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-3: invalid browser command".into()))?;
        let payload = self
            .kernel_browser_operation(
                &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
                Some(agent.id()),
                "host.browser",
                serde_json::to_value(request.command)
                    .map_err(|_| host_error("MD-3: invalid command".into()))?,
            )
            .await?;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
pub(super) fn host_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "kernel_browser",
        message,
    }
}
