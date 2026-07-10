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
        let pty_target = request.agent_id.as_deref().map_or_else(
            || format!("stub-pty:{}", request.session_id),
            |agent_id| format!("stub-pty:{}:{agent_id}", request.session_id),
        );
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
