//! MP-08 / MP-09 / MP-10 / MP-11 A02: authenticated source/task admission.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome, Registration};
use crate::runtime::state::agent_inbox_sender::SenderRelation;
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
        self.dispatch_agent_event_tool_for(run.session_id(), actor, Some(run), name, args)
            .await
    }
    /// A10: a leased agent's event tools reach the home ledger, which owns
    /// its waiting state and wake ordering. The forwarded context must name
    /// the current lease, worker run and running home prompt.
    pub(crate) async fn dispatch_leased_agent_event_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Err(ledger::error("room event tools are disabled"));
        }
        if context.home_prompt_id.is_none() {
            return Err(ledger::error(
                "leased event tools need the running home prompt",
            ));
        }
        let agent = self.authorize_forwarded_workspace_context(context)?;
        self.dispatch_agent_event_tool_for(&context.home_session_id, agent.id(), None, name, args)
            .await
    }
    /// A10-S02: a message turn that interrupted a waiting task tends to reuse
    /// that task's ids to wait again. Name this turn's own ids, and say that
    /// the earlier wait continues without it.
    fn earlier_turn_origin_refusal(
        &self,
        room: &str,
        actor: &str,
        running_prompt: &str,
        origin: Option<&str>,
    ) -> Result<String, DaemonError> {
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(actor))?;
        let ids = match tasks.iter().find(|t| t.prompt_id == running_prompt) {
            Some(task) => format!(
                "This turn's task_id is `{}` and its origin_prompt_id is `{running_prompt}`.",
                task.task_id
            ),
            None => format!("This turn's origin_prompt_id is `{running_prompt}`."),
        };
        let earlier_waits = origin.is_some_and(|origin| {
            tasks.iter().any(|t| {
                t.state == ledger::ExecutionState::Waiting
                    && (t.prompt_id == origin || t.task_id == origin)
            })
        });
        Ok(if earlier_waits {
            format!("origin_prompt_id names an earlier turn whose task is still waiting; that wait continues without this turn. Do not yield on it again: finish this turn now (an answer is optional). {ids}")
        } else {
            format!("origin_prompt_id is not this running turn's; never reuse ids from an earlier turn. {ids}")
        })
    }
    async fn dispatch_agent_event_tool_for(
        &self,
        room: &str,
        actor: &str,
        run: Option<&crate::provider::RuntimeProviderRun>,
        name: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, actor)
            .ok_or_else(|| ledger::error("no running turn"))?;
        if prompt.status() != crate::session::PromptStatus::Running {
            return Err(ledger::error("stale turn authority"));
        }
        if args["origin_prompt_id"].as_str() != Some(prompt.id()) {
            return Err(ledger::error(self.earlier_turn_origin_refusal(
                room,
                actor,
                prompt.id(),
                args["origin_prompt_id"].as_str(),
            )?));
        }
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| ledger::error("task_id required"))?;
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(actor))?;
        let task = tasks
            .iter()
            .find(|t| t.task_id == task_id && t.prompt_id == prompt.id())
            .ok_or_else(|| {
                ledger::error(match tasks.iter().find(|t| t.prompt_id == prompt.id()) {
                    Some(current) => format!(
                        "task unavailable in this room and turn. This turn's task_id is `{}` and its origin_prompt_id is `{}`. Use these exact IDs to retry.",
                        current.task_id, prompt.id()
                    ),
                    None => "task unavailable in this room and turn".to_owned(),
                })
            })?;
        // MP-08/MP-09/MP-10/MP-11 F03: polling cannot keep an accepted wait
        // inside a native turn or mutate its sources/deadline before settlement.
        if task.wait.is_some() {
            return Err(ledger::error("Wait already accepted. End this provider turn immediately without further tools, including inbox, wakes or subscriptions. The kernel will deliver a new turn for the source or deadline."));
        }
        let now = crate::session::unix_epoch_ms();
        let payload = match name {
            "chariox.events.inbox" => self.read_agent_inbox(room, actor, &args)?,
            "chariox.events.subscriptions" => {
                serde_json::json!({
                    "task":task,
                    "registrations":self.owned.durable_state_store.agent_registrations(task_id)?,
                    "peer_follows":self.owned.durable_state_store.agent_peer_follows(room, actor)?,
                })
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
                self.end_peer_follow_on_other_kernel(actor, id).await;
                serde_json::json!({"unsubscribed":id})
            }
            "chariox.events.subscribe" if other_kernel(&args).is_some() => {
                let kernel = other_kernel(&args).unwrap_or_default();
                let peer_room = args["room"]
                    .as_str()
                    .map(str::trim)
                    .filter(|room| !room.is_empty())
                    .ok_or_else(|| {
                        ledger::error("room required to follow an agent on another kernel")
                    })?;
                let source = args["source_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("source_id required"))?;
                match self.resolve_peer_kernel(kernel).await? {
                    Some(kernel) => {
                        self.subscribe_peer_on_other_kernel(
                            actor,
                            task_id,
                            prompt.id(),
                            &kernel,
                            peer_room,
                            source,
                        )
                        .await?
                    }
                    None if peer_room != room => self.subscribe_peer_in_other_room(
                        &session,
                        actor,
                        task_id,
                        prompt.id(),
                        &args,
                        peer_room,
                    )?,
                    None => return Err(ledger::error(
                        "that kernel is this one; follow an agent of this room without `kernel`",
                    )),
                }
            }
            "chariox.events.subscribe" if other_room(&args, room).is_some() => self
                .subscribe_peer_in_other_room(
                    &session,
                    actor,
                    task_id,
                    prompt.id(),
                    &args,
                    other_room(&args, room).unwrap_or_default(),
                )?,
            "chariox.events.subscribe" => {
                let source = args["source_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("source_id required"))?;
                let agents = self.session_agents(room);
                let delegate = agents.iter().any(|a| a.id() == source && a.id() != actor);
                let room_tasks = self
                    .owned
                    .durable_state_store
                    .agent_tasks(Some(room), None)?;
                let peer_task = room_tasks
                    .iter()
                    .any(|t| t.task_id == source && t.agent_id != actor);
                let peer = {
                    let followed = room_tasks
                        .iter()
                        .find(|t| t.task_id == source)
                        .map_or(source, |t| t.agent_id.as_str());
                    match (
                        agents.iter().find(|a| a.id() == actor),
                        agents.iter().find(|a| a.id() == followed),
                    ) {
                        (Some(me), Some(other)) if other.id() != actor => {
                            SenderRelation::of(me, Some(other), &agents) == SenderRelation::Peer
                        }
                        _ => false,
                    }
                };
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
                            peer,
                        },
                    })?;
                serde_json::json!({"registration_id":id})
            }
            "chariox.events.ack" => self.ack_agent_inbox(room, actor, &args, now)?,
            "chariox.events.yield" => {
                let ids: Vec<String> = serde_json::from_value(args["registration_ids"].clone())
                    .map_err(|_| ledger::error("registration_ids required"))?;
                let deadline = args["deadline_ms"]
                    .as_u64()
                    .ok_or_else(|| ledger::error("finite deadline_ms required"))?;
                if deadline <= now {
                    return Err(ledger::error(format!("deadline_ms {deadline} is not in the future: it is an absolute Unix time in milliseconds, not a duration; now_ms is {now}, so a wait of N seconds is now_ms + N*1000")));
                }
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Yield {
                        task: task_id.into(),
                        prompt: prompt.id().into(),
                        registrations: ids,
                        cursor: args["inbox_cursor"]
                            .as_u64()
                            .ok_or_else(|| ledger::error("inbox_cursor required"))?,
                        deadline,
                        reason: args["reason"].as_str().unwrap_or_default().into(),
                        now,
                    })?;
                serde_json::json!({"yield_requested":true,"waiting_commits_at_native_settlement":true,"next_action":"End this provider turn immediately without further tools. Do not check inbox, wakes or subscriptions: the kernel delivers a new turn for the source or deadline."})
            }
            "chariox.events.blocked" => {
                let task = task_id.into();
                let prompt = prompt.id().into();
                let reason = args["reason"].as_str().unwrap_or_default().into();
                let operation = match args.get("owner_action") {
                    None => Operation::Block {
                        task,
                        prompt,
                        reason,
                    },
                    Some(value) if value.as_str() == Some("browser_access") => {
                        self.refuse_browser_access_card_after_start_failure(room, actor)?;
                        Operation::BlockForBrowserAccess {
                            task,
                            prompt,
                            reason,
                        }
                    }
                    _ => {
                        return Err(ledger::error(
                            "Choose browser_access for a browser approval",
                        ))
                    }
                };
                let result = self.owned.durable_state_store.agent_lifecycle(operation)?;
                if let Outcome::Task(task) = result {
                    self.ensure_task_owner_interaction(task).await?;
                }
                serde_json::json!({"blocked":true})
            }
            "chariox.events.timer"
            | "chariox.events.process"
            | "chariox.events.terminal"
            | "chariox.events.cancel_wake"
            | "chariox.events.wakes" => {
                self.dispatch_agent_wake_tool(run, name, &args, task)
                    .await?
            }
            _ => return Err(ledger::error("unknown event tool")),
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }
}

