//! MD-2/MD-3: routing and focused-agent MCP adapter for the host service.
use super::*;
use crate::local::KernelBrowserCommand;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

const LOADER: &str = "chariox.load_kernel_browser";
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
        #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
        if matches!(command, KernelBrowserCommand::Stop) {
            for view in self.app_control().user_views().browser_views(&user) {
                self.forget_user_app_view(&user, &view.view_id);
            }
        }
        let host = self.owned.kernel_browser_host.clone();
        tokio::task::spawn_blocking(move || host.request(&user, command))
            .await
            .map_err(|_| host_error("MD-2: browser task failed".into()))?
            .map_err(host_error)
    }
    /// MD-2: appviews lane uses this seam after its installation/bridge admission.
    pub(crate) async fn kernel_browser_app_view(
        &self,
        user: String,
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
        generation: Option<u64>,
    ) -> Result<serde_json::Value, DaemonError> {
        let host = self.owned.kernel_browser_host.clone();
        tokio::task::spawn_blocking(move || host.app_view(&user, &request, generation))
            .await
            .map_err(|_| host_error("MD-2: App view task failed".into()))?
            .map_err(host_error)
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
            && self
                .owned
                .kernel_browser_host
                .is_focused(agent.owner_user_id(), id))
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
        if self
            .owned
            .kernel_browser_host
            .is_loaded(agent.owner_user_id(), agent.id())
        {
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
        if ![LOADER, BROWSER].contains(&name) {
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
            host.load(agent.owner_user_id(), agent.id())
                .map_err(host_error)?;
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"loaded": BROWSER}),
            });
        }
        let request: crate::local::KernelBrowserRequest = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-3: invalid browser command".into()))?;
        let user = agent.owner_user_id().to_string();
        let id = agent.id().to_string();
        let payload =
            tokio::task::spawn_blocking(move || host.request_as(&user, Some(&id), request.command))
                .await
                .map_err(|_| host_error("MD-3: browser task failed".into()))?
                .map_err(host_error)?;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
fn host_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "kernel_browser",
        message,
    }
}
