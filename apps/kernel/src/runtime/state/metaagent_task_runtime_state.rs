use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::session::MetaagentTaskStatus;

use super::KernelRuntimeState;

impl KernelRuntimeState {
    pub(crate) fn start_metaagent_task_for_prompt(
        &self,
        session_id: &str,
        metaagent_id: &str,
        prompt: &str,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(metaagent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: format!("agent `{metaagent_id}` is not in this session"),
            });
        }
        if !agent.is_metaagent() {
            return Ok(None);
        }
        let Some(session) = self
            .owned
            .session_store
            .write()
            .start_metaagent_task_if_needed(session_id, metaagent_id, prompt)?
        else {
            return Ok(None);
        };
        Ok(Some(self.project_metaagent_task_session(session)))
    }

    pub(crate) fn inject_orphaned_metaagent_task_event_after_turn(
        &self,
        session_id: &str,
        metaagent_id: &str,
        completion: &crate::session::PromptCompletion,
    ) -> Result<(), DaemonError> {
        let metaagent = self.owned.agent_store.get_agent(metaagent_id)?;
        if metaagent.session_id() != session_id || !metaagent.is_metaagent() {
            return Ok(());
        }
        if completion.started_next.is_some() {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(task) = session.metaagent_task(metaagent.id()) else {
            return Ok(());
        };
        if task.status() != MetaagentTaskStatus::Active {
            return Ok(());
        }
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, metaagent.id())
            .is_some()
        {
            return Ok(());
        }
        if self.metaagent_has_active_owned_regular_agent_work(&session, &metaagent) {
            return Ok(());
        }

        let source_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(&format!(
                "metaagent-task-orphaned-{}-{}",
                metaagent.id(),
                task.revision()
            ));
        let title = "Metaagent task needs a final decision".to_string();
        let summary = format!(
            "Your last turn ended while task `{}` is still active and no same-owner regular agents have active or queued work. Decide the task state now: if it is done, call `arroba.meta.complete_task`; if it cannot be completed after exhausting options, call `arroba.meta.mark_blocked`; otherwise update your plan and continue/delegate the remaining work. Do not answer only in natural language; update the kernel-managed task state through the metaagent runtime tools.",
            task.task_id()
        );
        let dispatches = self.owned.metaagent_event_prompt_for_metaagent(
            session_id,
            &metaagent,
            "metaagent.task.orphaned",
            None,
            &source_attachment_id,
            title,
            summary,
            serde_json::json!({
                "metaagent_id": metaagent.id(),
                "task_id": task.task_id(),
                "task_revision": task.revision(),
                "completed_prompt_id": completion.completed.id(),
                "completed_prompt_source_attachment_id": completion.completed.source_attachment_id(),
            }),
            "runtime".to_string(),
        );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(())
    }

    fn metaagent_has_active_owned_regular_agent_work(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
    ) -> bool {
        let activity = self.agent_activity_for_session(session);
        self.owned
            .agent_store
            .get_session_agents(session.id())
            .into_iter()
            .filter(|agent| !agent.is_metaagent())
            .filter(|agent| agent.owner_user_id() == metaagent.owner_user_id())
            .any(|agent| {
                activity
                    .get(agent.id())
                    .is_some_and(|agent_activity| agent_activity.busy)
                    || self
                        .owned
                        .prompt_state_owner
                        .active_prompt_for_agent(session, agent.id())
                        .is_some()
                    || self
                        .owned
                        .prompt_state_owner
                        .queued_prompt_count_for_agent(session, agent.id())
                        > 0
            })
    }

    pub(crate) async fn execute_metaagent_task_request(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::UpdateMetaagentTask(request) => {
                let metaagent =
                    self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                if request.task_markdown.is_none() && request.plan_markdown.is_none() {
                    return Err(DaemonError::LocalTransport {
                        operation: "update_metaagent_task",
                        message: "task_markdown or plan_markdown is required".to_string(),
                    });
                }
                let session_id = request.session_id;
                let metaagent_id = request.metaagent_id;
                let task_updated = request.task_markdown.is_some();
                let plan_updated = request.plan_markdown.is_some();
                {
                    let mut sessions = self.owned.session_store.write();
                    if let Some(task_markdown) = request.task_markdown {
                        sessions.update_metaagent_task_markdown(
                            &session_id,
                            &metaagent_id,
                            task_markdown,
                        )?;
                    } else {
                        let _ = sessions.get_session(&session_id)?;
                    }
                    if let Some(plan_markdown) = request.plan_markdown {
                        sessions.update_metaagent_plan_markdown(
                            &session_id,
                            &metaagent_id,
                            plan_markdown,
                        )?;
                    }
                }
                self.notify_metaagent_task_changed(
                    &session_id,
                    &metaagent,
                    metaagent_task_update_notification(task_updated, plan_updated),
                )
                .await;
                let session = self.owned.session_store.get_session(&session_id)?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &metaagent_id))
            }
            LocalDaemonRequest::PauseMetaagentTask(request) => {
                let metaagent =
                    self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().set_metaagent_task_status(
                    &request.session_id,
                    &request.metaagent_id,
                    MetaagentTaskStatus::Paused,
                )?;
                drop(session);
                self.cancel_active_metaagent_prompt_if_any(
                    &request.session_id,
                    &metaagent,
                    "pause_metaagent_task",
                )
                .await?;
                let session = self.owned.session_store.get_session(&request.session_id)?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            LocalDaemonRequest::ResumeMetaagentTask(request) => {
                let metaagent =
                    self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().set_metaagent_task_status(
                    &request.session_id,
                    &request.metaagent_id,
                    MetaagentTaskStatus::Active,
                )?;
                drop(session);
                self.notify_metaagent_task_changed(
                    &request.session_id,
                    &metaagent,
                    "The user resumed your task. Re-read the task and plan, then continue from the current state.",
                )
                .await;
                let session = self.owned.session_store.get_session(&request.session_id)?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            LocalDaemonRequest::AbortMetaagentTask(request) => {
                let metaagent =
                    self.ensure_session_metaagent(&request.session_id, &request.metaagent_id)?;
                let session = self.owned.session_store.write().abort_metaagent_task(
                    &request.session_id,
                    &request.metaagent_id,
                    request.reason,
                )?;
                drop(session);
                self.cancel_active_metaagent_prompt_if_any(
                    &request.session_id,
                    &metaagent,
                    "abort_metaagent_task",
                )
                .await?;
                let session = self.owned.session_store.get_session(&request.session_id)?;
                let session = self.project_metaagent_task_session(session);
                Ok(metaagent_task_response(session, &request.metaagent_id))
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: "unsupported metaagent task request".to_string(),
            }),
        }
    }

    fn ensure_session_metaagent(
        &self,
        session_id: &str,
        metaagent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(metaagent_id)?;
        if agent.session_id() != session_id || !agent.is_metaagent() {
            return Err(DaemonError::LocalTransport {
                operation: "metaagent_task_request",
                message: format!("agent `{metaagent_id}` is not a metaagent in this session"),
            });
        }
        Ok(agent)
    }

    async fn cancel_active_metaagent_prompt_if_any(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, metaagent.id())
        else {
            return Ok(());
        };
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            return Ok(());
        }
        let attachment_id = self.ensure_metaagent_task_attachment(session_id, metaagent)?;
        match self
            .cancel_agent_prompt(session_id, metaagent.id(), &attachment_id)
            .await
        {
            Ok(cancellation) => {
                if let Some(dispatch) = cancellation.dispatch {
                    self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                }
                Ok(())
            }
            Err(DaemonError::NoActivePrompt { .. }) => Ok(()),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "metaagent.task",
                    "failed to cancel active metaagent prompt",
                    serde_json::json!({
                        "operation": operation,
                        "session_id": session_id,
                        "metaagent_id": metaagent.id(),
                        "error": error.to_string(),
                    }),
                );
                Err(error)
            }
        }
    }

    async fn notify_metaagent_task_changed(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        prompt_text: &str,
    ) {
        let attachment_id = match self.ensure_metaagent_task_attachment(session_id, metaagent) {
            Ok(attachment_id) => attachment_id,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "metaagent.task",
                    "failed to attach metaagent task notifier",
                    serde_json::json!({
                        "session_id": session_id,
                        "metaagent_id": metaagent.id(),
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self
            .submit_metaagent_command_prompt(
                session_id,
                metaagent,
                &attachment_id,
                metaagent.id(),
                prompt_text.to_string(),
            )
            .await
        {
            crate::logging::warn_with_fields(
                "metaagent.task",
                "failed to notify metaagent about task change",
                serde_json::json!({
                    "session_id": session_id,
                    "metaagent_id": metaagent.id(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn ensure_metaagent_task_attachment(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let client_id = format!("metaagent:{}:task", metaagent.id());
        if let Some(attachment) = self
            .owned
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let attachment = self
            .owned
            .attach(crate::attachment::AttachRequest::for_user(
                session_id,
                client_id,
                crate::attachment::ClientCapabilityLevel::AutomationOnly,
                metaagent.owner_user_id(),
            ))?;
        Ok(attachment.id().to_string())
    }

    fn project_metaagent_task_session(
        &self,
        mut session: crate::session::RuntimeSession,
    ) -> crate::session::RuntimeSession {
        let agents = self.owned.agent_store.get_session_agents(session.id());
        session.set_agents(agents);
        self.owned.project_session_runtime_view(&mut session);
        self.owned.session_projection.update(session.clone());
        session
    }
}

fn metaagent_task_response(
    session: crate::session::RuntimeSession,
    metaagent_id: &str,
) -> LocalDaemonResponse {
    let task = session.metaagent_task(metaagent_id).cloned();
    LocalDaemonResponse::MetaagentTaskUpdated { session, task }
}

fn metaagent_task_update_notification(task_updated: bool, plan_updated: bool) -> &'static str {
    match (task_updated, plan_updated) {
        (true, true) => {
            "The user edited your task and plan. Re-read both, revise your approach as needed, and continue."
        }
        (true, false) => {
            "The user edited your task. Re-read it, revise your plan as needed, and continue."
        }
        (false, true) => {
            "The user edited your plan. Re-read it and continue from the updated plan."
        }
        (false, false) => "The user edited your task state. Re-read it and continue.",
    }
}
