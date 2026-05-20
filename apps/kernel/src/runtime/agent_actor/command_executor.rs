//! Agent command execution once a command has entered an agent lane.

use crate::app::KernelPreparedPromptSubmission;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::agent_prompt_service::AgentPromptCommandService;
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
use crate::session::{PromptCompletion, PromptIdAllocator, PromptQueueItem, PromptStatus};

use super::prompt_attachment_materialization::materialize_inline_prompt_attachments;
use super::AgentCommand;

#[derive(Clone)]
pub(super) struct AgentRuntimeCommandExecutor {
    prompt_commands: AgentPromptCommandService,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_id_allocator: PromptIdAllocator,
}

impl AgentRuntimeCommandExecutor {
    pub(super) fn new(
        prompt_commands: AgentPromptCommandService,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self {
            prompt_commands,
            session_projection,
            agent_runtime_projection,
            prompt_id_allocator,
        }
    }

    pub(super) async fn execute(
        &self,
        command: AgentCommand,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match command {
            AgentCommand::SubmitPrompt { request, trace_id } => {
                self.submit_prompt(request, trace_id).await
            }
            AgentCommand::CancelActivePrompt {
                request,
                target_agent_id,
            } => self.cancel_active_prompt(request, target_agent_id).await,
            AgentCommand::CompletePrompt {
                request,
                target_agent_id,
                next_queued_prompt,
            } => {
                self.complete_prompt(request, target_agent_id, next_queued_prompt)
                    .await
            }
        }
    }

    async fn submit_prompt(
        &self,
        request: crate::local::SubmitPromptRequest,
        trace_id: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let target_agent_id =
            request
                .target_agent_id
                .clone()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "no target agent".to_string(),
                })?;
        let prompt = PromptQueueItem::new(
            self.prompt_id_allocator.next_prompt_id(),
            &request.attachment_id,
            &target_agent_id,
            &request.prompt,
            PromptStatus::Queued,
        )
        .with_attachments(materialize_inline_prompt_attachments(
            &request.session_id,
            &target_agent_id,
            request.attachments,
        )?);
        let prepared = self
            .prompt_commands
            .submit_prepared_prompt(KernelPreparedPromptSubmission {
                session_id: request.session_id.clone(),
                prompt,
                force_queue: false,
            })
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        if let (crate::session::PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&prepared.outcome, prepared.dispatch.as_ref())
        {
            self.prompt_commands.start_active_turn_with_trace_id(
                &dispatch.session_id,
                &dispatch.agent_id,
                prompt.id(),
                &dispatch.provider_run_id,
                &trace_id,
            );
        }
        let agent_activity = self
            .prompt_commands
            .agent_activity_for_session(&prepared.session);

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_dispatch(dispatch);
        }
        if let Some(dispatch) = prepared.remote_dispatch {
            self.prompt_commands.spawn_remote_prompt_dispatch(dispatch);
        }

        Ok(LocalDaemonResponse::PromptSubmitted {
            outcome: prepared.outcome,
            session: prepared.session,
            agent_activity,
        })
    }

    async fn cancel_active_prompt(
        &self,
        request: crate::local::CancelActivePromptRequest,
        target_agent_id: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = self
            .prompt_commands
            .cancel_agent_prompt(
                &request.session_id,
                &target_agent_id,
                &request.attachment_id,
            )
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_abort(dispatch);
        }

        Ok(LocalDaemonResponse::PromptCancelled {
            cancellation: prepared.cancellation,
        })
    }

    async fn complete_prompt(
        &self,
        request: crate::local::CompletePromptRequest,
        target_agent_id: String,
        next_queued_prompt: Option<PromptQueueItem>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let completion = self
            .prompt_commands
            .complete_agent_prompt(
                &request.session_id,
                &target_agent_id,
                next_queued_prompt.clone(),
            )
            .await?;
        let session = self
            .prompt_commands
            .session_snapshot(&request.session_id)
            .await?;
        self.session_projection.update(session.clone());
        debug_assert!(
            completion_started_next_is_compatible(next_queued_prompt.as_ref(), &completion),
            "agent runtime queue-front preview should match compatibility advancement"
        );
        self.agent_runtime_projection.update_session(&session);

        Ok(LocalDaemonResponse::PromptCompleted { completion })
    }
}

fn completion_started_next_is_compatible(
    next_queued_prompt: Option<&PromptQueueItem>,
    completion: &PromptCompletion,
) -> bool {
    match (next_queued_prompt, completion.started_next.as_ref()) {
        (Some(expected), Some(started)) => expected.id() == started.id(),
        _ => true,
    }
}
