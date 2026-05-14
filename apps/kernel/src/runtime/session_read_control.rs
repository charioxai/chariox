use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::{ActiveTurnStore, DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::local::{
    GetSessionStateRequest, ListAgentsRequest, ListSessionsRequest, LocalDaemonResponse,
    ResolveSessionRequest,
};
use crate::runtime::projection::{
    agent_activity_for_session_projection, AgentRuntimeActivity, ProviderRunProjectionStore,
    SessionStateProjectionStore,
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
    app: &Arc<Mutex<DaemonApp>>,
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
    let sessions: Vec<_> = {
        let app = app.lock().await;
        app.sessions()
            .list_sessions()
            .into_iter()
            .filter(|session| session.has_member(caller_user_id))
            .collect()
    };
    session_projection.update_list(sessions.clone());
    Ok(LocalDaemonResponse::SessionsListed {
        sessions: sessions
            .into_iter()
            .map(|session| session.redacted_for_user(caller_user_id))
            .collect(),
    })
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

pub(crate) async fn execute_list_sessions_request(
    app: &Arc<Mutex<DaemonApp>>,
    _request: ListSessionsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    crate::app::KernelSessionReadService::new(&app).list_sessions_response()
}

pub(crate) async fn execute_resolve_session_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ResolveSessionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    crate::app::KernelSessionReadService::new(&app).resolve_session_response(request)
}

pub(crate) async fn execute_get_session_state_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: GetSessionStateRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    crate::app::KernelSessionReadService::new(&app).get_session_state_response(request)
}

pub(crate) async fn execute_list_agents_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListAgentsRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let app = app.lock().await;
    crate::app::KernelSessionReadService::new(&app).list_agents_response(request)
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
