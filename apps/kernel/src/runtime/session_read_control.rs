use std::collections::BTreeMap;

use crate::app::{ActiveTurnStore, PromptActivityStore};
use crate::error::DaemonError;
use crate::local::{
    GetSessionStateRequest, ListAgentsRequest, ListSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse, ResolveSessionRequest,
};
use crate::runtime::projection::{
    agent_activity_for_session_projection, AgentRuntimeActivity, ProviderRunProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::workflow_projection::{
    projected_resolve_workflow, projected_resolve_workflow_run, projected_workflow_id,
};
use crate::session::RuntimeSession;

pub(crate) fn projected_session_state_response(
    session_projection: &SessionStateProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
    prompt_activity: &PromptActivityStore,
    active_turns: &ActiveTurnStore,
    request: &GetSessionStateRequest,
    caller_user_id: &str,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    if let Some(session) = session_projection.get(&request.session_id) {
        if !session.has_member(caller_user_id) {
            return Some(Err(DaemonError::SessionAccessDenied {
                session_id: session.id().to_string(),
                user_id: caller_user_id.to_string(),
            }));
        }
        let session = session.redacted_for_user(caller_user_id);
        return Some(Ok(LocalDaemonResponse::SessionState {
            agent_activity: projected_agent_activity(
                &session,
                provider_run_projection,
                prompt_activity,
                active_turns,
            ),
            session,
        }));
    }
    if session_projection.has_warmed_list() {
        return Some(Err(DaemonError::SessionNotFound {
            session_id: request.session_id.clone(),
        }));
    }
    None
}

pub(crate) fn projected_resolve_session_response(
    session_projection: &SessionStateProjectionStore,
    request: &ResolveSessionRequest,
    caller_user_id: &str,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    if let Some(session) = session_projection
        .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())
    {
        return Some(redacted_session_resolved_response(session, caller_user_id));
    }
    if let Some(result) = session_projection.resolve_session_ref_id_from_warmed_list(
        &request.session_ref,
        request.workspace_id.as_deref(),
    ) {
        let session_id = match result {
            Ok(session_id) => session_id,
            Err(error) => return Some(Err(error)),
        };
        let session = match session_projection.get(&session_id) {
            Some(session) => session,
            None => {
                return Some(Err(DaemonError::SessionNotFound { session_id }));
            }
        };
        return Some(redacted_session_resolved_response(session, caller_user_id));
    }
    None
}

pub(crate) async fn projected_list_sessions_response(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    if let Some(sessions) = session_projection.list() {
        let sessions = sessions
            .into_iter()
            .filter(|session| session.has_member(caller_user_id))
            .map(|session| session.redacted_for_user(caller_user_id))
            .collect();
        return Ok(LocalDaemonResponse::SessionsListed { sessions });
    }
    let sessions: Vec<_> = runtime_state
        .list_session_snapshots()
        .into_iter()
        .filter(|session| session.has_member(caller_user_id))
        .collect();
    session_projection.update_list(sessions.clone());
    Ok(LocalDaemonResponse::SessionsListed {
        sessions: sessions
            .into_iter()
            .map(|session| session.redacted_for_user(caller_user_id))
            .collect(),
    })
}

pub(crate) async fn projected_session_read_response(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    provider_run_projection: &ProviderRunProjectionStore,
    provider_launch_pending: &ProviderLaunchPendingTracker,
    prompt_activity: &PromptActivityStore,
    active_turns: &ActiveTurnStore,
    request: &LocalDaemonRequest,
    caller_user_id: &str,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    if let LocalDaemonRequest::GetSessionState(request) = request {
        if !provider_launch_pending
            .has_unsettled_launch(
                &request.session_id,
                session_projection,
                provider_run_projection,
            )
            .await
        {
            if let Some(response) = projected_session_state_response(
                session_projection,
                provider_run_projection,
                prompt_activity,
                active_turns,
                request,
                caller_user_id,
            ) {
                return Some(response);
            }
        }
    }
    if let LocalDaemonRequest::ResolveSession(request) = request {
        if let Some(response) =
            projected_resolve_session_response(session_projection, request, caller_user_id)
        {
            return Some(response);
        }
    }
    if matches!(request, LocalDaemonRequest::ListSessions(_)) {
        return Some(
            projected_list_sessions_response(runtime_state, session_projection, caller_user_id)
                .await,
        );
    }
    None
}

pub(crate) fn projected_agent_activity(
    session: &RuntimeSession,
    provider_run_projection: &ProviderRunProjectionStore,
    prompt_activity: &PromptActivityStore,
    active_turns: &ActiveTurnStore,
) -> BTreeMap<String, AgentRuntimeActivity> {
    let prompt_activity = prompt_activity.read();
    let active_turns = active_turns.snapshot();
    agent_activity_for_session_projection(
        session,
        |agent_id| provider_run_projection.get_for_agent(session.id(), agent_id),
        &prompt_activity,
        &active_turns,
    )
}

