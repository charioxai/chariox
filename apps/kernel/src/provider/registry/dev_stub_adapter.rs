#[cfg(test)]
use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};

use super::AgentEndpointAdapter;

mod scripts;

use scripts::{dev_stub_pty_args, dev_stub_pty_env};

#[derive(Debug, Default)]
pub(super) struct DevStubAdapter;

impl DevStubAdapter {
    pub(super) const KEY: &'static str = "dev-stub";
}

const DISTRIBUTED_SCALE_SHARED_PTY_MODEL: &str = "distributed-scale-shared-pty";

#[cfg(test)]
const RUNTIME_MCP_IDLE_MODEL: &str = "runtime-mcp-idle";

pub(super) static DEV_STUB_ADAPTER: DevStubAdapter = DevStubAdapter;

impl AgentEndpointAdapter for DevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let pty_args = dev_stub_pty_args(request.model.as_str());
        let pty_env = dev_stub_pty_env(request);
        let pty_target = if request.model == DISTRIBUTED_SCALE_SHARED_PTY_MODEL {
            // The distributed scale drill measures Chariox's run/lease/relay overhead, not the
            // host's PTY limit. Each worker multiplexes its synthetic runs through one process,
            // matching structured providers that multiplex many logical sessions per server.
            "stub-pty:distributed-scale".to_string()
        } else {
            request.agent_id.as_deref().map_or_else(
                || format!("stub-pty:{}", request.session_id),
                |agent_id| format!("stub-pty:{}:{agent_id}", request.session_id),
            )
        };
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-stub:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(pty_target),
            pty_program: Some("/bin/sh".to_string()),
            pty_args,
            pty_env,
            pty_env_remove: request.provider_env_remove.clone(),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_bound_stub_runs_use_distinct_pty_targets_within_a_session() {
        let first = DEV_STUB_ADAPTER
            .connect(
                &LaunchProviderRequest::new(
                    "session-1",
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "native-tui-idle",
                )
                .with_agent_id("agent-1"),
            )
            .expect("first stub launch should plan");
        let second = DEV_STUB_ADAPTER
            .connect(
                &LaunchProviderRequest::new(
                    "session-1",
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id("agent-2"),
            )
            .expect("second stub launch should plan");

        assert_eq!(
            first.pty_target.as_deref(),
            Some("stub-pty:session-1:agent-1")
        );
        assert_eq!(
            second.pty_target.as_deref(),
            Some("stub-pty:session-1:agent-2")
        );
    }

    #[test]
    fn distributed_scale_stub_runs_share_the_scale_pty() {
        for agent_id in ["agent-1", "agent-2"] {
            let launch = DEV_STUB_ADAPTER
                .connect(
                    &LaunchProviderRequest::new(
                        "session-1",
                        "dev-stub",
                        "dev-stub",
                        "default",
                        DISTRIBUTED_SCALE_SHARED_PTY_MODEL,
                    )
                    .with_agent_id(agent_id),
                )
                .expect("distributed scale stub launch should plan");

            assert_eq!(
                launch.pty_target.as_deref(),
                Some("stub-pty:distributed-scale")
            );
        }
    }

    #[test]
    fn managed_runtime_mcp_idle_stub_uses_a_non_final_keepalive() {
        let launch = MANAGED_DEV_STUB_ADAPTER
            .connect(&LaunchProviderRequest::new(
                "session-1",
                "managed-dev-stub",
                "dev-stub",
                "default",
                RUNTIME_MCP_IDLE_MODEL,
            ))
            .expect("managed runtime MCP idle launch should plan");
        let script = launch
            .pty_args
            .get(1)
            .expect("managed runtime MCP idle launch should include a shell script");
        assert!(script.starts_with("stty -echo 2>/dev/null || true; trap 'exit 0' TERM INT HUP;"));
        assert!(script.contains("while :"));
        assert!(script.contains("printf '\\033[0m'"));
        assert!(script.contains("sleep 0.02"));
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct ManagedDevStubAdapter;

#[cfg(test)]
impl ManagedDevStubAdapter {
    pub(super) const KEY: &'static str = "managed-dev-stub";
}

#[cfg(test)]
pub(super) static MANAGED_DEV_STUB_ADAPTER: ManagedDevStubAdapter = ManagedDevStubAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for ManagedDevStubAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn supports_workspace_live_sync_write_enforcement(&self) -> bool {
        true
    }

    fn supports_turn_scoped_execution_config(&self) -> bool {
        true
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        let mut launch = DEV_STUB_ADAPTER.connect(request)?;
        if request.model == RUNTIME_MCP_IDLE_MODEL {
            // Keep the synthetic prompt active while a long-running runtime MCP call is in
            // flight. The short SGR reset heartbeat is deliberately non-final output;
            // this is a managed-dev-stub-only fixture, never a production provider behavior.
            launch.pty_args = vec![
                "-lc".to_string(),
                "stty -echo 2>/dev/null || true; trap 'exit 0' TERM INT HUP; while :; do printf '\\033[0m'; sleep 0.02; done".to_string(),
            ];
        }
        launch.process_label = format!(
            "managed-dev-stub:{}:{}:{}",
            request.provider, request.account_profile, request.model
        );
        Ok(launch)
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct FailingPtyAdapter;

#[cfg(test)]
impl FailingPtyAdapter {
    pub(super) const KEY: &'static str = "dev-invalid-pty";
}

#[cfg(test)]
pub(super) static FAILING_PTY_ADAPTER: FailingPtyAdapter = FailingPtyAdapter;

#[cfg(test)]
impl AgentEndpointAdapter for FailingPtyAdapter {
    fn key(&self) -> &'static str {
        Self::KEY
    }

    fn connect(
        &self,
        request: &LaunchProviderRequest,
    ) -> Result<ProviderLaunchResult, DaemonError> {
        Ok(ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: format!(
                "dev-invalid-pty:{}:{}:{}",
                request.provider, request.account_profile, request.model
            ),
            pty_target: Some(format!("invalid-pty:{}", request.session_id)),
            pty_program: Some("/definitely/not/a/real/provider".to_string()),
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: request.provider_env_remove.clone(),
            working_directory: request.working_directory.clone(),
            structured_endpoint: None,
        })
    }

    fn park(&self, _run: &RuntimeProviderRun) {}

    fn resume(&self, _run: &RuntimeProviderRun) {}

    fn terminate(&self, _run: &RuntimeProviderRun) {}
}
