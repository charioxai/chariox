//! MP-08 / MP-11: settle retired Meta work before restart recovery can dispatch it.
use super::DaemonApp;
use crate::error::DaemonError;

// Private durable context, never admitted from serialized user prompts. Reuse the
// existing prompt/private-state contract rather than introducing a wire shape.
const REMOTE_META_RETIREMENT_PREFIX: &str = "kernel-remote-meta-retirement:";

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RemoteMetaRetirementIntent {
    pub(crate) worker_kernel_id: String,
    pub(crate) leased_agent_id: String,
    pub(crate) execution_lease_id: String,
}

impl RemoteMetaRetirementIntent {
    pub(crate) fn is_pending(prompt: &crate::session::PromptQueueItem) -> bool {
        prompt
            .hidden_system_context()
            .starts_with(REMOTE_META_RETIREMENT_PREFIX)
    }

    pub(crate) fn read(prompt: &crate::session::PromptQueueItem) -> Result<Self, DaemonError> {
        serde_json::from_str(
            prompt
                .hidden_system_context()
                .strip_prefix(REMOTE_META_RETIREMENT_PREFIX)
                .unwrap_or_default(),
        )
        .map_err(|_| Self::error("invalid durable remote Meta retirement intent"))
    }

    fn context(binding: &crate::agent::RemoteAgentBinding) -> Result<String, DaemonError> {
        let intent = Self {
            worker_kernel_id: binding.worker_kernel_id.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            execution_lease_id: binding.execution_lease_id.clone(),
        };
        serde_json::to_string(&intent)
            .map(|json| format!("{REMOTE_META_RETIREMENT_PREFIX}{json}"))
            .map_err(|_| Self::error("could not encode remote Meta retirement intent"))
    }

    pub(crate) fn matches(&self, binding: &crate::agent::RemoteAgentBinding) -> bool {
        self.worker_kernel_id == binding.worker_kernel_id
            && self.leased_agent_id == binding.leased_agent_id
            && self.execution_lease_id == binding.execution_lease_id
    }

    pub(crate) fn error(message: &str) -> DaemonError {
        DaemonError::LocalTransport {
            operation: "retire remote Meta agent",
            message: message.into(),
        }
    }
}

const META_RETIREMENT_REASON: &str =
    "Meta tasks have been retired; use /sudo <prompt> and authorize it in the Chariox passkey popup";

