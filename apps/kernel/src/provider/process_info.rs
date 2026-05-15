use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::session::unix_epoch_ms;

use super::runtime_run::RuntimeProviderRun;
use super::types::{AgentEndpointMode, ProviderRunState};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProcessStatus {
    Active,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProcessInfo {
    pub process_id: String,
    pub provider: String,
    pub process_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub endpoint_mode: AgentEndpointMode,
    pub status: ProviderProcessStatus,
    pub started_at_ms: u64,
    pub last_activity_at_ms: u64,
    #[serde(default)]
    pub provider_session_ids: Vec<String>,
    #[serde(default)]
    pub owner_session_ids: Vec<String>,
    #[serde(default)]
    pub owner_provider_run_ids: Vec<String>,
    #[serde(default)]
    pub attached_session_ids: Vec<String>,
    #[serde(default)]
    pub active_workflow_run_ids: Vec<String>,
    pub teardown_safe: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown_blockers: Vec<String>,
}

impl ProviderProcessInfo {
    pub fn from_runs(
        process_id: String,
        runs: &[RuntimeProviderRun],
        attached_session_ids: BTreeSet<String>,
        active_workflow_run_ids: BTreeSet<String>,
        teardown_safe: bool,
        teardown_blockers: Vec<String>,
    ) -> Option<Self> {
        let first = runs
            .iter()
            .find(|run| run.endpoint_mode() == AgentEndpointMode::Managed)
            .or_else(|| runs.first())?;
        let status = if runs.iter().any(|run| {
            matches!(
                run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            )
        }) {
            ProviderProcessStatus::Active
        } else {
            ProviderProcessStatus::Idle
        };
        let started_at_ms = runs
            .iter()
            .map(RuntimeProviderRun::started_at_ms)
            .min()
            .unwrap_or_else(unix_epoch_ms);
        let last_activity_at_ms = runs
            .iter()
            .map(RuntimeProviderRun::last_activity_at_ms)
            .max()
            .unwrap_or_else(unix_epoch_ms);
        let provider_session_ids = runs
            .iter()
            .filter_map(|run| {
                run.provider_session_id().map(str::to_string).or_else(|| {
                    run.resume_state()
                        .opencode_session_id()
                        .or_else(|| run.resume_state().codex_thread_id())
                        .or_else(|| run.resume_state().claude_session_id())
                        .map(str::to_string)
                })
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let owner_session_ids = runs
            .iter()
            .map(|run| run.session_id().to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let owner_provider_run_ids = runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        Some(Self {
            process_id,
            provider: first.provider().to_string(),
            process_label: first.process_label().to_string(),
            pid: None,
            endpoint_mode: first.endpoint_mode(),
            status,
            started_at_ms,
            last_activity_at_ms,
            provider_session_ids,
            owner_session_ids,
            owner_provider_run_ids,
            attached_session_ids: attached_session_ids.into_iter().collect(),
            active_workflow_run_ids: active_workflow_run_ids.into_iter().collect(),
            teardown_safe,
            teardown_blockers,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{ProviderProcessInfo, ProviderProcessStatus};
    use crate::provider::RuntimeProviderRun;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    #[test]
    fn provider_process_info_prefers_explicit_provider_session_ids() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default");
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "codex".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let mut run = RuntimeProviderRun::new("provider-run-1", &request, launch_result);
        run.set_provider_session_id(Some("thread-123".to_string()));
        run.mark_running();
        let info = ProviderProcessInfo::from_runs(
            "process-1".to_string(),
            &[run],
            BTreeSet::new(),
            BTreeSet::new(),
            true,
            Vec::new(),
        )
        .expect("process info should be built");
        assert_eq!(info.status, ProviderProcessStatus::Active);
        assert_eq!(info.provider_session_ids, vec!["thread-123".to_string()]);
    }
}
