//! `chariox.agents.run`: sub-agents started and answered in one blocking call. Each worker is
//! what `agent spawn` makes, briefed with the message tool, and the call itself is the
//! one that follows it, so the end of a worker it answers is never also an inbox event.
use super::*;
use crate::runtime::state::{KernelRuntimeState, WorkerCaller, WorkerEnd};
use crate::transport::runtime_tools::{AgentsRunArgs, AgentsRunWorker, AGENTS_RUN_MAX_WORKERS};

/// A started worker the call still waits for. Dropping it (the call ends, or the caller's turn
/// is cancelled) stops the wait. An end the call took but never returned to its caller is
/// announced as `child_finished`, so the caller hears of it as of any other child.
struct WaitedWorker {
    name: String,
    agent: String,
    end: tokio::sync::oneshot::Receiver<WorkerEnd>,
    /// The end this call has taken and not yet answered or settled for the caller.
    taken: Option<WorkerEnd>,
    state: KernelRuntimeState,
}

impl Drop for WaitedWorker {
    fn drop(&mut self) {
        // The wait is released first, so an end is in the channel or goes to the event path.
        let sent = self.state.abandon_worker_wait(&self.agent, &mut self.end);
        if let Some(end) = self.taken.take().or(sent) {
            self.state.announce_undelivered_worker(end);
        }
    }
}

impl CommandRouter {
    pub(in crate::runtime::router) async fn dispatch_agents_run(
        &self,
        auth_token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.dispatch_agents_run_within(auth_token, arguments, crate::room_terminal::MAX_WAIT)
            .await
    }

    /// `max_wait` is the longest the call blocks; only tests pass another one.
    pub(in crate::runtime::router) async fn dispatch_agents_run_within(
        &self,
        auth_token: &str,
        arguments: serde_json::Value,
        max_wait: std::time::Duration,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.runtime_state.authorize_current_external_command()?;
        let (run, session, caller) = self
            .runtime_state
            .metaagent_context_for_auth_token(auth_token)?;
        let caller_turn = WorkerCaller::Local(&run);
        let mut router = self.clone();
        router.runtime_state = self.runtime_state.with_worker_caller_origin(caller_turn);
        router
            .run_agents(caller_turn, &session, &caller, arguments, max_wait)
            .await
    }

    /// A leased agent's call, forwarded by its worker kernel under the lease-verified home
    /// context: the workers join the home room and the agent's parent tree.
    pub(in crate::runtime::router) async fn dispatch_forwarded_agents_run(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.runtime_state
            .authorize_forwarded_workspace_context(&context)?;
        if context.home_prompt_id.is_none() {
            return Err(agents_run_error(
                "a leased agent's delegation needs its running home turn",
            ));
        }
        let (session, caller) = self
            .runtime_state
            .session_agent_snapshot(&context.home_session_id, &context.home_agent_id)?;
        let caller_turn = WorkerCaller::Leased(&context);
        let mut router = self.clone();
        router.runtime_state = self.runtime_state.with_worker_caller_origin(caller_turn);
        router
            .run_agents(
                caller_turn,
                &session,
                &caller,
                arguments,
                crate::room_terminal::MAX_WAIT,
            )
            .await
    }

