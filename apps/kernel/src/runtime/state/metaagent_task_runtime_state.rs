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

impl KernelRuntimeState {
    pub(super) fn persist_metaagent_task_session_update(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session = self.owned.session_snapshot(session_id)?;
        self.owned.durable_state_store.append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({
                "session": &session,
                "reason": reason,
            }),
        )?;
        Ok(session)
    }

    pub(crate) fn session_task_lane_busy(&self, session_id: &str) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(session.has_active_session_task() || session.has_pending_session_task())
    }

    pub(crate) fn enqueue_metaagent_task(
        &self,
        session_id: &str,
        metaagent_id: &str,
        source_attachment_id: &str,
        task_markdown: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<
        (
            crate::session::QueuedMetaagentTask,
            crate::session::RuntimeSession,
        ),
        DaemonError,
    > {
        let task = self.owned.session_store.write().enqueue_metaagent_task(
            session_id,
            metaagent_id,
            source_attachment_id,
            task_markdown,
            attachments,
        )?;
        let session = self.owned.session_snapshot(session_id)?;
        self.owned.durable_state_store.append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({
                "session": &session,
                "reason": "metaagent_task_queued",
            }),
        )?;
        Ok((task, session))
    }

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
        if let Err(error) = self
            .sync_remote_leased_agent_meta_mode(session_id, agent_id, true)
            .await
        {
            let _ = self.owned.agent_store.deactivate_agent_meta_mode(agent_id);
            return Err(error);
        }
        if let Err(error) = self
            .reload_agent_provider_for_policy(session_id, agent_id, "meta mode activation")
            .await
        {
            let _ = self
                .sync_remote_leased_agent_meta_mode(session_id, agent_id, false)
                .await;
            let _ = self.owned.agent_store.deactivate_agent_meta_mode(agent_id);
            return Err(error);
        }
        let session_result = (|| {
            let mut sessions = self.owned.session_store.write();
            sessions.start_or_update_metaagent_task(session_id, agent_id, task_prompt)
        })();
        let session = match session_result {
            Ok(session) => session,
            Err(error) => {
                let _ = self
                    .sync_remote_leased_agent_meta_mode(session_id, agent_id, false)
                    .await;
                let _ = self.owned.agent_store.deactivate_agent_meta_mode(agent_id);
                let _ = self
                    .reload_agent_provider_for_policy(
                        session_id,
                        agent_id,
                        "meta mode activation rollback",
                    )
                    .await;
                return Err(error);
            }
        };
        let task_id = session
            .metaagent_task(agent_id)
            .map(|task| task.task_id().to_string());
        if let Err(error) = self
            .owned
            .agent_store
            .activate_agent_meta_mode(agent_id, task_id)
        {
            let _ = self
                .sync_remote_leased_agent_meta_mode(session_id, agent_id, false)
                .await;
            let _ = self.owned.agent_store.deactivate_agent_meta_mode(agent_id);
            let _ = self
                .reload_agent_provider_for_policy(
                    session_id,
                    agent_id,
                    "meta mode activation rollback",
                )
                .await;
            return Err(error);
        }
        let session =
            self.persist_metaagent_task_session_update(session_id, "metaagent_task_started")?;
        Ok(self.project_metaagent_task_session(session))
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
        let task_attachment_id = self.ensure_metaagent_task_attachment(session_id, &agent)?;
        let _ = self.owned.remove_queued_metaagent_event_prompts_for_agent(
            session_id,
            agent_id,
            &task_attachment_id,
        )?;
        let session = self.owned.session_store.get_session(session_id)?;
        if session
            .queued_metaagent_tasks()
            .iter()
            .any(|task| task.metaagent_id() == agent_id)
        {
            return Ok(session);
        }
        self.sync_remote_leased_agent_meta_mode(session_id, agent_id, false)
            .await?;
        self.owned
            .agent_store
            .deactivate_agent_meta_mode(agent_id)?;
        let _ = self
            .reload_agent_provider_for_policy(session_id, agent_id, reason)
            .await?;
        Ok(self.owned.session_store.get_session(session_id)?)
    }

    pub(crate) fn meta_mode_entered_hidden_context() -> Result<String, DaemonError> {
        let (context, _manifest) = crate::prompt_assembly::PromptAssemblyService::from_env()?
            .assemble_meta_mode_entered_context()?;
        Ok(context)
    }

    async fn sync_remote_leased_agent_meta_mode(
        &self,
        session_id: &str,
        agent_id: &str,
        active: bool,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "sync remote leased agent meta mode",
                message: format!("agent `{agent_id}` is not in session `{session_id}`"),
            });
        }
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let mut config = self.config_snapshot().await;
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.apply_remote_relay_override(relay_url, relay_token);
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                &config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                crate::transport::relay_peer::RelayPeerRequest::UpdateLeasedAgentMetaMode {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    active,
                },
            ),
        )
        .await
        {
            Ok(Ok(
                crate::transport::relay_peer::RelayPeerResponse::LeasedAgentMetaModeUpdated {
                    ..
                },
            )) => Ok(()),
            Ok(Ok(other)) => Err(DaemonError::LocalTransport {
                operation: "sync remote leased agent meta mode",
                message: format!("unexpected remote meta mode response: {other:?}"),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(DaemonError::LocalTransport {
                operation: "sync remote leased agent meta mode",
                message: "timed out waiting for remote worker meta mode update".to_string(),
            }),
        }
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
        let Some(_session) = self
            .owned
            .session_store
            .write()
            .start_metaagent_task_if_needed(session_id, metaagent_id, prompt)?
        else {
            return Ok(None);
        };
        let session =
            self.persist_metaagent_task_session_update(session_id, "metaagent_task_started")?;
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
        if self.metaagent_has_unfinished_controlled_work(&session, &metaagent) {
            return Ok(());
        }
        if self
            .owned
            .metaagent_events
            .has_orphaned_task_event_for_revision(metaagent.id(), task.task_id(), task.revision())
        {
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
            "Your last turn ended while task `{}` is still active and no controlled regular agents have active or queued work. Decide the task state now: if it is done, call `chariox.meta.complete_task`; if it cannot be completed after exhausting options, call `chariox.meta.mark_blocked`; otherwise update your plan and continue/delegate the remaining work. Do not answer only in natural language; update the kernel-managed task state through the metaagent runtime tools.",
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

    pub(crate) fn metaagent_has_unfinished_controlled_work(
        &self,
        session: &crate::session::RuntimeSession,
        metaagent: &crate::agent::AgentInstance,
    ) -> bool {
        let activity = self.agent_activity_for_session(session);
        let active_agent_work = self
            .owned
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
            });
        active_agent_work
            || session.workflow_runs().iter().any(|run| {
                matches!(
                    run.status(),
                    crate::session::WorkflowRunStatus::Created
                        | crate::session::WorkflowRunStatus::Running
                        | crate::session::WorkflowRunStatus::Waiting
                        | crate::session::WorkflowRunStatus::Completing
                        | crate::session::WorkflowRunStatus::Paused
                ) && session.workflow(run.workflow_id()).is_some_and(|workflow| {
                    workflow.controlled_by_metaagent_id() == Some(metaagent.id())
                })
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
                self.persist_metaagent_task_session_update(&session_id, "metaagent_task_updated")?;
                let notification = {
                    let session = self.owned.session_store.get_session(&session_id)?;
                    metaagent_task_update_notification(
                        task_updated,
                        plan_updated,
                        session.metaagent_task(&metaagent_id),
                    )
                };
                self.notify_metaagent_task_changed(&session_id, &metaagent, &notification, true)
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
                self.persist_metaagent_task_session_update(
                    &request.session_id,
                    "metaagent_task_paused",
                )?;
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
                self.persist_metaagent_task_session_update(
                    &request.session_id,
                    "metaagent_task_resumed",
                )?;
                let queued_prompt = self.owned.prompt_state_owner.peek_next_queued_prompt(
                    &self.owned.session_store.get_session(&request.session_id)?,
                    &request.metaagent_id,
                );
                let queued_task_notification = queued_prompt.as_ref().is_some_and(|prompt| {
                    prompt.prompt()
                        == crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
                });
                let has_active_prompt = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(
                        &self.owned.session_store.get_session(&request.session_id)?,
                        &request.metaagent_id,
                    )
                    .is_some();
                if queued_prompt.is_some() && !has_active_prompt {
                    if metaagent.remote_execution().is_some() {
                        if let Some(mut submission) =
                            self.owned.advance_next_queued_remote_prompt_dispatch(
                                &request.session_id,
                                &request.metaagent_id,
                            )?
                        {
                            self.finish_owned_prompt_submission_workflow_start(&mut submission)
                                .await?;
                            self.spawn_remote_prompt_projection_drain_if_needed(&submission);
                            if let Some(dispatch) = submission.remote_dispatch.take() {
                                self.spawn_remote_prompt_dispatch(dispatch);
                            }
                        }
                    } else {
                        let provider_run_id = self
                            .owned
                            .provider_store
                            .get_run_for_agent(&request.session_id, &request.metaagent_id)
                            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                                session_id: request.session_id.clone(),
                            })?
                            .id()
                            .to_string();
                        if let Some(dispatch) = self.owned.advance_next_queued_prompt_dispatch(
                            &request.session_id,
                            &request.metaagent_id,
                            &provider_run_id,
                        )? {
                            self.spawn_prompt_dispatch(
                                dispatch,
                                self.provider_runtime_lanes.clone(),
                            );
                        }
                    }
                }
                if !queued_task_notification {
                    self.notify_metaagent_task_changed(
                        &request.session_id,
                        &metaagent,
                        "The user resumed your task. Re-read the task and plan, then continue from the current state.",
                        false,
                    )
                    .await;
                }
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
                self.persist_metaagent_task_session_update(
                    &request.session_id,
                    "metaagent_task_aborted",
                )?;
                self.cancel_active_metaagent_prompt_if_any(
                    &request.session_id,
                    &metaagent,
                    "abort_metaagent_task",
                )
                .await?;
                self.cancel_controlled_regular_agent_work(
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
                self.spawn_workflow_prompt_dispatches(
                    self.owned
                        .workflow_maybe_start_next_queued_prompt(&request.session_id),
                );
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

    async fn cancel_controlled_regular_agent_work(
        &self,
        session_id: &str,
        metaagent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let controlled_agents = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| !agent.is_metaagent())
            .filter(|agent| agent.controlled_by_metaagent_id() == Some(metaagent.id()))
            .collect::<Vec<_>>();
        for agent in controlled_agents {
            let _ = self
                .owned
                .remove_queued_prompts_for_agent(session_id, agent.id())?;
            let session = self.owned.session_store.get_session(session_id)?;
            let Some(active_prompt) = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent.id())
            else {
                continue;
            };
            if active_prompt.status() == crate::session::PromptStatus::Cancelling {
                continue;
            }
            let attachment_id = active_prompt.source_attachment_id().to_string();
            match self
                .cancel_agent_prompt(session_id, agent.id(), &attachment_id)
                .await
            {
                Ok(cancellation) => {
                    if let Some(dispatch) = cancellation.dispatch {
                        self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                    }
                }
                Err(DaemonError::NoActivePrompt { .. }) => {}
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "metaagent.task",
                        "failed to cancel controlled worker prompt",
                        serde_json::json!({
                            "operation": operation,
                            "session_id": session_id,
                            "metaagent_id": metaagent.id(),
                            "worker_agent_id": agent.id(),
                            "error": error.to_string(),
                        }),
                    );
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn cancel_active_metaagent_prompt_if_any(
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
        allow_steer: bool,
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
        let paused = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| session.metaagent_task(metaagent.id()).cloned())
            .is_some_and(|task| task.status() == MetaagentTaskStatus::Paused);
        let result = self
            .submit_metaagent_task_notification(
                session_id,
                metaagent,
                &attachment_id,
                prompt_text.to_string(),
                allow_steer && !paused,
                paused,
            )
            .await;
        if let Err(error) = result {
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

    pub(super) fn ensure_metaagent_task_attachment(
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
        self.owned.update_session_projection(session)
    }
}

fn metaagent_task_response(
    session: crate::session::RuntimeSession,
    metaagent_id: &str,
) -> LocalDaemonResponse {
    let task = session.metaagent_task(metaagent_id).cloned();
    LocalDaemonResponse::MetaagentTaskUpdated { session, task }
}

fn metaagent_task_update_notification(
    task_updated: bool,
    plan_updated: bool,
    task: Option<&crate::session::MetaagentTask>,
) -> String {
    let lead = match (task_updated, plan_updated) {
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
    };
    let Some(task) = task else {
        return lead.to_string();
    };
    format!(
        "{lead}\n\nUpdated task:\n{}\n\nUpdated plan:\n{}",
        task.task_markdown(),
        task.plan_markdown(),
    )
}

#[cfg(test)]
mod task_notification_tests {
    use super::metaagent_task_update_notification;
    use crate::session::MetaagentTask;

    #[test]
    fn task_edit_notification_carries_the_authoritative_task_and_plan() {
        let mut task = MetaagentTask::new("task-1", "meta-1", "Build the viewer");
        task.update_plan_markdown("1. Inspect\n2. Fix");
        let notification = metaagent_task_update_notification(true, false, Some(&task));

        assert!(notification.contains("Updated task:\nBuild the viewer"));
        assert!(notification.contains("Updated plan:\n1. Inspect\n2. Fix"));
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
