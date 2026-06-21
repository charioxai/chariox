use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let attachment = self.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(super) fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }

        let mut sessions = self.session_store.write();
        let attachment = self.attachment_store.attach(&mut sessions, request)?;
        drop(sessions);

        if self.agent_store.get_session_agents(&session_id).is_empty() {
            let session = self.session_store.get_session(&session_id)?;
            let agent_request = session::agent_request_from_session_defaults(&session, None)
                .with_worktree(session.worktree_id());
            let mut sessions = self.session_store.write();
            let _ = self
                .agent_store
                .create_agent(agent_request, &mut sessions)?;
            drop(sessions);
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.sync_focused_provider_run_if_idle(&session_id)?;

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    pub(super) fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let mut sessions = self.session_store.write();
        let (attachment, effect) = self
            .attachment_store
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);
        self.terminal_stream
            .remove_attachment(attachment.session_id(), attachment_id);

        let session = self.session_store.get_session(attachment.session_id())?;
        let owner_removed_queued_prompt_count = self
            .prompt_state_owner
            .remove_queued_prompts_by_attachment(&session, attachment_id);
        self.mirror_prompt_owner_session_state(attachment.session_id())?;
        let removed_queued_prompt_count = effect
            .removed_queued_prompt_count
            .max(owner_removed_queued_prompt_count);
        let session_after_detach = self.session_store.get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self.activate_next_queued_prompt_for_agent(
                    attachment.session_id(),
                    agent_id,
                    None,
                )?;
            }
        }

        let remaining_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(attachment.session_id());
        let active_prompt_agent_id = self
            .prompt_state_owner
            .active_prompt_agent_id(&self.session_snapshot(attachment.session_id())?);
        if remaining_attachment_ids.is_empty() && active_prompt_agent_id.is_none() {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                let run = self.provider_store.get_run(&active_provider_run_id)?;
                if run.state() != crate::provider::ProviderRunState::Ended {
                    let outcome = self
                        .provider_store
                        .park_run_provider_only(attachment.session_id(), &active_provider_run_id)?;
                    if self
                        .session_store
                        .get_session(attachment.session_id())?
                        .active_provider_run_id()
                        == Some(outcome.run().id())
                    {
                        self.session_store
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    self.provider_run_projection.update(outcome.into_run());
                }
            }
            for run in self.provider_store.list_runs() {
                if run.session_id() == attachment.session_id() {
                    self.clear_prompt_activity(run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );
        self.session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }
}
