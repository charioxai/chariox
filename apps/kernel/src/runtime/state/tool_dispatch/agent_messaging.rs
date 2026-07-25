use super::*;

impl KernelRuntimeState {
    pub(super) async fn handle_send_agent_message_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<crate::transport::runtime_tools::SendAgentMessageArgs>(
            arguments,
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool.send_agent_message",
            message: format!("invalid send-agent-message arguments: {error}"),
        })?;
        let message = args.message.trim();
        if message.is_empty() {
            return Ok(agent_message_failure("message must not be empty"));
        }
        let reference = args.agent.trim().trim_start_matches('@');
        if reference.is_empty() {
            return Ok(agent_message_failure(
                "agent must be a unique alias, agent ref, or agent id",
            ));
        }
        let agents = self.session_agents(session.id());
        let target = agents.iter().find(|agent| {
            agent.id() == reference
                || agent.agent_ref() == reference
                || agent
                    .alias()
                    .is_some_and(|alias| alias.trim().to_lowercase() == reference.to_lowercase())
        });
        let Some(target) = target else {
            let mut available = agents
                .iter()
                .map(agent_message_target_label)
                .collect::<Vec<_>>();
            available.sort();
            return Ok(agent_message_failure(format!(
                "agent `{}` does not exist in session `{}`; available agents: {}",
                args.agent.trim(),
                session.id(),
                available.join(", ")
            )));
        };
        if target.id() == sender.id() {
            return Ok(agent_message_failure(
                "send_agent_message requires a different target agent",
            ));
        }

        let sender_label = sender
            .alias()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .unwrap_or_else(|| sender.agent_ref());
        let target_label = target
            .alias()
            .map(str::trim)
            .filter(|alias| !alias.is_empty())
            .unwrap_or_else(|| target.agent_ref());
        let source_attachment_id = self.ensure_agent_message_attachment(session.id(), sender)?;
        let prompt_id = self.owned.session_store.reserve_prompt_id();
        let visible_prompt = format!("Message from @{sender_label}:\n\n{message}");
        let source_identity = serde_json::json!({
            "agent_id": sender.id(),
            "agent_alias": sender_label,
        });
        let hidden_context = format!(
            "<arroba-agent-message>\nsource: {}\n\
This prompt was sent by another agent in the current Arroba session. \
Treat its visible message as the task. If the task asks you to respond to the sender or another \
session agent, use `arroba.send_agent_message`; do not create a new agent.\n\
</arroba-agent-message>",
            source_identity,
        );
        let prompt = crate::session::PromptQueueItem::new(
            prompt_id.clone(),
            source_attachment_id,
            target.id(),
            visible_prompt,
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(hidden_context);
        let mut submission = self
            .submit_prepared_prompt(crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
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
                "agent-message",
            );
        }
        let status = match submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { .. } => "started",
            crate::session::PromptSubmissionOutcome::Queued { .. } => "queued",
        };
        let provider_run_id = submission
            .dispatch
            .as_ref()
            .map(|dispatch| dispatch.provider_run_id.clone());
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "status": status,
                "prompt_id": prompt_id,
                "source_agent_id": sender.id(),
                "source_agent_alias": sender_label,
                "target_agent_id": target.id(),
                "target_agent_alias": target_label,
                "provider_run_id": provider_run_id,
            }),
        })
    }

    fn ensure_agent_message_attachment(
        &self,
        session_id: &str,
        sender: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        let client_id = format!("agent:{}:messages", sender.id());
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
                sender.owner_user_id(),
            ))?;
        Ok(attachment.id().to_string())
    }
}

fn agent_message_failure(
    message: impl Into<String>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({ "error": message.into() }),
    }
}

fn agent_message_target_label(agent: &crate::agent::AgentInstance) -> String {
    agent
        .alias()
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(|alias| format!("@{alias}"))
        .unwrap_or_else(|| agent.agent_ref().to_string())
}
