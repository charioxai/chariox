use std::time::Duration;

use tokio::time::sleep;

use crate::error::DaemonError;
use crate::local::{
    ListProviderProcessesRequest, LocalDaemonRequest, LocalDaemonResponse,
    TeardownProviderProcessesRequest,
};
use crate::provider::ProviderProcessInfo;
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;

pub(crate) async fn execute_provider_process_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    agent_runtime_projection: &AgentRuntimeProjectionStore,
    provider_process_projection: &ProviderProcessProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListProviderProcesses(request) => {
            execute_list_provider_processes_request(
                runtime_state,
                provider_process_projection,
                request,
            )
            .await
        }
        LocalDaemonRequest::TeardownProviderProcesses(request) => {
            execute_teardown_provider_processes_request(
                runtime_state,
                session_projection,
                agent_runtime_projection,
                provider_process_projection,
                provider_run_projection,
                caller_user_id,
                request,
            )
            .await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "provider process request",
            message: "unsupported provider process request".to_string(),
        }),
    }
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

pub(crate) fn provider_processes_visible_to_user_from_projection(
    processes: Vec<ProviderProcessInfo>,
    provider_run_projection: &ProviderRunProjectionStore,
    caller_user_id: &str,
) -> Vec<ProviderProcessInfo> {
    provider_processes_visible_to_user(processes, caller_user_id, |run_id, user_id| {
        provider_run_projection
            .get(run_id)
            .is_some_and(|run| run.owned_by(user_id))
    })
}

pub(crate) fn provider_processes_teardownable_by_user_from_projection(
    processes: Vec<ProviderProcessInfo>,
    provider_run_projection: &ProviderRunProjectionStore,
    caller_user_id: &str,
) -> Vec<ProviderProcessInfo> {
    provider_processes_teardownable_by_user(processes, caller_user_id, |run_id, user_id| {
        provider_run_projection
            .get(run_id)
            .is_some_and(|run| run.owned_by(user_id))
    })
}

pub(crate) fn provider_processes_teardownable_by_user(
    processes: Vec<ProviderProcessInfo>,
    caller_user_id: &str,
    mut provider_run_owned_by: impl FnMut(&str, &str) -> bool,
) -> Vec<ProviderProcessInfo> {
    processes
        .into_iter()
        .filter(|process| {
            !process.owner_provider_run_ids.is_empty()
                && process
                    .owner_provider_run_ids
                    .iter()
                    .all(|run_id| provider_run_owned_by(run_id, caller_user_id))
        })
        .collect()
}

pub(crate) fn projected_provider_processes_response(
    provider_process_projection: &ProviderProcessProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
    request: &ListProviderProcessesRequest,
    caller_user_id: &str,
) -> Option<LocalDaemonResponse> {
    provider_process_projection
        .list(request.provider.as_deref())
        .map(|processes| LocalDaemonResponse::ProviderProcessesListed {
            processes: provider_processes_visible_to_user_from_projection(
                processes,
                provider_run_projection,
                caller_user_id,
            ),
        })
}

pub(crate) async fn execute_list_provider_processes_request(
    runtime_state: &KernelRuntimeState,
    provider_process_projection: &ProviderProcessProjectionStore,
    request: ListProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let list = runtime_state.list_provider_processes(request.provider.as_deref());
    provider_process_projection.update_list(list.canonical_processes);
    if list.delay_ms > 0 {
        sleep(Duration::from_millis(list.delay_ms)).await;
    }
    Ok(LocalDaemonResponse::ProviderProcessesListed {
        processes: list.filtered_processes,
    })
}

pub(crate) async fn teardown_provider_processes(
    runtime_state: &KernelRuntimeState,
    provider_run_projection: &ProviderRunProjectionStore,
    caller_user_id: &str,
    request: TeardownProviderProcessesRequest,
) -> Result<crate::runtime::state::ProviderProcessTeardown, DaemonError> {
    let allowed_process_ids = provider_processes_teardownable_by_user_from_projection(
        runtime_state
            .list_provider_processes(request.provider.as_deref())
            .filtered_processes,
        provider_run_projection,
        caller_user_id,
    )
    .into_iter()
    .map(|process| process.process_id)
    .collect::<std::collections::HashSet<_>>();
    runtime_state
        .teardown_provider_processes(
            request.provider.as_deref(),
            request.force,
            Some(&allowed_process_ids),
        )
        .await
}

pub(crate) async fn execute_teardown_provider_processes_request(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    agent_runtime_projection: &AgentRuntimeProjectionStore,
    provider_process_projection: &ProviderProcessProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
    caller_user_id: &str,
    request: TeardownProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let teardown = teardown_provider_processes(
        runtime_state,
        provider_run_projection,
        caller_user_id,
        request,
    )
    .await?;
    for session in &teardown.sessions {
        agent_runtime_projection.update_session(session);
        session_projection.update(session.clone());
    }
    provider_process_projection.update_list(teardown.canonical_processes);
    Ok(LocalDaemonResponse::ProviderProcessesTornDown {
        processes: teardown.processes,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::provider::{AgentEndpointMode, ProviderProcessInfo, ProviderProcessStatus};
    use crate::runtime::provider_process_control::{
        provider_processes_teardownable_by_user, provider_processes_visible_to_user,
    };

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

    #[test]
    fn provider_process_teardown_requires_all_runs_owned_by_caller() {
        let owned_runs = HashSet::from(["run-owned".to_string()]);
        let teardownable = provider_processes_teardownable_by_user(
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
            teardownable
                .iter()
                .map(|process| process.process_id.as_str())
                .collect::<Vec<_>>(),
            vec!["process-owned"]
        );
    }

    fn process(process_id: &str, owner_provider_run_ids: Vec<&str>) -> ProviderProcessInfo {
        ProviderProcessInfo {
            process_id: process_id.to_string(),
            provider: "codex".to_string(),
            process_label: process_id.to_string(),
            pid: None,
            resident_set_bytes: None,
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
