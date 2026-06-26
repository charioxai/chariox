use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::session::MetaagentTaskStatus;

use super::KernelRuntimeState;

pub(crate) struct MetaSlashCommand {
    pub(crate) task_prompt: String,
}

pub(crate) fn parse_meta_slash_command(prompt: &str) -> Option<MetaSlashCommand> {
    let trimmed = prompt.trim_start();
    let rest = trimmed.strip_prefix("/meta")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let task_prompt = rest.trim_start().to_string();
    if task_prompt.is_empty() {
        return None;
    }
    Some(MetaSlashCommand { task_prompt })
}

const META_MODE_ENTERED_CONTEXT: &str = "Kernel mode transition: this agent is now operating in Arroba meta mode for the active task. Delegate implementation to owned regular agents or workflows, use Arroba meta tools for planning and supervision, and finish by calling arroba.meta.complete_task, arroba.meta.mark_blocked, or by honoring user pause/abort controls.";

const META_MODE_EXITED_CONTEXT: &str = "Kernel mode transition: this agent has left Arroba meta mode and is now a regular agent again. Meta runtime tools are no longer available. Continue future work normally under the restored provider profile.";

impl KernelRuntimeState {
    pub(crate) async fn activate_meta_mode_for_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        task_prompt: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "meta_mode.activate",
                message: format!("agent `{agent_id}` is not in session `{session_id}`"),
            });
        }
        self.owned
            .agent_store
            .activate_agent_meta_mode(agent_id, None)?;
        let session = {
            let mut sessions = self.owned.session_store.write();
            let session = sessions.start_metaagent_task_if_needed(session_id, agent_id, task_prompt)?;
            match session {
                Some(session) => session,
                None => sessions.get_session(session_id)?,
            }
        };
        let task_id = session
            .metaagent_task(agent_id)
            .map(|task| task.task_id().to_string());
        self.owned
            .agent_store
            .activate_agent_meta_mode(agent_id, task_id)?;
        let _ = self.reload_agent_provider_if_idle(session_id, agent_id, "meta mode activation")?;
        Ok(self.project_metaagent_task_session(
            self.owned.session_store.get_session(session_id)?,
        ))
    }

    pub(crate) async fn deactivate_meta_mode_for_terminal_task(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id || !agent.is_metaagent() {
            return Ok(self.owned.session_store.get_session(session_id)?);
        }
        self.owned.agent_store.deactivate_agent_meta_mode(agent_id)?;
        let _ = self.reload_agent_provider_if_idle(session_id, agent_id, reason)?;
        if let Err(error) = self
            .submit_meta_mode_exited_prompt(session_id, agent_id, reason)
            .await
        {
            crate::logging::warn_with_fields(
                "metaagent.task",
                "failed to submit meta mode exit continuation prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "reason": reason,
                    "error": error.to_string(),
                }),
            );
        }
        Ok(self.owned.session_store.get_session(session_id)?)
    }

    pub(crate) fn meta_mode_entered_hidden_context() -> &'static str {
        META_MODE_ENTERED_CONTEXT
    }

    async fn submit_meta_mode_exited_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let attachment_id = self.ensure_metaagent_task_attachment(
            session_id,
            &self.owned.agent_store.get_agent(agent_id)?,
        )?;
        let prompt = crate::session::PromptQueueItem::new(
            self.owned.session_store.reserve_prompt_id(),
            attachment_id,
            agent_id,
            format!(
                "Kernel mode transition: meta mode ended after {reason}. You are now a regular agent again. Meta tools are no longer available."
            ),
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(META_MODE_EXITED_CONTEXT);
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            })
            .await?;
        if let (crate::session::PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &dispatch.session_id,
                &dispatch.agent_id,
                prompt.id(),
                &dispatch.provider_run_id,
                "meta-mode-exited",
            );
        }
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }

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
            "Your last turn ended while task `{}` is still active and no controlled regular agents have active or queued work. Decide the task state now: if it is done, call `arroba.meta.complete_task`; if it cannot be completed after exhausting options, call `arroba.meta.mark_blocked`; otherwise update your plan and continue/delegate the remaining work. Do not answer only in natural language; update the kernel-managed task state through the metaagent runtime tools.",
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
            .filter(|agent| agent.controlled_by_metaagent_id() == Some(metaagent.id()))
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
                let session = self
                    .deactivate_meta_mode_for_terminal_task(
                        &request.session_id,
                        &request.metaagent_id,
                        "meta task abort",
                    )
                    .await?;
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

#[cfg(test)]
mod tests {
    use super::parse_meta_slash_command;

    #[test]
    fn parse_meta_slash_command_only_matches_first_command_token() {
        let parsed = parse_meta_slash_command("  /meta Build the thing")
            .expect("/meta with task should parse");
        assert_eq!(parsed.task_prompt, "Build the thing");

        assert!(parse_meta_slash_command("/metadata should stay normal").is_none());
        assert!(parse_meta_slash_command("please /meta do this").is_none());
        assert!(parse_meta_slash_command("/meta").is_none());
        assert!(parse_meta_slash_command("/meta\tTabbed task").is_some());
    }
}
