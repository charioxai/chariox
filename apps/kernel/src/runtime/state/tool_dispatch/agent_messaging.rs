use super::*;
use sha2::{Digest, Sha256};

impl KernelRuntimeState {
    pub(super) fn handle_list_session_agents_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        requester: &crate::agent::AgentInstance,
    ) -> crate::transport::runtime_tools::RuntimeToolResult {
        let mut agents = self.session_agents(session.id());
        agents.sort_by(|left, right| {
            left.created_at_ms()
                .cmp(&right.created_at_ms())
                .then_with(|| left.id().cmp(right.id()))
        });
        crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "session_id": session.id(),
                "focused_agent_id": session.focused_agent_id(),
                "agents": agents
                    .iter()
                    .map(|agent| session_agent_description(session, requester, agent))
                    .collect::<Vec<_>>(),
            }),
        }
    }

    pub(super) fn handle_get_session_agent_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        requester: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> crate::transport::runtime_tools::RuntimeToolResult {
        let args = match serde_json::from_value::<
            crate::transport::runtime_tools::GetSessionAgentArgs,
        >(arguments)
        {
            Ok(args) => args,
            Err(error) => {
                return agent_message_failure(format!(
                    "invalid get-session-agent arguments: {error}"
                ));
            }
        };
        let agents = self.session_agents(session.id());
        match resolve_session_agent(&agents, &args.agent) {
            Ok(agent) => crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "session_id": session.id(),
                    "agent": session_agent_description(session, requester, agent),
                }),
            },
            Err(message) => agent_message_failure(message),
        }
    }

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
        let idempotency_key = args
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if args.idempotency_key.is_some() && idempotency_key.is_none() {
            return Ok(agent_message_failure(
                "idempotency_key must not be empty when provided",
            ));
        }
        let agents = self.session_agents(session.id());
        let target = match resolve_session_agent(&agents, &args.agent) {
            Ok(target) => target,
            Err(message) => {
                return Ok(agent_message_failure(message));
            }
        };
        if target.id() == sender.id() {
            return Ok(agent_message_failure(
                "send_agent_message requires a different target agent",
            ));
        }
        let durable_identity = idempotency_key.map(|key| {
            let operation_id = format!(
                "agent-message:{}:{:x}",
                sender.id(),
                Sha256::digest(key.as_bytes())
            );
            let fingerprint = serde_json::to_vec(&serde_json::json!({
                "target_agent_id": target.id(),
                "message": message,
            }))
            .map(|payload| format!("sha256:{:x}", Sha256::digest(payload)))
            .unwrap_or_default();
            (operation_id, fingerprint)
        });
        let mut idempotency_store =
            if let Some((operation_id, fingerprint)) = durable_identity.as_ref() {
                let store = self.owned.agent_message_idempotency.lock().await;
                if let Some(existing) = store.entries.get(operation_id) {
                    if existing.fingerprint == *fingerprint {
                        return Ok(existing.result.clone());
                    }
                    return Ok(agent_message_failure(
                        "idempotency_key was already used for a different agent message",
                    ));
                }
                Some(store)
            } else {
                None
            };

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
        let mut prompt = crate::session::PromptQueueItem::new(
            prompt_id.clone(),
            source_attachment_id,
            target.id(),
            visible_prompt,
            crate::session::PromptStatus::Queued,
        )
        .with_hidden_system_context(hidden_context);
        if let Some((operation_id, fingerprint)) = durable_identity.as_ref() {
            prompt = prompt.with_durable_operation(operation_id, fingerprint);
        }
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
        let result = crate::transport::runtime_tools::RuntimeToolResult {
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
        };
        if let (Some(store), Some((operation_id, fingerprint))) =
            (idempotency_store.as_mut(), durable_identity)
        {
            store.record(operation_id, fingerprint, result.clone());
        }
        Ok(result)
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

fn resolve_session_agent<'a>(
    agents: &'a [crate::agent::AgentInstance],
    reference: &str,
) -> Result<&'a crate::agent::AgentInstance, String> {
    let reference = reference.trim().trim_start_matches('@');
    if reference.is_empty() {
        return Err("agent must be a unique alias, agent ref, or agent id".to_string());
    }
    agents
        .iter()
        .find(|agent| {
            agent.id() == reference
                || agent.agent_ref() == reference
                || agent
                    .alias()
                    .is_some_and(|alias| alias.trim().eq_ignore_ascii_case(reference))
        })
        .ok_or_else(|| {
            let mut available = agents
                .iter()
                .map(agent_message_target_label)
                .collect::<Vec<_>>();
            available.sort();
            format!(
                "agent `{reference}` does not exist in this session; available agents: {}",
                available.join(", ")
            )
        })
}

fn session_agent_description(
    session: &crate::session::RuntimeSession,
    requester: &crate::agent::AgentInstance,
    agent: &crate::agent::AgentInstance,
) -> serde_json::Value {
    let effective = crate::session::effective_agent_execution_config(session, Some(agent));
    let location = agent.remote_execution().map_or_else(
        || serde_json::json!({ "kind": "local" }),
        |remote| {
            serde_json::json!({
                "kind": "remote",
                "kernel_id": remote.worker_kernel_id,
                "machine_id": remote.worker_machine_id,
            })
        },
    );
    let extension_names = |kind| {
        agent
            .extension_grants()
            .iter()
            .filter(|grant| grant.kind == kind)
            .map(|grant| grant.name.clone())
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "id": agent.id(),
        "agent_ref": agent.agent_ref(),
        "alias": agent.alias(),
        "address": agent_message_target_label(agent),
        "is_self": agent.id() == requester.id(),
        "is_focused": session.focused_agent_id() == Some(agent.id()),
        "provider": agent.provider(),
        "model": agent.model(),
        "effort": agent.effort(),
        "account_profile": agent.provider_account_profile(),
        "execution_mode": effective.mode.as_str(),
        "permission_level": effective.permission_level.as_str(),
        "operating_mode": match agent.operating_mode() {
            crate::agent::AgentOperatingMode::Regular => "regular",
            crate::agent::AgentOperatingMode::Meta => "meta",
        },
        "state": match agent.state() {
            crate::agent::AgentState::Idle => "idle",
            crate::agent::AgentState::Working => "working",
            crate::agent::AgentState::Focused => "focused",
            crate::agent::AgentState::Error => "error",
        },
        "is_processing": agent.is_processing(),
        "has_active_prompt": session.active_prompt_for_agent(agent.id()).is_some(),
        "queued_prompt_count": session
            .queued_prompts_for_agent(agent.id())
            .map_or(0, std::collections::VecDeque::len),
        "controlled_by_metaagent_id": agent.controlled_by_metaagent_id(),
        "visible_in_freeform": agent.visible_in_freeform(),
        "location": location,
        "extensions": {
            "mcps": extension_names(crate::extension::ExtensionKind::Mcp),
            "skills": extension_names(crate::extension::ExtensionKind::Skill),
            "scripts": extension_names(crate::extension::ExtensionKind::Script),
            "connectors": extension_names(crate::extension::ExtensionKind::Connector),
        },
    })
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
