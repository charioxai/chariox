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
        let reopens = sessions
            .get_session(&session_id)?
            .attach_creates_default_agent();
        let attachment = self.attachment_store.attach(&mut sessions, request)?;
        drop(sessions);

        if reopens && self.agent_store.get_session_agents(&session_id).is_empty() {
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
                    "reason": "an ended session reopened with no agents",
                }),
            );
        }

        if let Err(error) = self.sync_focused_provider_run_if_idle(&session_id) {
            match error {
                DaemonError::ProviderRunNotFound { provider_run_id } => {
                    self.session_store
                        .set_active_provider_run(&session_id, None)?;
                    crate::logging::warn_with_fields(
                        "daemon.session",
                        "cleared stale provider run while attaching session",
                        serde_json::json!({
                            "session_id": session_id,
                            "provider_run_id": provider_run_id,
                        }),
                    );
                }
                error => return Err(error),
            }
        }

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

        self.mirror_prompt_owner_session_state(attachment.session_id())?;
        let removed_queued_prompt_count = effect.removed_queued_prompt_count;
        let session_after_detach = self.session_store.get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {removed_queued_prompt_count} queued prompt(s) from detached attachment `{attachment_id}`."
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
                    "Removed the active prompt from detached attachment `{attachment_id}` and advanced the queue."
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
        self.park_detached_idle_provider_run(attachment.session_id())?;

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

impl KernelRuntimeState {
    /// MP-11 SB-01: public automation cannot borrow a human attachment to
    /// turn its prompt into an owner-authored capability request.
    pub(crate) fn authorize_prompt_attachment_role(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        if !self.room_agent_tools_enabled()
            || command.is_terminal_caller()
            || (command.caller.connection_class.is_none() && command.caller.metaagent_id.is_none())
        {
            // No connection class denotes a kernel-owned command, never an
            // admitted public connection (local and relay transports set it).
            return Ok(());
        }
        let check = |session: &str, id: &str| {
            let attachment = self.owned.ensure_attachment_in_session(session, id)?;
            if matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            ) {
                return Err(crate::runtime::room_tool_admission::denied(
                    "automated prompts require an automated attachment",
                ));
            }
            Ok(())
        };
        match request {
            LocalDaemonRequest::SubmitPrompt(prompt) => {
                check(&prompt.session_id, &prompt.attachment_id)
            }
            LocalDaemonRequest::UpdateQueuedPrompt(prompt) => {
                check(&prompt.session_id, &prompt.attachment_id)
            }
            LocalDaemonRequest::SubmitPrompts(batch) => {
                for item in &batch.prompts {
                    check(
                        item.session_id.as_deref().unwrap_or(&batch.session_id),
                        item.attachment_id
                            .as_deref()
                            .unwrap_or(&batch.attachment_id),
                    )?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// MP-08/MP-11 SB-01: admission assigns the human/automation attachment
    /// role; a caller-selected terminal capability never establishes provenance.
    pub(crate) async fn attach_for_caller(
        &self,
        mut request: crate::attachment::AttachRequest,
        terminal_caller: bool,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        if !terminal_caller
            && matches!(
                request.capability_level,
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            request.capability_level = crate::attachment::ClientCapabilityLevel::AutomationOnly;
        }
        self.attach(request).await
    }
}