    async fn run_agents(
        &self,
        caller_turn: WorkerCaller<'_>,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
        max_wait: std::time::Duration,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<AgentsRunArgs>(arguments)
            .map_err(|error| agents_run_error(format!("invalid tool arguments: {error}")))?;
        validate_workers(&args.workers)?;
        if session
            .active_prompt_for_agent(caller.id())
            .is_none_or(|prompt| prompt.id() != args.origin_prompt_id)
        {
            return Err(agents_run_error(
                "origin_prompt_id is not this running turn's; use the exact id from this turn's hidden context",
            ));
        }
        let mut items = Vec::with_capacity(args.workers.len());
        let mut waited = Vec::new();
        for worker in &args.workers {
            match self
                .start_worker(caller_turn, session, caller, worker, &args.origin_prompt_id)
                .await
            {
                Ok(started) => {
                    items.push(None);
                    waited.push((items.len() - 1, started));
                }
                Err(failure) => items.push(Some(failure)),
            }
        }
        // Each end stays with its worker, unanswered, until every worker is settled: a call that
        // ends before it returns the aggregate gives the ends back to the event path.
        let deadline = tokio::time::Instant::now() + max_wait;
        let mut ended = Vec::new();
        for (index, mut worker) in waited {
            match tokio::time::timeout_at(deadline, &mut worker.end).await {
                Ok(Ok(end)) => {
                    worker.taken = Some(end);
                    ended.push((index, worker));
                }
                Ok(Err(_)) => {
                    items[index] = Some(failed_item(
                        &worker.name,
                        Some(&worker.agent),
                        "the kernel stopped waiting for this worker; read its state with chariox.get_session_agent",
                    ));
                }
                Err(_) => {
                    match self
                        .hand_over_running_worker(caller_turn, &args.origin_prompt_id, &mut worker)
                        .await?
                    {
                        HandOver::Running(item) => items[index] = Some(item),
                        HandOver::Ended => ended.push((index, worker)),
                    }
                }
            }
        }
        for (index, worker) in &mut ended {
            items[*index] = Some(self.deliver(session, caller, worker)?);
        }
        let workers = items.into_iter().flatten().collect::<Vec<_>>();
        let running = workers
            .iter()
            .filter_map(|item| item["registration_id"].as_str())
            .collect::<Vec<_>>();
        let mut payload = serde_json::json!({ "workers": workers });
        if !running.is_empty() {
            payload["next_action"] = format!(
                "Some workers are still running. Finish your turn with chariox.events.yield on these registration_ids: {}. Their answers arrive as child_finished events.",
                running.join(", ")
            )
            .into();
        }
        self.runtime_state.authorize_current_external_command()?;
        Ok(RuntimeToolResult { ok: true, payload })
    }

    /// Spawns one worker as the caller's sub-agent and sends it its brief. A worker that cannot be
    /// started is that item's failure; the others still run.
    async fn start_worker(
        &self,
        caller_turn: WorkerCaller<'_>,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        worker: &AgentsRunWorker,
        origin_prompt_id: &str,
    ) -> Result<WaitedWorker, serde_json::Value> {
        let command = format!("agents.run {}", worker.name);
        let mut spawn_args = vec![worker.name.clone()];
        for (flag, value) in [("--model", &worker.model), ("--effort", &worker.effort)] {
            if let Some(value) = value {
                spawn_args.extend([flag.to_string(), value.clone()]);
            }
        }
        let spawned = Box::pin(self.dispatch_meta_agent_spawn_command(
            caller_turn.spawn_run(),
            session,
            caller,
            &command,
            &spawn_args,
        ))
        .await
        .map_err(|error| failed_item(&worker.name, None, error))?;
        if !spawned.ok {
            return Err(failed_item(
                &worker.name,
                None,
                spawned.payload["error"].as_str().unwrap_or("spawn refused"),
            ));
        }
        let agent = spawned.payload["response"]["agent"]["id"]
            .as_str()
            .ok_or_else(|| failed_item(&worker.name, None, "spawn returned no agent"))?
            .to_string();
        // Registered before the brief is sent, so a worker that answers at once is not missed.
        let waited = WaitedWorker {
            name: worker.name.clone(),
            end: self.runtime_state.wait_for_worker(&agent),
            agent,
            taken: None,
            state: self.runtime_state.clone(),
        };
        let brief = crate::transport::runtime_tools::SendAgentMessageArgs {
            agent: waited.agent.clone(),
            room: None,
            kernel: None,
            message: worker.brief.clone(),
            origin_prompt_id: origin_prompt_id.to_string(),
            urgent: false,
            reply_requested: false,
            attachments: Vec::new(),
            idempotency_key: None,
        };
        let sent = self
            .runtime_state
            .send_agent_message_as_caller(
                session,
                caller,
                serde_json::to_value(brief)
                    .map_err(|error| failed_item(&worker.name, Some(&waited.agent), error))?,
            )
            .await
            .map_err(|error| failed_item(&worker.name, Some(&waited.agent), error))?;
        if !sent.ok {
            let reason = sent.payload["error"]
                .as_str()
                .map_or_else(|| sent.payload.to_string(), str::to_string);
            return Err(failed_item(
                &worker.name,
                Some(&waited.agent),
                format!("the worker was created but its brief was not delivered: {reason}"),
            ));
        }
        Ok(waited)
    }

