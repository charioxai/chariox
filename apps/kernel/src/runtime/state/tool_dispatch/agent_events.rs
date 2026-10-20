//! MP-08 / MP-09 / MP-10 / MP-11 A02: authenticated source/task admission.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome};

impl KernelRuntimeState {
    pub(super) async fn dispatch_agent_event_tool(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if run.workflow_tools_enabled() {
            return Err(ledger::error("event tools are unavailable in workflow turns; finish through the workflow turn envelope"));
        }
        let actor = run
            .agent_instance_id()
            .ok_or_else(|| ledger::error("provider has no agent"))?;
        let current_run = self
            .owned
            .provider_store
            .get_run_for_agent(run.session_id(), actor)
            .ok_or_else(|| ledger::error("provider run unavailable"))?;
        if current_run.id() != run.id() {
            return Err(ledger::error("stale provider tool authority"));
        }
        self.dispatch_agent_event_tool_for(run.session_id(), actor, name, args)
            .await
    }

    async fn dispatch_agent_event_tool_for(
        &self,
        room: &str,
        actor: &str,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, actor)
            .ok_or_else(|| ledger::error("no running turn"))?;
        if prompt.workflow_run_id().is_some() {
            return Err(ledger::error("event tools are unavailable in workflow turns; finish through the workflow turn envelope"));
        }
        if prompt.status() != crate::session::PromptStatus::Running
            || args["origin_prompt_id"].as_str() != Some(prompt.id())
        {
            return Err(ledger::error("stale turn authority"));
        }
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| ledger::error("task_id required"))?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(actor))?
            .into_iter()
            .find(|t| t.task_id == task_id && t.prompt_id == prompt.id())
            .ok_or_else(|| ledger::error("task unavailable in this room and turn"))?;
        // MP-08/MP-09/MP-10/MP-11 F03: polling cannot keep an accepted wait
        // inside a native turn or mutate its sources/deadline before settlement.
        if task.wait.is_some() {
            return Err(ledger::error("Wait already accepted. End this provider turn immediately without further tools, including inbox or wakes. The kernel will deliver a new turn for the source or deadline."));
        }
        let now = crate::session::unix_epoch_ms();
        let payload = match name {
            "chariox.events.inbox" => {
                serde_json::to_value(self.owned.durable_state_store.agent_inbox(
                    room,
                    actor,
                    args["after"].as_u64().unwrap_or(0),
                )?)
                .map_err(|_| ledger::error("inbox encoding failed"))?
            }
            "chariox.events.ack" => {
                let seq = args["sequence"]
                    .as_u64()
                    .ok_or_else(|| ledger::error("sequence required"))?;
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Ack {
                        room: room.into(),
                        agent: actor.into(),
                        sequence: seq,
                        handled: args["handled"].as_bool().unwrap_or(false),
                        now,
                    })?;
                serde_json::json!({"acknowledged":seq})
            }
            "chariox.events.yield" => {
                let ids: Vec<String> = serde_json::from_value(args["registration_ids"].clone())
                    .map_err(|_| ledger::error("registration_ids required"))?;
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Yield {
                        task: task_id.into(),
                        prompt: prompt.id().into(),
                        registrations: ids,
                        cursor: args["inbox_cursor"]
                            .as_u64()
                            .ok_or_else(|| ledger::error("inbox_cursor required"))?,
                        deadline: args["deadline_ms"]
                            .as_u64()
                            .ok_or_else(|| ledger::error("finite deadline_ms required"))?,
                        reason: args["reason"].as_str().unwrap_or_default().into(),
                        now,
                    })?;
                serde_json::json!({"yield_requested":true,"waiting_commits_at_native_settlement":true,"next_action":"End this provider turn immediately without further tools. Do not check inbox or wakes: the kernel delivers a new turn for the source or deadline."})
            }
            "chariox.events.blocked" => {
                let result = self
                    .owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Block {
                        task: task_id.into(),
                        prompt: prompt.id().into(),
                        reason: args["reason"].as_str().unwrap_or_default().into(),
                    })?;
                if let Outcome::Task(task) = result {
                    self.ensure_task_owner_interaction(task).await?;
                }
                serde_json::json!({"blocked":true})
            }
            "chariox.events.timer" | "chariox.events.cancel_wake" | "chariox.events.wakes" => {
                self.dispatch_agent_wake_tool(name, &args, &task).await?
            }
            _ => return Err(ledger::error("unknown event tool")),
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }
}
