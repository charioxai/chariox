use crate::agent::RemoteAgentBinding;
use crate::error::DaemonError;
use crate::session::{PromptCompletion, PromptQueueItem};
use crate::transport::flow_control;

use super::super::KernelAgentService;

pub(super) enum KernelPromptCompletionAdmission {
    Remote {
        session_id: String,
        agent_id: String,
        remote_execution: RemoteAgentBinding,
        next_queued_prompt: Option<PromptQueueItem>,
    },
    Local {
        session_id: String,
        agent_id: String,
        provider_run_id: Option<String>,
        next_queued_prompt: Option<PromptQueueItem>,
    },
}

pub(super) struct KernelPromptOwnerCompletion {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) completed: PromptQueueItem,
    pub(super) provider_run_id: Option<String>,
    pub(super) remote_execution: Option<RemoteAgentBinding>,
    pub(super) remote_provider_run_id: Option<String>,
    pub(super) next_queued_prompt: Option<PromptQueueItem>,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCompletion, DaemonError> {
        self.complete_active_prompt_for_kernel(session_id, agent_id, provider_run_id, None)
    }

    pub(crate) fn complete_active_prompt_for_kernel(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: Option<&PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        let admission = self.prepare_prompt_completion_admission(
            session_id,
            agent_id,
            provider_run_id,
            next_queued_prompt,
        )?;
        let completion = match admission {
            KernelPromptCompletionAdmission::Remote { .. } => {
                let completed = self.complete_remote_prompt_from_admission(admission)?;
                self.finish_remote_prompt_completion(completed)?
            }
            KernelPromptCompletionAdmission::Local { .. } => {
                let completed = self.complete_local_prompt_from_admission(admission)?;
                self.finish_local_prompt_completion(completed)?
            }
        };
        Ok(completion)
    }

    fn prepare_prompt_completion_admission(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: Option<&PromptQueueItem>,
    ) -> Result<KernelPromptCompletionAdmission, DaemonError> {
        let target_agent = self.app.agents.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let next_queued_prompt = next_queued_prompt.cloned();
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            return Ok(KernelPromptCompletionAdmission::Remote {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                remote_execution,
                next_queued_prompt,
            });
        }
        Ok(KernelPromptCompletionAdmission::Local {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            next_queued_prompt,
        })
    }

    fn complete_local_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Local {
            session_id,
            agent_id,
            provider_run_id,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected local prompt completion admission".to_string(),
            });
        };

        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id,
            remote_execution: None,
            remote_provider_run_id: None,
            next_queued_prompt,
        })
    }

    fn finish_local_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        if !flow_control::prompt_completion_recorded(
            self.app,
            completion
                .provider_run_id
                .as_deref()
                .unwrap_or(&completion.agent_id),
        ) {
            let recipient_attachment_ids = self
                .app
                .attachments
                .list_session_attachment_ids(&completion.session_id);
            let completion_provider_run_id = completion
                .provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            self.record_assistant_message_completion(
                &completion.session_id,
                completion_provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completion.completed.id()),
                crate::session::unix_epoch_ms(),
            );
            flow_control::mark_prompt_completion_recorded(self.app, completion_provider_run_id);
        }
        crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
            self.app,
            &completion.session_id,
            &completion.completed,
            completion.provider_run_id.as_deref(),
        )?;
        let completion_provider_run_id = completion.provider_run_id.clone().or_else(|| {
            self.app
                .providers
                .get_run_for_agent(&completion.session_id, &completion.agent_id)
                .map(|run| run.id().to_string())
        });
        if let Some(provider_run_id) = completion_provider_run_id.as_deref() {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
            flow_control::clear_active_turn(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            self.advance_next_queued_prompt(
                &completion.session_id,
                &completion.agent_id,
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&completion.session_id)?;

        Ok(PromptCompletion {
            completed: completion.completed,
            started_next,
        })
    }

    pub(super) fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .app
            .providers
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.app.terminal.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }
}
