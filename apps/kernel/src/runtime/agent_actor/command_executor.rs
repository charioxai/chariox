//! Agent command execution once a command has entered an agent lane.

use crate::app::KernelPreparedPromptSubmission;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::agent_prompt_service::AgentPromptCommandService;
use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
use crate::session::{PromptCompletion, PromptIdAllocator, PromptQueueItem, PromptStatus};

use super::command_lane::PromptSubmitResponseMode;
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
            AgentCommand::SubmitPrompt {
                request,
                trace_id,
                response_mode,
            } => self.submit_prompt(request, trace_id, response_mode).await,
            AgentCommand::CancelActivePrompt {
                request,
                target_agent_id,
            } => self.cancel_active_prompt(request, target_agent_id).await,
            AgentCommand::SteerQueuedPrompt { request } => self.steer_queued_prompt(request).await,
            AgentCommand::CancelQueuedPrompt { request } => {
                self.cancel_queued_prompt(request).await
            }
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
        response_mode: PromptSubmitResponseMode,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let target_agent_id =
            request
                .target_agent_id
                .clone()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "no target agent".to_string(),
                })?;
        let meta_slash = crate::runtime::state::parse_meta_slash_command(&request.prompt);
        let consumed_meta_slash = meta_slash.is_some();
        let (provider_prompt, hidden_system_context) = if let Some(meta_slash) = meta_slash {
            self.prompt_commands
                .activate_meta_mode_for_prompt(
                    &request.session_id,
                    &target_agent_id,
                    &meta_slash.task_prompt,
                )
                .await?;
            (
                meta_slash.task_prompt,
                self.prompt_commands.meta_mode_entered_hidden_context()?,
            )
        } else {
            (request.prompt.clone(), String::new())
        };
        let prompt = PromptQueueItem::new(
            self.prompt_id_allocator.next_prompt_id(),
            &request.attachment_id,
            &target_agent_id,
            provider_prompt,
            PromptStatus::Queued,
        )
        .with_hidden_system_context(hidden_system_context)
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
                refresh_projection: response_mode == PromptSubmitResponseMode::Full,
            })
            .await?;
        let mut response_session = prepared.session.clone();
        if !consumed_meta_slash {
            if let Some(updated_session) = self.prompt_commands.start_metaagent_task_for_prompt(
                &request.session_id,
                &target_agent_id,
                &request.prompt,
            )? {
                response_session = updated_session;
            }
        }
        if response_mode == PromptSubmitResponseMode::Full {
            self.session_projection.update(response_session.clone());
            self.agent_runtime_projection
                .update_session(&response_session);
        }

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
        let agent_activity = if response_mode == PromptSubmitResponseMode::Full {
            self.prompt_commands
                .agent_activity_for_session(&response_session)
        } else {
            Default::default()
        };

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_dispatch(dispatch);
        }
        if let Some(dispatch) = prepared.remote_dispatch {
            self.prompt_commands.spawn_remote_prompt_dispatch(dispatch);
        }

        Ok(LocalDaemonResponse::PromptSubmitted {
            outcome: prepared.outcome,
            session: response_session,
            agent_activity,
            agent_activity_revision: if response_mode == PromptSubmitResponseMode::Full {
                self.session_projection.change_sequence()
            } else {
                0
            },
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

    async fn steer_queued_prompt(
        &self,
        request: crate::local::SteerQueuedPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = self
            .prompt_commands
            .steer_queued_prompt(
                &request.session_id,
                &request.target_agent_id,
                &request.attachment_id,
                &request.prompt_id,
            )
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        let agent_activity = self
            .prompt_commands
            .agent_activity_for_session(&prepared.session);
        self.prompt_commands
            .spawn_queued_prompt_steer_dispatch(prepared.dispatch);

        Ok(LocalDaemonResponse::QueuedPromptSteered {
            prompt: prepared.prompt,
            session: prepared.session,
            agent_activity,
            agent_activity_revision: self.session_projection.change_sequence(),
        })
    }

    async fn cancel_queued_prompt(
        &self,
        request: crate::local::CancelQueuedPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = self
            .prompt_commands
            .cancel_queued_prompt(
                &request.session_id,
                &request.target_agent_id,
                &request.attachment_id,
                &request.prompt_id,
            )
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        Ok(LocalDaemonResponse::QueuedPromptCancelled {
            prompt: prepared.prompt,
            session: prepared.session,
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
        let mut session = self
            .prompt_commands
            .session_snapshot(&request.session_id)
            .await?;
        self.session_projection.update(session.clone());
        debug_assert!(
            completion_started_next_is_compatible(next_queued_prompt.as_ref(), &completion),
            "agent runtime queue-front preview should match compatibility advancement"
        );
        self.agent_runtime_projection.update_session(&session);

        self.prompt_commands
            .inject_metaagent_turn_completion_event(
                &request.session_id,
                &target_agent_id,
                &completion,
            )?;
        session = self
            .prompt_commands
            .session_snapshot(&request.session_id)
            .await?;
        self.session_projection.update(session.clone());
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