    /// A worker's end the call returns to its caller: the end is settled for the caller here, so
    /// no inbox event or open obligation is left for it. Only the aggregate's return makes the
    /// answer the caller's, so this runs once every worker is settled.
    fn deliver(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        worker: &mut WaitedWorker,
    ) -> Result<serde_json::Value, DaemonError> {
        let end = worker
            .taken
            .as_ref()
            .expect("a worker is delivered after its end was taken");
        let result = self.runtime_state.worker_result(&worker.name, end)?;
        self.runtime_state
            .settle_delivered_worker(session.id(), caller.id(), end)?;
        worker.taken = None;
        Ok(result)
    }

    /// The call stops waiting for a worker that is still running: the caller follows it as after
    /// `agent spawn`. The follow is made before the wait is released, so an end that lands between
    /// the two is either handed to this call or announced to the follow, never lost.
    async fn hand_over_running_worker(
        &self,
        caller_turn: WorkerCaller<'_>,
        origin_prompt_id: &str,
        worker: &mut WaitedWorker,
    ) -> Result<HandOver, DaemonError> {
        let registration = match self
            .runtime_state
            .follow_running_worker(caller_turn, origin_prompt_id, &worker.agent)
            .await
        {
            Ok(registration) => registration,
            Err(error) => {
                return Ok(HandOver::Running(serde_json::json!({
                    "name": worker.name,
                    "agent": worker.agent,
                    "status": "running",
                    "follow_refused": error.to_string(),
                })))
            }
        };
        if let Some(end) = self
            .runtime_state
            .abandon_worker_wait(&worker.agent, &mut worker.end)
        {
            worker.taken = Some(end);
            self.runtime_state
                .unfollow_worker(caller_turn, origin_prompt_id, &registration)
                .await?;
            return Ok(HandOver::Ended);
        }
        Ok(HandOver::Running(serde_json::json!({
            "name": worker.name,
            "agent": worker.agent,
            "status": "running",
            "registration_id": registration,
        })))
    }
}

/// What the call found of a worker it stopped waiting for.
enum HandOver {
    /// Still running: the caller follows it.
    Running(serde_json::Value),
    /// It ended while the call handed it over; the call answers it.
    Ended,
}

fn validate_workers(workers: &[AgentsRunWorker]) -> Result<(), DaemonError> {
    if workers.is_empty() || workers.len() > AGENTS_RUN_MAX_WORKERS {
        return Err(agents_run_error(format!(
            "give between 1 and {AGENTS_RUN_MAX_WORKERS} workers per call"
        )));
    }
    let mut names = std::collections::BTreeSet::new();
    for worker in workers {
        if worker.brief.trim().is_empty() {
            return Err(agents_run_error(format!(
                "worker `{}` has an empty brief",
                worker.name
            )));
        }
        if !names.insert(worker.name.to_lowercase()) {
            return Err(agents_run_error(format!(
                "worker name `{}` is used twice; each worker needs its own name",
                worker.name
            )));
        }
    }
    Ok(())
}

fn failed_item(
    name: &str,
    agent: Option<&str>,
    reason: impl std::fmt::Display,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "agent": agent,
        "status": "failed",
        "reason": reason.to_string(),
    })
}

fn agents_run_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_agents.run",
        message: message.into(),
    }
}
