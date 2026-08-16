use super::*;

impl KernelRuntimeState {
    pub(crate) async fn create_agent_prompt_schedule(
        &self,
        request: crate::local::CreateAgentPromptScheduleRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&request.agent_id)?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        let schedule = self
            .owned
            .session_store
            .write()
            .create_agent_prompt_schedule(
                &request.session_id,
                &request.agent_id,
                request.kind,
                request.interval_seconds,
                request.prompt,
            )?;
        let session = self.owned.persist_agent_prompt_schedule_session(
            &request.session_id,
            "agent_prompt_schedule_created",
        )?;
        Ok(crate::local::LocalDaemonResponse::AgentPromptScheduleCreated { schedule, session })
    }

    pub(crate) async fn cancel_agent_prompt_schedule(
        &self,
        request: crate::local::CancelAgentPromptScheduleRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let schedule = self
            .owned
            .session_store
            .write()
            .cancel_agent_prompt_schedule(&request.session_id, &request.schedule_id)?;
        let session = self.owned.persist_agent_prompt_schedule_session(
            &request.session_id,
            "agent_prompt_schedule_cancelled",
        )?;
        Ok(crate::local::LocalDaemonResponse::AgentPromptScheduleCancelled { schedule, session })
    }

    pub(crate) async fn dispatch_due_agent_prompt_schedules(&self, now_ms: u64) {
        let dispatches = self
            .owned
            .session_store
            .write()
            .claim_due_agent_prompt_schedules(now_ms)
            .dispatches;
        for dispatch in dispatches {
            let result = self.dispatch_agent_prompt_schedule(&dispatch).await;
            let update = match result {
                Ok(()) => self
                    .owned
                    .session_store
                    .write()
                    .mark_agent_prompt_schedule_dispatched(
                        &dispatch.session_id,
                        &dispatch.schedule_id,
                        now_ms,
                    ),
                Err(error) => {
                    self.owned.record_notice_for_agent(
                        &dispatch.session_id,
                        None,
                        Some(&dispatch.agent_id),
                        self.owned
                            .attachment_store
                            .list_session_attachment_ids(&dispatch.session_id),
                        format!(
                            "Scheduled prompt `{}` could not be delivered: {error}",
                            dispatch.schedule_id
                        ),
                    );
                    self.owned
                        .session_store
                        .write()
                        .mark_agent_prompt_schedule_failed(
                            &dispatch.session_id,
                            &dispatch.schedule_id,
                            now_ms,
                            error.to_string(),
                        )
                }
            };
            if let Err(error) = update.and_then(|_| {
                self.owned
                    .persist_agent_prompt_schedule_session(
                        &dispatch.session_id,
                        "agent_prompt_schedule_tick",
                    )
                    .map(|_| ())
            }) {
                crate::logging::warn_with_fields(
                    "daemon.agent_prompt_schedule",
                    "agent prompt schedule state update failed",
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "schedule_id": dispatch.schedule_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    async fn dispatch_agent_prompt_schedule(
        &self,
        dispatch: &crate::session::AgentPromptScheduleDispatch,
    ) -> Result<(), DaemonError> {
        let source_attachment_id = self
            .owned
            .ensure_agent_prompt_schedule_attachment(&dispatch.session_id)?;
        let prompt_id = self.owned.session_store.reserve_prompt_id();
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id,
            source_attachment_id,
            &dispatch.agent_id,
            &dispatch.prompt,
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(crate::prompt_assembly::render_configured_prompt(
            "runtime/scheduled-prompt",
            crate::prompt_assembly::bundled_prompt_template("runtime/scheduled-prompt")
                .expect("scheduled prompt context must be registered"),
            &[("SCHEDULE_ID", &dispatch.schedule_id)],
        ));
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: dispatch.session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .await?;
        if let (
            crate::session::PromptSubmissionOutcome::Started { prompt },
            Some(runtime_dispatch),
        ) = (&submission.outcome, submission.dispatch.as_ref())
        {
            self.start_active_turn_with_trace_id(
                &runtime_dispatch.session_id,
                &runtime_dispatch.agent_id,
                prompt.id(),
                &runtime_dispatch.provider_run_id,
                "scheduled-prompt",
            );
        }
        if let Some(runtime_dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(runtime_dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(remote_dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(remote_dispatch);
        }
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    fn ensure_agent_prompt_schedule_attachment(
        &self,
        session_id: &str,
    ) -> Result<String, DaemonError> {
        let client_id = format!("agent-schedules:{session_id}");
        if let Some(attachment) = self
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .find(|attachment| attachment.session_id() == session_id)
        {
            return Ok(attachment.id().to_string());
        }
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.attach(crate::attachment::AttachRequest::for_user(
            session_id,
            client_id,
            crate::attachment::ClientCapabilityLevel::AutomationOnly,
            session.owner_user_id(),
        ))?;
        Ok(attachment.id().to_string())
    }

    fn persist_agent_prompt_schedule_session(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        self.durable_state_store.append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({
                "session": &session,
                "reason": reason,
            }),
        )?;
        Ok(session)
    }
}
