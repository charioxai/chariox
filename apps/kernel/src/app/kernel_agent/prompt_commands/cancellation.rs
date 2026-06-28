use crate::error::DaemonError;
use crate::session::{PromptCancellation, PromptStatus};
use crate::transport::flow_control;

use super::super::KernelAgentService;

impl<'a> KernelAgentService<'a> {
    pub(crate) fn cancel_active_after_prompt_start_failure(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) {
        let _ = self
            .app
            .prompt_owner_cancel_active_prompt_only(session_id, agent_id);
        flow_control::clear_prompt_activity(self.app, provider_run_id);
    }

    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app)
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent_id = self
            .app
            .prompt_owner_active_prompt_agent_id(session_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.cancel_active_prompt_internal(session_id, &target_agent_id, Some(attachment_id))
    }

    pub(crate) fn cancel_active_prompt_internal(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        let target_agent = self.app.agents.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(PromptCancellation {
                prompt: active_prompt,
                started_next: None,
            });
        }
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            return self.cancel_remote_active_prompt(
                session_id,
                agent_id,
                attachment_id,
                &active_prompt,
                remote_execution,
            );
        }
        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;

        let uses_structured_prompt_io = self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run);
        if !uses_structured_prompt_io {
            crate::app::terminal_input::ProviderTerminalInput::new(self.app).send_provider_input(
                session_id,
                &provider_run_id,
                attachment_id.unwrap_or(active_prompt.source_attachment_id()),
                b"\x03",
            )?;
        }

        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
        flow_control::note_prompt_settlement_requested(self.app, &provider_run_id);
        if uses_structured_prompt_io {
            self.app
                .providers
                .enqueue_structured_prompt_abort(session_id.to_string(), provider_run_id.clone())?;
        }
        let recipients = match attachment_id {
            Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
            None => self.app.attachments.list_session_attachment_ids(session_id),
        };
        let message = match attachment_id {
            Some(attachment_id) => format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
            None => format!(
                "Arroba requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        };
        self.app
            .record_notice(session_id, Some(&provider_run_id), recipients, message);
        if uses_structured_prompt_io && provider_run.adapter_key() == "claude" {
            return self.finalize_active_prompt_cancellation(
                session_id,
                agent_id,
                Some(&provider_run_id),
            );
        }
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;

        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        let prompt = self
            .app
            .prompt_owner_finalize_active_prompt_cancellation(session_id, agent_id)?;
        crate::app::workflow_runtime::cancel_workflow_prompt_from_runtime(
            self.app, session_id, &prompt,
        )?;
        let cancellation_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.app
                .providers
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        if let Some(provider_run_id) = cancellation_provider_run_id.as_deref() {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_none()
        {
            self.advance_next_queued_prompt(session_id, agent_id, None)?
        } else {
            None
        };
        if started_next.is_none() {
            self.app.sync_focused_provider_run_if_idle(session_id)?;
        }
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;

        Ok(PromptCancellation {
            prompt,
            started_next,
        })
    }
}
