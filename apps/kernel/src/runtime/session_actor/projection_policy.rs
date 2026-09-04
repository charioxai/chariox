use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::projection::SessionStateProjectionStore;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::session::RuntimeSession;
use crate::terminal::TerminalStreamStore;

pub(super) enum SessionProjectionAction {
    Update(RuntimeSession),
    Remove { session_id: String },
}

pub(super) async fn update_focus_projection_after_session_command(
    focus_projection: &FocusedAgentProjection,
    session_id: &str,
    result: &Result<LocalDaemonResponse, DaemonError>,
    focused_agent_id: Option<&str>,
) {
    match result {
        Ok(LocalDaemonResponse::SessionCreated { session, .. }) => {
            focus_projection
                .update(
                    session.id(),
                    focused_agent_id.or_else(|| session.focused_agent_id()),
                )
                .await;
        }
        Ok(LocalDaemonResponse::SessionEnded { .. })
        | Ok(LocalDaemonResponse::SessionDeleted { .. }) => {
            focus_projection.remove(session_id).await;
        }
        Ok(_) => {
            focus_projection.update(session_id, focused_agent_id).await;
        }
        Err(_) => {}
    }
}

pub(super) fn session_response_projection_action(
    response: &LocalDaemonResponse,
) -> Option<SessionProjectionAction> {
    match response {
        LocalDaemonResponse::SessionCreated { session, .. }
        | LocalDaemonResponse::PromptSubmitted { session, .. }
        | LocalDaemonResponse::QueuedPromptSteered { session, .. }
        | LocalDaemonResponse::QueuedPromptCancelled { session, .. }
        | LocalDaemonResponse::QueuedPromptUpdated { session, .. }
        | LocalDaemonResponse::AgentAliased { session, .. }
        | LocalDaemonResponse::AgentConfigUpdated { session, .. }
        | LocalDaemonResponse::AgentProfileUpdated { session, .. }
        | LocalDaemonResponse::AgentForked { session, .. }
        | LocalDaemonResponse::SessionConfigUpdated { session, .. }
        | LocalDaemonResponse::AgentPromptScheduleCreated { session, .. }
        | LocalDaemonResponse::AgentPromptScheduleCancelled { session, .. }
        | LocalDaemonResponse::SessionEnded { session }
        | LocalDaemonResponse::SessionAliased { session } => {
            Some(SessionProjectionAction::Update(session.clone()))
        }
        LocalDaemonResponse::SessionDeleted { session } => Some(SessionProjectionAction::Remove {
            session_id: session.id().to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

    #[test]
    fn prompt_responses_with_session_snapshots_project_updated_session() {
        let session = RuntimeSession::new(
            "session-prompt",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "hello",
            PromptStatus::Running,
        );

        let submitted = LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started {
                prompt: prompt.clone(),
            },
            session: session.clone(),
            agent_activity: Default::default(),
            agent_activity_revision: 0,
        };
        let steered = LocalDaemonResponse::QueuedPromptSteered {
            prompt: prompt.clone(),
            session: session.clone(),
            agent_activity: Default::default(),
            agent_activity_revision: 0,
        };
        let cancelled = LocalDaemonResponse::QueuedPromptCancelled {
            prompt: prompt.clone(),
            session: session.clone(),
            agent_activity: Default::default(),
            agent_activity_revision: 0,
        };
        let updated = LocalDaemonResponse::QueuedPromptUpdated {
            prompt,
            session: session.clone(),
            agent_activity: Default::default(),
            agent_activity_revision: 0,
        };

        for response in [submitted, steered, cancelled, updated] {
            let Some(SessionProjectionAction::Update(projected)) =
                session_response_projection_action(&response)
            else {
                panic!("prompt response should project updated session");
            };
            assert_eq!(projected, session);
        }
    }
}

pub(super) fn projected_runtime_notices_response(
    session_projection: &SessionStateProjectionStore,
    terminal_stream: &TerminalStreamStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::PollRuntimeNotices(request) = request else {
        return None;
    };
    if session_projection
        .get(&request.session_id)
        .is_some_and(|session| session.has_attachment(&request.attachment_id))
    {
        return Some(Ok(LocalDaemonResponse::RuntimeNotices {
            notices: terminal_stream
                .drain_notice_records(&request.session_id, &request.attachment_id),
        }));
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    let result = match session_projection.session_id_for_attachment(&request.attachment_id) {
        Some(_) => Err(DaemonError::AttachmentNotInSession {
            session_id: request.session_id.clone(),
            attachment_id: request.attachment_id.clone(),
        }),
        None => Err(DaemonError::AttachmentNotFound {
            attachment_id: request.attachment_id.clone(),
        }),
    };
    Some(result)
}

pub(super) fn projected_resize_terminal_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::ResizeTerminal(request) = request else {
        return None;
    };
    if let Some(session) = session_projection.get(&request.session_id) {
        if request.provider_run_id.is_none() && session.active_provider_run_id().is_none() {
            return Some(Err(DaemonError::NoActiveProviderRun {
                session_id: request.session_id.clone(),
            }));
        }
        return None;
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    Some(Err(DaemonError::SessionNotFound {
        session_id: request.session_id.clone(),
    }))
}

pub(super) fn projected_terminal_input_absence_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::SendTerminalInput(request) = request else {
        return None;
    };
    if let Some(session) = session_projection.get(&request.session_id) {
        if !session.has_attachment(&request.attachment_id) {
            return match session_projection.session_id_for_attachment(&request.attachment_id) {
                Some(_) => Some(Err(DaemonError::AttachmentNotInSession {
                    session_id: request.session_id.clone(),
                    attachment_id: request.attachment_id.clone(),
                })),
                None => Some(Err(DaemonError::AttachmentNotFound {
                    attachment_id: request.attachment_id.clone(),
                })),
            };
        }
        if request.provider_run_id.is_none() && session.active_provider_run_id().is_none() {
            return Some(Err(DaemonError::NoActiveProviderRun {
                session_id: request.session_id.clone(),
            }));
        }
        return None;
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    Some(Err(DaemonError::SessionNotFound {
        session_id: request.session_id.clone(),
    }))
}

pub(super) fn projected_config_update_absence_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::UpdateSessionConfig(request) = request else {
        return None;
    };
    if session_projection
        .get(&request.session_id)
        .is_some_and(|session| session.has_attachment(&request.attachment_id))
    {
        return None;
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    let result = match session_projection.session_id_for_attachment(&request.attachment_id) {
        Some(_) => Err(DaemonError::AttachmentNotInSession {
            session_id: request.session_id.clone(),
            attachment_id: request.attachment_id.clone(),
        }),
        None => Err(DaemonError::AttachmentNotFound {
            attachment_id: request.attachment_id.clone(),
        }),
    };
    Some(result)
}

pub(super) fn projected_session_absence_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let session_id = match request {
        LocalDaemonRequest::AttachToSession(request) => &request.session_id,
        LocalDaemonRequest::FocusAgent(request) => &request.session_id,
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => &request.session_id,
        LocalDaemonRequest::CycleAgentFocus(request) => &request.session_id,
        LocalDaemonRequest::AliasSession(request) => &request.session_id,
        LocalDaemonRequest::AliasAgent(request) => &request.session_id,
        LocalDaemonRequest::SpawnAgents(request) => &request.session_id,
        LocalDaemonRequest::UndoTurn(request) => &request.session_id,
        LocalDaemonRequest::ForkAgent(request) => &request.session_id,
        LocalDaemonRequest::UpdateAgentProfile(request) => &request.session_id,
        LocalDaemonRequest::CreateAgentPromptSchedule(request) => &request.session_id,
        LocalDaemonRequest::CancelAgentPromptSchedule(request) => &request.session_id,
        LocalDaemonRequest::StartRoomEnvironment(request) => &request.session_id,
        LocalDaemonRequest::BindRoomEnvironmentSlice(request) => &request.session_id,
        LocalDaemonRequest::StopRoomEnvironment(request) => &request.session_id,
        LocalDaemonRequest::RetryRoomEnvironment(request) => &request.session_id,
        LocalDaemonRequest::UpdateRoomEnvironmentViewport(request) => &request.session_id,
        LocalDaemonRequest::UpdateRoomEnvironmentPointer(request) => &request.session_id,
        LocalDaemonRequest::RequestRoomEnvironmentInputTakeover(request) => &request.session_id,
        LocalDaemonRequest::ReleaseRoomEnvironmentInput(request) => &request.session_id,
        LocalDaemonRequest::SubmitRoomEnvironmentAction(request) => &request.session_id,
        LocalDaemonRequest::SubmitRoomEnvironmentBrowserAction(request) => &request.session_id,
        LocalDaemonRequest::ReadRoomEnvironmentClipboard(request) => &request.session_id,
        LocalDaemonRequest::CancelRoomEnvironmentAction(request) => &request.session_id,
        LocalDaemonRequest::EndSession(request) => &request.session_id,
        _ => return None,
    };
    let Some(session) = session_projection.get(session_id) else {
        if session_projection.has_warmed_list() {
            return Some(Err(DaemonError::SessionNotFound {
                session_id: session_id.clone(),
            }));
        }
        return None;
    };
    let agent_presence_check = match request {
        LocalDaemonRequest::FocusAgent(request) => {
            Some((request.session_id.as_str(), request.agent_id.as_str()))
        }
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(request) => {
            Some((request.session_id.as_str(), request.agent_id.as_str()))
        }
        _ => None,
    };
    if let Some((session_id, agent_id)) = agent_presence_check {
        if !session.agents().iter().any(|agent| agent.id() == agent_id) {
            return Some(Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            }));
        }
    }
    None
}

