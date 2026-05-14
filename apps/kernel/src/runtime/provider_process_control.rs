use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{
    ListProviderProcessesRequest, LocalDaemonResponse, TeardownProviderProcessesRequest,
};
use crate::provider::ProviderProcessInfo;
use crate::session::RuntimeSession;

pub(crate) struct ProviderProcessTeardown {
    pub(crate) processes: Vec<ProviderProcessInfo>,
    pub(crate) sessions: Vec<RuntimeSession>,
}

pub(crate) fn provider_processes_visible_to_user(
    processes: Vec<ProviderProcessInfo>,
    caller_user_id: &str,
    mut provider_run_owned_by: impl FnMut(&str, &str) -> bool,
) -> Vec<ProviderProcessInfo> {
    processes
        .into_iter()
        .filter(|process| {
            process
                .owner_provider_run_ids
                .iter()
                .any(|run_id| provider_run_owned_by(run_id, caller_user_id))
        })
        .collect()
}

pub(crate) async fn execute_list_provider_processes_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (processes, delay_ms) = {
        let app = app.lock().await;
        (
            app.list_provider_processes(request.provider.as_deref())?,
            app.config().provider_process_list_delay_ms,
        )
    };
    if delay_ms > 0 {
        sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(LocalDaemonResponse::ProviderProcessesListed { processes })
}

pub(crate) async fn teardown_provider_processes(
    app: &Arc<Mutex<DaemonApp>>,
    request: TeardownProviderProcessesRequest,
) -> Result<ProviderProcessTeardown, DaemonError> {
    let mut app = app.lock().await;
    let processes = app.teardown_provider_processes(request.provider.as_deref(), request.force)?;
    let session_ids = processes
        .iter()
        .flat_map(|process| process.owner_session_ids.iter())
        .cloned()
        .collect::<HashSet<_>>();
    let sessions = session_ids
        .into_iter()
        .filter_map(|session_id| {
            crate::app::KernelSessionReadService::new(&app)
                .session_snapshot(&session_id)
                .ok()
        })
        .collect::<Vec<_>>();
    Ok(ProviderProcessTeardown {
        processes,
        sessions,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::provider::{AgentEndpointMode, ProviderProcessInfo, ProviderProcessStatus};
    use crate::runtime::provider_process_control::provider_processes_visible_to_user;

    #[test]
    fn provider_process_visibility_follows_owned_provider_runs() {
        let owned_runs = HashSet::from(["run-owned".to_string()]);
        let visible = provider_processes_visible_to_user(
            vec![
                process("process-owned", vec!["run-owned"]),
                process("process-shared", vec!["run-other", "run-owned"]),
                process("process-foreign", vec!["run-other"]),
                process("process-orphan", vec![]),
            ],
            "user-1",
            |run_id, caller_user_id| caller_user_id == "user-1" && owned_runs.contains(run_id),
        );

        assert_eq!(
            visible
                .iter()
                .map(|process| process.process_id.as_str())
                .collect::<Vec<_>>(),
            vec!["process-owned", "process-shared"]
        );
    }

    fn process(process_id: &str, owner_provider_run_ids: Vec<&str>) -> ProviderProcessInfo {
        ProviderProcessInfo {
            process_id: process_id.to_string(),
            provider: "codex".to_string(),
            process_label: process_id.to_string(),
            pid: None,
            endpoint_mode: AgentEndpointMode::Managed,
            status: ProviderProcessStatus::Active,
            started_at_ms: 1,
            last_activity_at_ms: 2,
            provider_session_ids: vec![],
            owner_session_ids: vec![],
            owner_provider_run_ids: owner_provider_run_ids
                .into_iter()
                .map(str::to_string)
                .collect(),
            attached_session_ids: vec![],
            active_workflow_run_ids: vec![],
            teardown_safe: true,
            teardown_blockers: vec![],
        }
    }
}
