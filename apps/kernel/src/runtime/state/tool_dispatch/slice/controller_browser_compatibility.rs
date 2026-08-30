use crate::error::DaemonError;
use crate::runtime::browser_controller_compatibility::{
    BrowserCompatibilityWait, DEFAULT_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS,
};
use crate::runtime::state::KernelRuntimeState;

use super::controller_browser::ensure_controller_browser_environment;

impl KernelRuntimeState {
    pub(super) async fn controller_browser_open_url_compatibility_tool_result(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        slice_id: &str,
        agent_id: &str,
        url: &str,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let session_id = provider_run.session_id();
        ensure_controller_browser_environment(self, session_id, "runtime_tool_slice_open_url")
            .await?;
        let result = self
            .navigate_browser_environment_compatibility(session_id, url)
            .await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "source": "browser_controller",
                "slice_id": slice_id,
                "agent_id": agent_id,
                "session_id": session_id,
                "browser": {
                    "action_kind": "navigate",
                    "url": result.url,
                    "document_id": result.document_id,
                    "browser_generation": result.browser_generation,
                },
            }),
        })
    }

    pub(super) async fn controller_browser_wait_for_selector_compatibility_tool_result(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        slice_id: &str,
        agent_id: &str,
        selector: String,
        timeout_ms: Option<u64>,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.controller_browser_wait_compatibility_tool_result(
            provider_run,
            slice_id,
            agent_id,
            BrowserCompatibilityWait::Selector(selector),
            timeout_ms,
        )
        .await
    }

    pub(super) async fn controller_browser_wait_for_idle_compatibility_tool_result(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        slice_id: &str,
        agent_id: &str,
        timeout_ms: Option<u64>,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.controller_browser_wait_compatibility_tool_result(
            provider_run,
            slice_id,
            agent_id,
            BrowserCompatibilityWait::Idle,
            timeout_ms,
        )
        .await
    }

    async fn controller_browser_wait_compatibility_tool_result(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        slice_id: &str,
        agent_id: &str,
        wait: BrowserCompatibilityWait,
        timeout_ms: Option<u64>,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let session_id = provider_run.session_id();
        ensure_controller_browser_environment(
            self,
            session_id,
            "runtime_tool_slice_browser_wait_compatibility",
        )
        .await?;
        let result = self
            .wait_for_browser_environment_compatibility(
                session_id,
                wait,
                timeout_ms.unwrap_or(DEFAULT_BROWSER_COMPATIBILITY_WAIT_TIMEOUT_MS),
            )
            .await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: result.ok,
            payload: serde_json::json!({
                "source": "browser_controller",
                "slice_id": slice_id,
                "agent_id": agent_id,
                "session_id": session_id,
                "browser": {
                    "action_kind": result.kind,
                    "ok": result.ok,
                    "elapsed_ms": result.elapsed_ms,
                    "document_id": result.document_id,
                    "browser_generation": result.browser_generation,
                },
            }),
        })
    }
}