impl DaemonApp {
    pub(super) fn retire_restored_meta_tasks(&self) -> Result<(), DaemonError> {
        let mut changed = false;
        for mut session in self.sessions.list_all_sessions() {
            let mut retired_agents = Vec::new();
            let mut retired_prompts = Vec::new();
            for mut agent in self.agents.get_session_agents(session.id()) {
                let was_meta = agent.is_metaagent();
                let was_controlled = agent.controlled_by_metaagent_id().is_some();
                if !was_meta && !was_controlled {
                    continue;
                }
                if was_meta {
                    // The old active turn and internal Meta notifications must
                    // not restart with instructions to call retired tools.
                    if let Some(binding) = agent.remote_execution() {
                        // Keep the exact home prompt, ACK and worker identity until
                        // cancellation AND worker Meta-mode-off are acknowledged.
                        // An idle lease also needs mode cleanup before ordinary work.
                        if self
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent.id())
                            .is_none()
                        {
                            self.prompt_state_owner.submit_prepared_prompt(
                                &session,
                                crate::session::PromptQueueItem::new(
                                    // Keep a cleanup-only identity outside the ordinary
                                    // prompt allocator, including across another restart.
                                    format!(
                                        "meta-retirement:{}:{}",
                                        agent.id(),
                                        self.sessions.reserve_prompt_id()
                                    ),
                                    crate::scheduler::runtime::workflow_prompt_source_attachment_id(
                                        "meta-retirement",
                                    ),
                                    agent.id(),
                                    META_RETIREMENT_REASON,
                                    crate::session::PromptStatus::Queued,
                                ),
                                false,
                            )?;
                        }
                        let original = self
                            .prompt_state_owner
                            .begin_cancelling_active_prompt(&session, agent.id())
                            .expect("remote retirement has an active intent");
                        let replacement = original.clone().with_hidden_system_context(
                            RemoteMetaRetirementIntent::context(binding)?,
                        );
                        self.prompt_state_owner.replace_active_prompt_if_matches(
                            &session,
                            agent.id(),
                            &original,
                            replacement,
                        );
                    } else if let Some(prompt) = self
                        .prompt_state_owner
                        .cancel_active_prompt_only(&session, agent.id())
                    {
                        retired_prompts.push(prompt.id().to_string());
                    }
                    let (_, queued) = self.prompt_state_owner.state_parts(&session, agent.id());
                    // Task edits use ordinary automation attachments. Their
                    // client attribution survives in durable private prompt
                    // state even though the attachment itself does not.
                    let task_notifier_client_id = format!("metaagent:{}:task", agent.id());
                    let injected_prompt_ids = self
                        .metaagent_events
                        .list(agent.id(), None, None, usize::MAX)
                        .into_iter()
                        .filter(|event| event.session_id == session.id())
                        .filter_map(|event| event.injected_prompt_id)
                        .collect::<std::collections::BTreeSet<_>>();
                    for prompt in queued {
                        // Turn events inherit the worker's terminal attachment.
                        // Legacy queue admission rewrites their injection IDs,
                        // but the kernel-rendered private event component is
                        // durable (even with a custom template or pruned inbox).
                        // Public user text cannot supply this private component.
                        let has_private_event_component = prompt
                            .hidden_system_context()
                            .strip_prefix("<metaagent-event>\n")
                            .and_then(|body| body.strip_suffix("\n</metaagent-event>"))
                            .is_some();
                        let is_meta_notification_source =
                            crate::scheduler::runtime::is_workflow_prompt_attachment(
                                prompt.source_attachment_id(),
                            ) || prompt.source_client_id()
                                == Some(task_notifier_client_id.as_str())
                                || injected_prompt_ids.contains(prompt.id())
                                || has_private_event_component;
                        if is_meta_notification_source && prompt.prompt()
                            == crate::scheduler::prompt_injection::METAAGENT_EVENT_VISIBLE_PROMPT
                        {
                            self.prompt_state_owner.remove_queued_prompt(
                                &session,
                                agent.id(),
                                prompt.id(),
                            );
                            retired_prompts.push(prompt.id().to_string());
                        }
                    }
                    agent.deactivate_meta_mode();
                }
                // Controlled agents retain their ordinary in-flight and queued
                // work; only the retired delegation ownership is released.
                agent.set_controlled_by_metaagent_id(None);
                retired_agents.push(agent.id().to_string());
                self.agents.restore_agent(agent);
            }
            let retired_tasks = session.retire_metaagent_tasks(META_RETIREMENT_REASON);
            if retired_agents.is_empty() && retired_tasks.is_empty() {
                continue;
            }
            changed = true;
            self.prompt_state_owner.project_into_session(&mut session);
            session.set_agents(self.agents.get_session_agents(session.id()));
            // Record cancellation IDs without task text or secret context. A
            // failed write fails bootstrap; recovery never admits old work.
            self.durable_state.append_event(
                "session.updated",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": &session,
                    "prompt_private_states": session.durable_prompt_private_states(),
                    "reason": "meta_retired",
                    "retired_task_ids": retired_tasks,
                    "retired_agent_ids": retired_agents,
                    "retired_prompt_ids": retired_prompts,
                }),
            )?;
            self.sessions.write().restore_session(session.clone());
            self.update_session_projection(session);
        }
        if changed {
            // The checkpoint commits agent modes and prompt cancellation along
            // with task settlement before listeners/restart recovery start.
            self.save_durable_state_snapshot()?;
        }
        Ok(())
    }
}
