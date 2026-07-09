use crate::agent::RemoteAgentBinding;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
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

        let prompt_completion = PromptCompletion {
            completed: completion.completed,
            started_next,
        };
        self.inject_controlled_agent_turn_completion_event(
            &completion.session_id,
            &completion.agent_id,
            &prompt_completion,
        )?;
        self.inject_orphaned_metaagent_task_event_after_turn(
            &completion.agent_id,
            &prompt_completion,
        )?;
        Ok(prompt_completion)
    }

    pub(super) fn record_assistant_message_completion(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        self.app.record_assistant_message_completion(
            session_id,
            provider_run_id,
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(super) fn inject_controlled_agent_turn_completion_event(
        &mut self,
        session_id: &str,
        completed_agent_id: &str,
        completion: &PromptCompletion,
    ) -> Result<(), DaemonError> {
        let completed_agent = self.app.agents.get_agent(completed_agent_id)?;
        if completed_agent.is_metaagent() {
            return Ok(());
        }
        let Some(metaagent_id) = completed_agent
            .controlled_by_metaagent_id()
            .map(str::to_string)
        else {
            return Ok(());
        };
        let metaagent = self.app.agents.get_agent(&metaagent_id)?;
        if metaagent.session_id() != session_id || !metaagent.is_metaagent() {
            return Ok(());
        }

        let source_attachment_id = self.ensure_metaagent_task_attachment(session_id, &metaagent)?;
        let prompt_id = self.app.sessions_mut().reserve_prompt_id();
        let prompt_preview = completion
            .completed
            .prompt()
            .chars()
            .take(240)
            .collect::<String>();
        let title = format!(
            "{} completed a turn",
            completed_agent
                .alias()
                .unwrap_or_else(|| completed_agent.agent_ref())
        );
        let summary = format!(
            "Agent {} completed prompt {}. User prompt preview: {}",
            completed_agent.agent_ref(),
            completion.completed.id(),
            if prompt_preview.trim().is_empty() {
                "<empty>"
            } else {
                prompt_preview.trim()
            }
        );
        let record = self.app.metaagent_event_store().record(
            crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session_id.to_string(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: Some(completed_agent.id().to_string()),
                title: title.clone(),
                summary: summary.clone(),
                detail: serde_json::json!({
                    "completed_prompt_id": completion.completed.id(),
                    "source_attachment_id": completion.completed.source_attachment_id(),
                    "completed_agent_id": completed_agent.id(),
                    "completed_agent_ref": completed_agent.agent_ref(),
                    "completed_agent_alias": completed_agent.alias(),
                    "started_next_prompt_id": completion.started_next.as_ref().map(|prompt| prompt.id()),
                }),
                injected_prompt_id: Some(prompt_id.clone()),
            },
        );
        self.persist_metaagent_event_record("metaagent.event.recorded", &record);
        let assembly = crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
            crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                event_id: record.event_id.clone(),
                event_kind: record.kind.clone(),
                source: completed_agent.agent_ref().to_string(),
                title,
                body: summary,
            },
        );
        let prompt = PromptQueueItem::new(
            prompt_id,
            &source_attachment_id,
            metaagent.id(),
            assembly.visible_user_prompt,
            crate::session::PromptStatus::Queued,
        );
        let submitted = match self.submit_prepared_prompt_for_kernel(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            },
        ) {
            Ok(submitted) => submitted,
            Err(error) => {
                self.update_metaagent_event_prompt_delivery(
                    &record.event_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                    Some(error.to_string()),
                );
                return Err(error);
            }
        };
        let delivery_status = match &submitted.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Submitted
            }
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
            }
        };
        self.update_metaagent_event_prompt_delivery(&record.event_id, delivery_status, None);
        if let Err(error) = self
            .finish_compat_prompt_dispatch(submitted.dispatch)
            .and_then(|_| self.finish_compat_remote_prompt_dispatch(submitted.remote_dispatch))
        {
            self.update_metaagent_event_prompt_delivery(
                &record.event_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                Some(error.to_string()),
            );
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn inject_orphaned_metaagent_task_event_after_turn(
        &mut self,
        metaagent_id: &str,
        completion: &PromptCompletion,
    ) -> Result<(), DaemonError> {
        let metaagent = self.app.agents.get_agent(metaagent_id)?;
        if !metaagent.is_metaagent() {
            return Ok(());
        }
        if completion.started_next.is_some() {
            return Ok(());
        }
        if self
            .app
            .providers
            .get_run_for_agent(metaagent.session_id(), metaagent.id())
            .is_some_and(|run| self.app.active_turns.snapshot().contains_key(run.id()))
        {
            return Ok(());
        }
        let session_id = metaagent.session_id().to_string();
        let mut session = self.app.sessions.get_session(&session_id)?;
        let Some(task) = session.metaagent_task(metaagent.id()).cloned() else {
            return Ok(());
        };
        if task.status() != crate::session::MetaagentTaskStatus::Active {
            return Ok(());
        }
        if self
            .app
            .prompt_state_owner
            .active_prompt_for_agent(&session, metaagent.id())
            .is_some()
        {
            return Ok(());
        }
        session.set_agents(self.app.agents.get_session_agents(&session_id));
        if self.metaagent_has_active_owned_regular_agent_work(&session, &metaagent) {
            return Ok(());
        }
        if self
            .app
            .metaagent_event_store()
            .has_orphaned_task_event_for_revision(metaagent.id(), task.task_id(), task.revision())
        {
            return Ok(());
        }

        let source_attachment_id =
            self.ensure_metaagent_task_attachment(&session_id, &metaagent)?;
        let prompt_id = self.app.sessions_mut().reserve_prompt_id();
        let title = "Metaagent task needs a final decision".to_string();
        let summary = format!(
            "Your last turn ended while task `{}` is still active and no same-owner regular agents have active or queued work. Decide the task state now: if it is done, call `arroba.meta.complete_task`; if it cannot be completed after exhausting options, call `arroba.meta.mark_blocked`; otherwise update your plan and continue/delegate the remaining work. Do not answer only in natural language; update the kernel-managed task state through the metaagent runtime tools.",
            task.task_id()
        );
        let record = self.app.metaagent_event_store().record(
            crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session_id.clone(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "metaagent.task.orphaned".to_string(),
                source_agent_id: None,
                title: title.clone(),
                summary: summary.clone(),
                detail: serde_json::json!({
                    "metaagent_id": metaagent.id(),
                    "task_id": task.task_id(),
                    "task_revision": task.revision(),
                    "completed_prompt_id": completion.completed.id(),
                    "completed_prompt_source_attachment_id": completion.completed.source_attachment_id(),
                }),
                injected_prompt_id: Some(prompt_id.clone()),
            },
        );
        self.persist_metaagent_event_record("metaagent.event.recorded", &record);
        let assembly = crate::scheduler::prompt_injection::render_metaagent_event_prompt_assembly(
            crate::scheduler::prompt_injection::MetaagentEventPromptContext {
                event_id: record.event_id.clone(),
                event_kind: record.kind.clone(),
                source: "runtime".to_string(),
                title,
                body: summary,
            },
        );
        let prompt = PromptQueueItem::new(
            prompt_id,
            &source_attachment_id,
            metaagent.id(),
            assembly.visible_user_prompt,
            crate::session::PromptStatus::Queued,
        );
        let submitted = match self.submit_prepared_prompt_for_kernel(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            },
        ) {
            Ok(submitted) => submitted,
            Err(error) => {
                self.update_metaagent_event_prompt_delivery(
                    &record.event_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                    Some(error.to_string()),
                );
                return Err(error);
            }
        };
        let delivery_status = match &submitted.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Submitted
            }
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
            }
        };
        self.update_metaagent_event_prompt_delivery(&record.event_id, delivery_status, None);
        if let Err(error) = self
            .finish_compat_prompt_dispatch(submitted.dispatch)
            .and_then(|_| self.finish_compat_remote_prompt_dispatch(submitted.remote_dispatch))
        {
            self.update_metaagent_event_prompt_delivery(
                &record.event_id,
                crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                Some(error.to_string()),
            );
            return Err(error);
        }
        Ok(())
    }

    fn metaagent_has_active_owned_regular_agent_work(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
    ) -> bool {
        let prompt_activity = self.app.prompt_activity.read();
        let active_turns = self.app.active_turns.snapshot();
        let activity = crate::runtime::projection::agent_activity_for_session_projection(
            session,
            |agent_id| self.app.providers.get_run_for_agent(session.id(), agent_id),
            &prompt_activity,
            &active_turns,
            None,
            |_| None,
        );
        self.app
            .agents
            .get_session_agents(session.id())
            .into_iter()
            .filter(|agent| !agent.is_metaagent())
            .filter(|agent| agent.owner_user_id() == metaagent.owner_user_id())
            .any(|agent| {
                activity
                    .get(agent.id())
                    .is_some_and(|agent_activity| agent_activity.busy)
                    || self
                        .app
                        .prompt_state_owner
                        .active_prompt_for_agent(session, agent.id())
                        .is_some()
                    || self
                        .app
                        .prompt_state_owner
                        .queued_prompt_count_for_agent(session, agent.id())
                        > 0
            })
    }

    fn ensure_metaagent_task_attachment(
        &mut self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let client_id = format!("metaagent:{}:task", metaagent.id());
        if let Some(attachment) = self
            .app
            .attachments
            .list_client_attachments(&client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let attachment = self.app.attach(AttachRequest::for_user(
            session_id,
            client_id,
            ClientCapabilityLevel::AutomationOnly,
            metaagent.owner_user_id(),
        ))?;
        Ok(attachment.id().to_string())
    }

    fn update_metaagent_event_prompt_delivery(
        &self,
        event_id: &str,
        status: crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus,
        error: Option<String>,
    ) {
        if let Some(record) = self
            .app
            .metaagent_event_store()
            .update_prompt_delivery_status(event_id, status, error)
        {
            self.persist_metaagent_event_record("metaagent.event.delivery_updated", &record);
        }
    }

    fn persist_metaagent_event_record(
        &self,
        kind: &'static str,
        record: &crate::runtime::metaagent_event::MetaagentEventRecord,
    ) {
        if let Err(error) = self.app.durable_state_store().append_event(
            kind,
            Some(record.event_id.clone()),
            serde_json::json!({
                "record": record,
            }),
        ) {
            crate::logging::warn_with_fields(
                "metaagent.event",
                "failed to persist metaagent event record",
                serde_json::json!({
                    "kind": kind,
                    "event_id": &record.event_id,
                    "metaagent_id": &record.metaagent_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}