pub(super) fn session_id_for_projection_refresh(
    result: &Result<LocalDaemonResponse, DaemonError>,
) -> Option<String> {
    match result {
        Ok(LocalDaemonResponse::SessionAttached { attachment })
        | Ok(LocalDaemonResponse::SessionDetached { attachment }) => {
            Some(attachment.session_id().to_string())
        }
        Ok(LocalDaemonResponse::SessionCreated { session, .. }) => Some(session.id().to_string()),
        Ok(LocalDaemonResponse::AgentOutputSeenAcknowledged { session_id, .. }) => {
            Some(session_id.clone())
        }
        Ok(LocalDaemonResponse::AgentFocused { agent }) => Some(agent.session_id().to_string()),
        Ok(LocalDaemonResponse::AgentSpawned { agent })
        | Ok(LocalDaemonResponse::AgentAliased { agent, .. })
        | Ok(LocalDaemonResponse::AgentConfigUpdated { agent, .. })
        | Ok(LocalDaemonResponse::AgentProfileUpdated { agent, .. })
        | Ok(LocalDaemonResponse::AgentDestroyed { agent }) => Some(agent.session_id().to_string()),
        Ok(LocalDaemonResponse::AgentsSpawned { agents }) => {
            agents.first().map(|agent| agent.session_id().to_string())
        }
        Ok(LocalDaemonResponse::TurnUndone { result }) => Some(result.session_id.clone()),
        Ok(LocalDaemonResponse::AgentForked { session, .. }) => Some(session.id().to_string()),
        Ok(LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) }) => {
            Some(agent.session_id().to_string())
        }
        Ok(LocalDaemonResponse::AgentFocusCycled { agent: None }) => None,
        Ok(LocalDaemonResponse::TerminalResized { session_id, .. }) => Some(session_id.clone()),
        Ok(LocalDaemonResponse::SessionConfigUpdated { session, .. }) => {
            Some(session.id().to_string())
        }
        Ok(LocalDaemonResponse::SessionAliased { session }) => Some(session.id().to_string()),
        _ => None,
    }
}
