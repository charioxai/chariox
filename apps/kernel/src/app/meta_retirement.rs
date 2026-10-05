//! MP-08 / MP-11: settle retired Meta work before restart recovery can dispatch it.
use super::DaemonApp;
use crate::error::DaemonError;

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
                    if let Some(prompt) = self
                        .prompt_state_owner
                        .cancel_active_prompt_only(&session, agent.id())
                    {
                        retired_prompts.push(prompt.id().to_string());
                    }
                    let (_, queued) = self.prompt_state_owner.state_parts(&session, agent.id());
                    for prompt in queued {
                        if crate::scheduler::runtime::is_workflow_prompt_attachment(
                            prompt.source_attachment_id(),
                        ) && prompt.prompt()
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
