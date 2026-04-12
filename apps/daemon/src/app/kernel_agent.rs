use super::prompt_lifecycle::{
    KernelPromptAbortDispatch, KernelPromptCancellation, KernelPromptDispatch,
    KernelPromptSubmission,
};
use super::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{PromptAttachment, PromptCancellation, PromptStatus, PromptSubmissionOutcome};
use crate::transport::flow_control;

pub(crate) struct KernelAgentService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.app.submit_prompt(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub(crate) fn submit_prompt_for_kernel(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.app.sessions.get_session(session_id)?;

        let target_agent_id = target_agent_id
            .or_else(|| session_before.focused_agent_id())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            let outcome = self.submit_prompt(
                session_id,
                attachment_id,
                Some(&target_agent_id),
                prompt,
                attachments,
            )?;
            let session = self.app.local_api_session_snapshot(session_id)?;
            return Ok(KernelPromptSubmission {
                outcome,
                session,
                dispatch: None,
            });
        }

        let queued_while_active = session_before
            .active_prompt_for_agent(&target_agent_id)
            .is_some();
        let provider_run_id = if queued_while_active {
            self.app
                .providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
        } else {
            Some(
                self.app
                    .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)?,
            )
        };
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);

        self.app.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let (_session, outcome) = if provider_run_is_starting {
            self.app.sessions.queue_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        } else {
            self.app.sessions.submit_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        };

        let mut dispatch = None;
        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.to_string(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                let provider_run = self
                    .app
                    .prepare_provider_prompt_dispatch(session_id, provider_run_id)?;
                if self
                    .app
                    .providers
                    .run_uses_structured_prompt_io(&provider_run)
                {
                    flow_control::note_prompt_started(self.app, provider_run_id);
                    dispatch = Some(KernelPromptDispatch {
                        session_id: session_id.to_string(),
                        provider_run_id: provider_run_id.to_string(),
                        agent_id: target_agent_id.clone(),
                        prompt: prompt.prompt().to_string(),
                        attachments: prompt.attachments().to_vec(),
                    });
                } else if let Err(error) = self.app.send_provider_input(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt().as_bytes(),
                ) {
                    let _ = self
                        .app
                        .sessions
                        .cancel_active_prompt(session_id, &target_agent_id);
                    flow_control::clear_prompt_activity(self.app, provider_run_id);
                    return Err(error);
                } else {
                    flow_control::note_prompt_started(self.app, provider_run_id);
                }
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.app.record_notice(
                    session_id,
                    provider_run_id.as_deref(),
                    self.app.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        session_before
                            .queued_prompts_for_agent(&target_agent_id)
                            .map(|queue| queue.len())
                            .unwrap_or(0)
                            + 1
                    ),
                );
            }
        }

        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok(KernelPromptSubmission {
            outcome,
            session,
            dispatch,
        })
    }

    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent_id = self
            .app
            .sessions
            .get_session(session_id)?
            .focused_agent_id()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?
            .to_string();
        self.app
            .cancel_active_prompt_internal(session_id, &target_agent_id, Some(attachment_id))
    }

    pub(crate) fn cancel_active_prompt_for_kernel(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent_id = self
            .app
            .sessions
            .get_session(session_id)?
            .focused_agent_id()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?
            .to_string();
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return self
                .app
                .cancel_active_prompt_internal(session_id, &target_agent_id, Some(attachment_id))
                .map(|cancellation| KernelPromptCancellation {
                    cancellation,
                    dispatch: None,
                });
        }

        let active_prompt = self
            .app
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(&target_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(KernelPromptCancellation {
                cancellation: PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                dispatch: None,
            });
        }

        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(session_id, &target_agent_id)
            .map(|run| run.id().to_string())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;
        let dispatch = if self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run)
        {
            Some(KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.clone(),
            })
        } else {
            self.app
                .send_provider_input(session_id, &provider_run_id, attachment_id, b"\x03")?;
            None
        };

        let (_session, prompt) = self
            .app
            .sessions
            .begin_cancelling_active_prompt(session_id, &target_agent_id)?;
        flow_control::note_prompt_settlement_requested(self.app, &provider_run_id);
        self.app.record_notice(
            session_id,
            Some(&provider_run_id),
            self.app.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        );

        Ok(KernelPromptCancellation {
            cancellation: PromptCancellation {
                prompt,
                started_next: None,
            },
            dispatch,
        })
    }
}