pub(crate) fn projected_session_inspection_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
    caller_user_id: &str,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    match request {
        LocalDaemonRequest::ListAgents(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            let session = session.redacted_for_user(caller_user_id);
            Some(Ok(LocalDaemonResponse::AgentsListed {
                agents: session.agents().to_vec(),
            }))
        }
        LocalDaemonRequest::ListWorkflows(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(Ok(LocalDaemonResponse::WorkflowsListed {
                workflows: session
                    .workflows()
                    .iter()
                    .cloned()
                    .map(|workflow| workflow.redacted_for_user(caller_user_id))
                    .collect(),
            }))
        }
        LocalDaemonRequest::ResolveWorkflow(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(
                projected_resolve_workflow(&session, &request.workflow_ref).map(|workflow| {
                    LocalDaemonResponse::WorkflowResolved {
                        workflow: workflow.redacted_for_user(caller_user_id),
                    }
                }),
            )
        }
        LocalDaemonRequest::ListWorkflowRuns(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(
                projected_workflow_id(&session, request.workflow_ref.as_deref()).map(
                    |workflow_id| {
                        let workflow_runs = session
                            .workflow_runs()
                            .iter()
                            .filter(|workflow_run| {
                                workflow_id
                                    .as_deref()
                                    .is_none_or(|id| workflow_run.workflow_id() == id)
                            })
                            .cloned()
                            .map(|workflow_run| {
                                let workflow = workflow_id.as_deref().and_then(|id| {
                                    session
                                        .workflows()
                                        .iter()
                                        .find(|workflow| workflow.id() == id)
                                });
                                workflow_run.redacted_for_user(workflow, caller_user_id)
                            })
                            .collect();
                        LocalDaemonResponse::WorkflowRunsListed { workflow_runs }
                    },
                ),
            )
        }
        LocalDaemonRequest::GetWorkflowRun(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(
                projected_resolve_workflow_run(&session, &request.workflow_run_ref).map(
                    |workflow_run| {
                        let workflow = session
                            .workflows()
                            .iter()
                            .find(|workflow| workflow.id() == workflow_run.workflow_id());
                        LocalDaemonResponse::WorkflowRun {
                            workflow_run: workflow_run.redacted_for_user(workflow, caller_user_id),
                        }
                    },
                ),
            )
        }
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(
                projected_workflow_id(&session, request.workflow_ref.as_deref()).map(
                    |workflow_id| {
                        let watchdogs = session
                            .workflow_watchdogs()
                            .iter()
                            .filter(|watchdog| {
                                workflow_id
                                    .as_deref()
                                    .is_none_or(|id| watchdog.workflow_id() == id)
                            })
                            .cloned()
                            .collect();
                        LocalDaemonResponse::WorkflowWatchdogsListed { watchdogs }
                    },
                ),
            )
        }
        LocalDaemonRequest::ListWorkflowPromptQueues(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(Ok(LocalDaemonResponse::WorkflowPromptQueuesListed {
                queues: match request.workflow_ref.as_deref() {
                    Some(workflow_ref) => {
                        let workflow_id = match projected_workflow_id(&session, Some(workflow_ref))
                        {
                            Ok(workflow_id) => workflow_id,
                            Err(error) => return Some(Err(error)),
                        };
                        workflow_id
                            .map(|workflow_id| {
                                session.workflow_prompt_queues_for_workflow(&workflow_id)
                            })
                            .unwrap_or_default()
                    }
                    None => session.workflow_prompt_queues().to_vec(),
                },
            }))
        }
        LocalDaemonRequest::ListQueuedWorkflowPrompts(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(Ok(LocalDaemonResponse::QueuedWorkflowPromptsListed {
                queued_prompts: session.workflow_queued_prompts().iter().cloned().collect(),
            }))
        }
        _ => None,
    }
}

pub(crate) async fn execute_list_sessions_request(
    runtime_state: &KernelRuntimeState,
    _request: ListSessionsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::SessionsListed {
        sessions: runtime_state.list_session_snapshots(),
    })
}

pub(crate) async fn execute_resolve_session_request(
    runtime_state: &KernelRuntimeState,
    request: ResolveSessionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::SessionResolved {
        session: runtime_state.resolve_session_snapshot(request)?,
    })
}

pub(crate) async fn execute_get_session_state_request(
    runtime_state: &KernelRuntimeState,
    request: GetSessionStateRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state.session_state_response(request)
}

pub(crate) async fn execute_list_agents_request(
    runtime_state: &KernelRuntimeState,
    request: ListAgentsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(runtime_state.list_agents_response(request))
}

pub(crate) async fn execute_session_read_request(
    runtime_state: &KernelRuntimeState,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::ListSessions(request) => {
            execute_list_sessions_request(runtime_state, request).await
        }
        LocalDaemonRequest::ResolveSession(request) => {
            execute_resolve_session_request(runtime_state, request).await
        }
        LocalDaemonRequest::GetSessionState(request) => {
            execute_get_session_state_request(runtime_state, request).await
        }
        LocalDaemonRequest::ListAgents(request) => {
            execute_list_agents_request(runtime_state, request).await
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "session read request",
            message: "unsupported session read request".to_string(),
        }),
    }
}

pub(crate) fn projected_session_or_absence(
    session_projection: &SessionStateProjectionStore,
    session_id: &str,
) -> Option<Result<RuntimeSession, DaemonError>> {
    if let Some(session) = session_projection.get(session_id) {
        return Some(Ok(session));
    }
    if session_projection.has_warmed_list() {
        return Some(Err(DaemonError::SessionNotFound {
            session_id: session_id.to_string(),
        }));
    }
    None
}

fn redacted_session_resolved_response(
    session: RuntimeSession,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    if !session.has_member(caller_user_id) {
        return Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: caller_user_id.to_string(),
        });
    }
    Ok(LocalDaemonResponse::SessionResolved {
        session: session.redacted_for_user(caller_user_id),
    })
}