/// The kernel named by a subscribe that follows an agent on another kernel of the owner.
fn other_kernel(args: &serde_json::Value) -> Option<&str> {
    args["kernel"]
        .as_str()
        .map(str::trim)
        .filter(|kernel| !kernel.is_empty())
}

/// The room named by a subscribe that follows an agent outside the caller's room.
fn other_room<'a>(args: &'a serde_json::Value, room: &str) -> Option<&'a str> {
    args["room"]
        .as_str()
        .map(str::trim)
        .filter(|other| !other.is_empty() && *other != room)
}

impl KernelRuntimeState {
    /// A peer in another room of the owner is followed by its address there; its turns end in
    /// that room and reach this task as peer events.
    fn subscribe_peer_in_other_room(
        &self,
        session: &crate::session::RuntimeSession,
        actor: &str,
        task_id: &str,
        prompt_id: &str,
        args: &serde_json::Value,
        other: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let source = args["source_id"]
            .as_str()
            .ok_or_else(|| ledger::error("source_id required"))?;
        let caller = self.owned.agent_store.get_agent(actor)?;
        let peer = self.resolve_message_target(session, &caller, Some(other), source)?;
        let id = format!("subscription-{:032x}", rand::random::<u128>());
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Subscribe {
                task: task_id.into(),
                prompt: prompt_id.into(),
                registration: Registration {
                    id: id.clone(),
                    task_id: task_id.into(),
                    source_id: peer.id().into(),
                    obligation_id: None,
                    source_cursor: args["source_cursor"].as_u64().unwrap_or(0),
                    live: true,
                    peer: true,
                },
            })?;
        Ok(serde_json::json!({"registration_id":id,"source_id":peer.id(),"room":other}))
    }
}
