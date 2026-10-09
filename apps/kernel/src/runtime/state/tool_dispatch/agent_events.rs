//! MP-08 / MP-09 / MP-10 / MP-11 A02: authenticated source/task admission.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome, Registration};
impl KernelRuntimeState {
    pub(super) async fn dispatch_agent_event_tool(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Err(ledger::error("room event tools are disabled"));
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
        let session = self.owned.session_store.get_session(run.session_id())?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, actor)
            .ok_or_else(|| ledger::error("no running turn"))?;
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
            .agent_tasks(Some(run.session_id()), Some(actor))?
            .into_iter()
            .find(|t| t.task_id == task_id && t.prompt_id == prompt.id())
            .ok_or_else(|| ledger::error("task unavailable in this room and turn"))?;
        let now = crate::session::unix_epoch_ms();
        let payload = match name {
            "chariox.events.inbox" => {
                serde_json::to_value(self.owned.durable_state_store.agent_inbox(
                    run.session_id(),
                    actor,
                    args["after"].as_u64().unwrap_or(0),
                )?)
                .map_err(|_| ledger::error("inbox encoding failed"))?
            }
            "chariox.events.subscriptions" => {
                serde_json::json!({"task":task,"registrations":self.owned.durable_state_store.agent_registrations(task_id)?})
            }
            "chariox.events.unsubscribe" => {
                let id = args["registration_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("registration_id required"))?;
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Unsubscribe {
                        task: task_id.into(),
                        prompt: prompt.id().into(),
                        registration: id.into(),
                    })?;
                serde_json::json!({"unsubscribed":id})
            }
            "chariox.events.subscribe" => {
                let source = args["source_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("source_id required"))?;
                let agents = self.session_agents(run.session_id());
                let delegate = agents.iter().any(|a| a.id() == source && a.id() != actor);
                let peer_task = self
                    .owned
                    .durable_state_store
                    .agent_tasks(Some(run.session_id()), None)?
                    .iter()
                    .any(|t| t.task_id == source && t.agent_id != actor);
                let workflow = session.workflow_runs().iter().any(|w| {
                    w.id() == source
                        && !matches!(
                            w.status(),
                            crate::session::WorkflowRunStatus::Completed
                                | crate::session::WorkflowRunStatus::Failed
                                | crate::session::WorkflowRunStatus::Stopped
                        )
                });
                if !delegate && !workflow && !peer_task {
                    return Err(ledger::error(
                        "source unavailable or no longer live in this room",
                    ));
                }
                let obligation = args["obligation_id"].as_str().map(str::to_owned);
                let id = obligation
                    .as_ref()
                    .map(|id| format!("completion-{id}"))
                    .unwrap_or_else(|| format!("subscription-{:032x}", rand::random::<u128>()));
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Subscribe {
                        task: task_id.into(),
                        prompt: prompt.id().into(),
                        registration: Registration {
                            id: id.clone(),
                            task_id: task_id.into(),
                            source_id: source.into(),
                            obligation_id: obligation,
                            source_cursor: args["source_cursor"].as_u64().unwrap_or(0),
                            live: true,
                        },
                    })?;
                serde_json::json!({"registration_id":id})
            }
            "chariox.events.ack" => {
                let seq = args["sequence"]
                    .as_u64()
                    .ok_or_else(|| ledger::error("sequence required"))?;
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Ack {
                        room: run.session_id().into(),
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
                serde_json::json!({"yield_requested":true,"waiting_commits_at_native_settlement":true})
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
            "chariox.events.timer"
            | "chariox.events.process"
            | "chariox.events.cancel_wake"
            | "chariox.events.wakes" => {
                self.dispatch_agent_wake_tool(run, name, &args, &task)
                    .await?
            }
            _ => return Err(ledger::error("unknown event tool")),
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }
}
