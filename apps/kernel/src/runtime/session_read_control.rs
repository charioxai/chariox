use crate::error::DaemonError;
use crate::local::{
    GetRoomEnvironmentStateRequest, GetSessionStateRequest, ListAgentsRequest, ListSessionsRequest,
    LocalDaemonRequest, LocalDaemonResponse, ResolveSessionRequest,
};
use crate::runtime::projection::{ProviderRunProjectionStore, SessionStateProjectionStore};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::workflow_projection::{projected_resolve_workflow, projected_workflow_id};
use crate::session::{EnvironmentError, RuntimeSession};

fn ensure_projected_workflow_metaagent_scope(
    workflow_metaagent_id: Option<&str>,
    caller_metaagent_id: Option<&str>,
    reference: &str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let Some(caller_metaagent_id) = caller_metaagent_id else {
        return Ok(());
    };
    if workflow_metaagent_id == Some(caller_metaagent_id) {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "workflow `{reference}` is not controlled by metaagent `{caller_metaagent_id}`"
            ),
        })
    }
}

pub(crate) fn projected_session_state_response(
    runtime_state: &KernelRuntimeState,
    session_projection: &SessionStateProjectionStore,
    request: &GetSessionStateRequest,
    caller_user_id: &str,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let session = match runtime_state.session_state_response(request.clone()) {
        Ok(LocalDaemonResponse::SessionState { session, .. }) => session,
        Ok(_) => unreachable!("session state request returned a different response"),
        Err(error) => return Some(Err(error)),
    };
    if !session.has_member(caller_user_id) {
        return Some(Err(DaemonError::SessionAccessDenied {
            session_id: session.id().to_string(),
            user_id: caller_user_id.to_string(),
        }));
    }
    let agent_activity =
        runtime_state.agent_activity_for_session_with_unread(&session, Some(caller_user_id));
    let redacted_session = session.redacted_for_user(caller_user_id);
    let visible_agent_ids = redacted_session
        .agents()
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    Some(Ok(LocalDaemonResponse::SessionState {
        agent_activity: agent_activity
            .into_iter()
            .filter(|(agent_id, _)| visible_agent_ids.contains(agent_id))
            .collect(),
        agent_activity_revision: session_projection.change_sequence(),
        session: redacted_session,
    }))
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
                runtime_state,
                session_projection,
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

pub(crate) fn projected_session_inspection_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
    caller_user_id: &str,
    caller_metaagent_id: Option<&str>,
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
                    .filter(|workflow| {
                        caller_metaagent_id.is_none_or(|metaagent_id| {
                            workflow.controlled_by_metaagent_id() == Some(metaagent_id)
                        })
                    })
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
                projected_resolve_workflow(&session, &request.workflow_ref).and_then(|workflow| {
                    ensure_projected_workflow_metaagent_scope(
                        workflow.controlled_by_metaagent_id(),
                        caller_metaagent_id,
                        &request.workflow_ref,
                        "resolve workflow",
                    )?;
                    Ok(LocalDaemonResponse::WorkflowResolved {
                        workflow: workflow.redacted_for_user(caller_user_id),
                    })
                }),
            )
        }
        // Workflow run history is intentionally not served from the hot session projection.
        // The workflow lane merges active runs with the durable paginated history store.
        LocalDaemonRequest::ListWorkflowRuns(_) | LocalDaemonRequest::GetWorkflowRun(_) => None,
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
        LocalDaemonRequest::ListWorkflowSchedules(request) => {
            let session =
                match projected_session_or_absence(session_projection, &request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
            Some(
                projected_workflow_id(&session, request.workflow_ref.as_deref()).map(
                    |workflow_id| {
                        let schedules = session
                            .workflow_watchdogs()
                            .iter()
                            .filter(|schedule| {
                                workflow_id
                                    .as_deref()
                                    .is_none_or(|id| schedule.workflow_id() == id)
                            })
                            .cloned()
                            .collect();
                        LocalDaemonResponse::WorkflowSchedulesListed { schedules }
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

pub(crate) async fn execute_get_room_environment_state_request(
    runtime_state: &KernelRuntimeState,
    request: GetRoomEnvironmentStateRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime_state
        .room_environment_snapshot(&request.session_id)
        .map(|environment| LocalDaemonResponse::RoomEnvironmentState { environment })
        .map_err(room_environment_read_error)
}

fn room_environment_read_error(error: EnvironmentError) -> DaemonError {
    match error {
        EnvironmentError::RoomNotFound { session_id } => {
            DaemonError::SessionNotFound { session_id }
        }
        EnvironmentError::EnvironmentNotFound { session_id } => DaemonError::RoomEnvironment {
            operation: "environment.state.get",
            code: "environment_not_found",
            message: format!("Room `{session_id}` has no Environment"),
        },
        other => DaemonError::RoomEnvironment {
            operation: "environment.state.get",
            code: "environment_state_unavailable",
            message: format!("{other:?}"),
        },
    }
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
        LocalDaemonRequest::GetRoomEnvironmentState(request) => {
            execute_get_room_environment_state_request(runtime_state, request).await
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
