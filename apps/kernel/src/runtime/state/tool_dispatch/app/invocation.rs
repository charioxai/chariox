use super::*;
use crate::runtime::app_call_errors;
use chariox_app_runtime::app_catalog::{Actor, CallerContext};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl KernelRuntimeState {
    /// `caller_run_id` is the provider run that made the call: the local run,
    /// or a leased agent's worker run.
    pub(super) async fn invoke_bound_app_tool(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_run_id: &str,
        tool: &RemoteExtensionTool,
        input: serde_json::Value,
        remote: Option<crate::transport::relay_peer::RemoteExtensionInvocationContext>,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if tool.kind != ExtensionKind::App {
            return Err(unavailable());
        }
        // Captured at submission, before any wait for an on-demand start.
        let turn_id = self.app_call_turn_id(agent, caller_run_id, remote.is_some());
        let cancelled = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        let observe = cancelled.0.clone();
        let budget =
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(move || {
                observe.load(Ordering::Acquire)
            });
        let lease = match self
            .app_lease_on_demand(agent.owner_user_id(), &tool.name)
            .await
        {
            Ok(lease) => lease,
            Err(_) if self.app_updating(agent.owner_user_id(), &tool.name).await => {
                return Err(coded(app_call_errors::updating()));
            }
            Err(error) => return Err(error),
        };
        let expected_version = format!(
            "{}:{}",
            lease.catalog().generation(),
            lease.catalog().app_catalog().catalog_digest()
        );
        if tool.version_hash.as_deref() != Some(expected_version.as_str()) {
            return Err(unavailable());
        }
        let slot = lease
            .reserve_call(Duration::from_secs(30))
            .map_err(|error| coded(app_call_errors::busy_error(&error)))?;
        slot.validate_input(&tool.tool_name, &input)
            .map_err(|error| coded(app_call_errors::input_error(&error)))?;
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|_| admission_busy())?;
        let owned = self.owned.clone();
        let expected = agent.clone();
        let tool = tool.clone();
        let response = tokio::task::spawn_blocking(move || {
            // The blocking closure retains admission even if the awaiting MCP
            // connection closes. No guard is held while awaiting App execution.
            let _permit = permit;
            let agents = owned.agent_store.read();
            let current = agents.get_agent(expected.id())?;
            require_binding(&current, &expected, &tool, remote.as_ref())?;
            let caller = CallerContext {
                actor: Actor::Agent(current.id().into()),
                room_id: current.session_id().into(),
                operation_id: format!("app-operation-{:016x}", rand::random::<u64>()),
                task_id: None,
                turn_id,
            };
            let result = owned
                .durable_state_store
                .enqueue_app_tool(slot, &tool.tool_name, input, caller, budget)
                .map_err(|error| coded(app_call_errors::enqueue_error(&error)));
            drop(agents);
            result.map(|response| (response, expected, tool, remote))
        })
        .await
        .map_err(|_| unavailable())??;
        let (response, expected, tool, remote) = response;
        let reply = response.receive().await.map_err(worker_call_error)?;
        let permit = self
            .app_control()
            .admit_reply(reply.remaining(crate::session::unix_epoch_ms()))
            .await
            .map_err(|_| coded(app_call_errors::reply_unrecorded()))?;
        let owned = self.owned.clone();
        let payload = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let agents = owned.agent_store.read();
            let current = agents.get_agent(expected.id())?;
            require_binding(&current, &expected, &tool, remote.as_ref())?;
            let result = owned
                .durable_state_store
                .accept_app_tool_reply(reply)
                .map_err(tool_call_error);
            drop(agents);
            result
        })
        .await
        .map_err(|_| unavailable())??;
        Ok(RuntimeToolResult { ok: true, payload })
    }

    /// Whether a call that found no running worker met the App's approved
    /// update in progress, rather than a stopped or failed App.
    pub(crate) async fn app_updating(&self, owner: &str, installation: &str) -> bool {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || store.app_update_underway(&owner, &installation))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }
}

const ON_DEMAND_START: Duration = Duration::from_secs(20);
/// When the live-worker limit is full, a worker idle at least this long may
/// be stopped (and kept dormant) to admit an on-demand start.
const EVICTABLE_IDLE_MS: u64 = 60_000;
/// A call still waiting at the live limit tries another eviction this often.
const EVICTION_RETRY: Duration = Duration::from_secs(1);

impl KernelRuntimeState {
    /// A dormant (idle-stopped) App starts on its next tool call, and so does
    /// one that should be running but has no worker (its kernel restarted, or
    /// recovery could not start it yet, e.g. every live slot was taken). A
    /// user stop or a failed worker is never started this way.
    pub(crate) async fn app_lease_on_demand(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<crate::runtime::app_worker::AppWorkerLease, DaemonError> {
        let control = self.app_control();
        if let Some(lease) = control.active_app_lease(owner, installation) {
            return Ok(lease);
        }
        let deadline = tokio::time::Instant::now() + ON_DEMAND_START;
        if !control.is_app_dormant(owner, installation) {
            use crate::durable_state::app_worker_lifecycle::WorkerPhase;
            // A start already under way (a user start, a restart after a
            // crash) is waited for.
            while self
                .app_worker_status(owner, installation)
                .await
                .is_some_and(|status| status.phase == WorkerPhase::Starting)
                && tokio::time::Instant::now() < deadline
            {
                if let Some(lease) = control.active_app_lease(owner, installation) {
                    return Ok(lease);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if let Some(lease) = control.active_app_lease(owner, installation) {
                return Ok(lease);
            }
            let status = self.app_worker_status(owner, installation).await;
            if !starts_on_demand(status.as_ref()) {
                return Err(unavailable());
            }
        }
        let mut started = false;
        let mut next_eviction = tokio::time::Instant::now();
        loop {
            if let Some(lease) = control.active_app_lease(owner, installation) {
                return Ok(lease);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(unavailable());
            }
            if !started {
                let lifecycle = control.lifecycle().clone();
                let (start_owner, start_installation) = (owner.to_owned(), installation.to_owned());
                let handle = tokio::runtime::Handle::current();
                match tokio::task::spawn_blocking(move || {
                    lifecycle.start_on_demand_blocking(&start_owner, &start_installation, handle)
                })
                .await
                .map_err(|_| unavailable())?
                {
                    Ok(_) => started = true,
                    // Transient contention (a concurrent operation on this
                    // installation) clears by itself: retry without evicting.
                    Err(crate::runtime::app_lifecycle::LifecycleError::Busy) => {}
                    // Every live slot is taken: make room, then retry. A
                    // concurrent call may take the freed slot or evict the
                    // same idle worker first, so try again while the limit
                    // holds, at most once per EVICTION_RETRY.
                    Err(crate::runtime::app_lifecycle::LifecycleError::LiveLimit) => {
                        if tokio::time::Instant::now() >= next_eviction {
                            next_eviction = tokio::time::Instant::now() + EVICTION_RETRY;
                            self.evict_idle_app(owner, installation).await;
                        }
                    }
                    Err(_) => {
                        control.forget_app_dormant(owner, installation);
                        return Err(unavailable());
                    }
                }
            } else if self.app_start_failed(owner, installation).await {
                control.forget_app_dormant(owner, installation);
                return Err(unavailable());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Stop the least-recently-used idle worker (other than the target),
    /// keeping it dormant, so an on-demand start can take its live slot.
    pub(crate) async fn evict_idle_app(&self, owner: &str, installation: &str) {
        let control = self.app_control().clone();
        let now = crate::session::unix_epoch_ms();
        let mut leases = control.active_app_leases(None, 16);
        let candidates: Vec<_> = leases
            .iter()
            .map(|lease| {
                (
                    lease.owner().to_owned(),
                    lease.catalog().installation_id().to_owned(),
                    lease.idle_ms(now),
                )
            })
            .collect();
        let store = self.owned.durable_state_store.clone();
        let target = (owner.to_owned(), installation.to_owned());
        // Workers with undelivered events are not idle; rank only the others,
        // so one busy App cannot block eviction of an idle one.
        let Ok(Some(index)) = tokio::task::spawn_blocking(move || {
            let candidates: Vec<_> = candidates
                .iter()
                .map(|(owner, installation, idle)| (owner.as_str(), installation.as_str(), *idle))
                .collect();
            eviction_victim(
                &candidates,
                (&target.0, &target.1),
                |owner, installation| store.has_deliverable_app_events(owner, installation),
            )
        })
        .await
        else {
            return;
        };
        let lease = leases.swap_remove(index);
        drop(leases);
        let lifecycle = control.lifecycle().clone();
        let store = self.owned.durable_state_store.clone();
        let victim_owner = lease.owner().to_owned();
        let catalog = lease.catalog().clone();
        drop(lease);
        let _ = tokio::task::spawn_blocking(move || {
            let installation = catalog.installation_id().to_owned();
            lifecycle.idle_stop_blocking(&victim_owner, catalog, || {
                control
                    .active_app_lease(&victim_owner, &installation)
                    .is_some_and(|lease| {
                        lease.idle_ms(crate::session::unix_epoch_ms()) >= EVICTABLE_IDLE_MS
                    })
                    && !store.has_deliverable_app_events(&victim_owner, &installation)
            })
        })
        .await;
    }

    async fn app_start_failed(&self, owner: &str, installation: &str) -> bool {
        self.app_worker_status(owner, installation)
            .await
            .is_some_and(|status| {
                status.phase == crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
            })
    }

    async fn app_worker_status(
        &self,
        owner: &str,
        installation: &str,
    ) -> Option<crate::durable_state::app_worker_lifecycle::WorkerStatus> {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || store.app_worker_status(&owner, &installation))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
    }
}

/// A worker that should be running but has none starts on demand; a user
/// stop (not desired) or a failed worker (restart backoff, quarantine) never.
fn starts_on_demand(
    status: Option<&crate::durable_state::app_worker_lifecycle::WorkerStatus>,
) -> bool {
    status.is_some_and(|status| {
        status.desired_running
            && status.phase != crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
    })
}

impl KernelRuntimeState {
    /// The agent's turn (its active Chariox prompt) when it makes the call. It
    /// is captured once at submission: a later turn or focus change never
    /// re-attributes an accepted call.
    fn app_call_turn_id(
        &self,
        agent: &crate::agent::AgentInstance,
        caller_run_id: &str,
        relayed: bool,
    ) -> Option<String> {
        let session = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok()?;
        call_turn_id(
            self.owned
                .prompt_state_owner
                .active_prompt_for_agent_snapshot(&session, agent.id()),
            caller_run_id,
            relayed,
        )
    }
}

/// Only a turn the calling run has received is the calling turn: no turn beats
/// a wrong one. `relayed` is a leased agent's call from its worker run. An id
/// the App context cannot carry is left out rather than failing the call.
fn call_turn_id(
    active: Option<crate::session::PromptQueueItem>,
    caller_run_id: &str,
    relayed: bool,
) -> Option<String> {
    use crate::session::DurablePromptDeliveryPhase::{Accepted, Delivered, Dispatching};
    active
        .filter(|prompt| match prompt.durable_delivery_phase() {
            // Dispatched by the kernel: once delivered, to the calling run.
            Some(Delivered) => prompt
                .durable_delivery_provider_run_id()
                .is_none_or(|run| run == caller_run_id),
            // Still on its way: the provider is between turns.
            Some(Dispatching) => false,
            // Recorded but never dispatched: a turn the provider started itself
            // (typed in its native TUI, always local) stays here for the whole
            // turn. A local kernel prompt passes through it only until its
            // dispatch starts; a leased agent's home prompt stays here until
            // the worker has it, so a relayed call needs Delivered.
            Some(Accepted) | None => {
                !relayed
                    && prompt.status() == crate::session::PromptStatus::Running
                    && prompt.durable_delivery_provider_run_id().is_none()
            }
        })
        .map(|prompt| prompt.id().to_string())
        .filter(|id| CallerContext::valid_id(id))
}

fn require_binding(
    current: &crate::agent::AgentInstance,
    expected: &crate::agent::AgentInstance,
    tool: &RemoteExtensionTool,
    remote: Option<&crate::transport::relay_peer::RemoteExtensionInvocationContext>,
) -> Result<(), DaemonError> {
    if current.owner_user_id() != expected.owner_user_id()
        || current.session_id() != expected.session_id()
        || !current.has_extension_grant(ExtensionKind::App, &tool.name)
    {
        return Err(unavailable());
    }
    if let Some(context) = remote {
        let binding = current.remote_execution().ok_or_else(unavailable)?;
        if current.id() != context.home_agent_id
            || current.session_id() != context.home_session_id
            || binding.leased_agent_id != context.leased_agent_id
            || context.worker_kernel_id.as_deref() != Some(binding.worker_kernel_id.as_str())
            || binding.active_worker_provider_run_id.as_deref()
                != Some(context.worker_provider_run_id.as_str())
        {
            return Err(unavailable());
        }
    }
    Ok(())
}

/// The longest-idle worker other than `target`, if idle for at least
/// `EVICTABLE_IDLE_MS` and not `busy`. Candidates are `(owner, installation,
/// idle_ms)`; `busy` is checked only for otherwise eligible workers.
fn eviction_victim(
    candidates: &[(&str, &str, u64)],
    target: (&str, &str),
    busy: impl Fn(&str, &str) -> bool,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (owner, installation, idle))| {
            (*owner, *installation) != target
                && *idle >= EVICTABLE_IDLE_MS
                && !busy(owner, installation)
        })
        .max_by_key(|(_, (_, _, idle))| *idle)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_names_only_the_turn_its_own_run_is_serving() {
        use crate::session::DurablePromptDeliveryPhase::{Accepted, Delivered, Dispatching};
        let prompt = |id: &str,
                      delivery: Option<(
            crate::session::DurablePromptDeliveryPhase,
            Option<&str>,
        )>| {
            let mut prompt = crate::session::PromptQueueItem::new(
                id,
                "attachment-1",
                "agent-1",
                "use the App",
                crate::session::PromptStatus::Running,
            );
            if let Some((phase, run)) = delivery {
                prompt.set_durable_delivery(phase, run.map(str::to_string), None);
            }
            prompt
        };
        // Delivered to the calling run (a local run, or a leased agent's
        // worker run): that turn.
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt-1", Some((Delivered, Some("run-a"))))),
                "run-a",
                true
            )
            .as_deref(),
            Some("prompt-1")
        );
        // A turn typed in the provider's native TUI is recorded, never
        // dispatched: the prompt owner leaves it Accepted with no run.
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt-1", Some((Accepted, None)))),
                "run-a",
                false
            )
            .as_deref(),
            Some("prompt-1")
        );
        // A leased agent's home prompt is Accepted until the worker has it.
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt-2", Some((Accepted, None)))),
                "run-a",
                true
            ),
            None
        );
        // Still being dispatched (even to the same run), another run's turn,
        // no turn, or an id the context cannot carry: none.
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt-2", Some((Dispatching, Some("run-a"))))),
                "run-a",
                false
            ),
            None
        );
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt-2", Some((Delivered, Some("run-b"))))),
                "run-a",
                false
            ),
            None
        );
        assert_eq!(call_turn_id(None, "run-a", false), None);
        assert_eq!(
            call_turn_id(
                Some(prompt("prompt 3", Some((Accepted, None)))),
                "run-a",
                false
            ),
            None
        );
    }

    #[test]
    fn eviction_picks_the_longest_idle_other_worker_past_the_threshold() {
        let old = EVICTABLE_IDLE_MS;
        let candidates = [
            ("alice", "todo", old * 5),
            ("alice", "docs", old - 1),
            ("bob", "slack", old * 2),
            ("bob", "todo", old * 3),
        ];
        let idle = |_: &str, _: &str| false;
        // The target itself is never chosen, even when it is the idlest.
        assert_eq!(
            eviction_victim(&candidates, ("alice", "todo"), idle),
            Some(3)
        );
        assert_eq!(eviction_victim(&candidates, ("bob", "todo"), idle), Some(0));
        // Nothing idle long enough: no eviction.
        assert_eq!(eviction_victim(&candidates[1..2], ("x", "y"), idle), None);
        assert_eq!(eviction_victim(&[], ("x", "y"), idle), None);
        // A busy (undelivered events) idlest worker is skipped for the next one.
        let busy = |owner: &str, installation: &str| (owner, installation) == ("alice", "todo");
        assert_eq!(eviction_victim(&candidates, ("bob", "todo"), busy), Some(2));
    }
}

#[cfg(test)]
mod on_demand_tests {
    use super::starts_on_demand;
    use crate::durable_state::app_worker_lifecycle::{WorkerPhase, WorkerStatus};

    fn status(phase: WorkerPhase, desired_running: bool) -> WorkerStatus {
        WorkerStatus {
            generation: 1,
            attempt: "attempt".into(),
            phase,
            desired_running,
            failure: None,
            updated_ms: 1,
            failures: 0,
        }
    }

    #[test]
    fn a_worker_that_should_run_starts_on_demand_but_a_user_stop_or_failure_never_does() {
        // Stopped by a kernel restart (or never started by recovery): start it.
        assert!(starts_on_demand(Some(&status(WorkerPhase::Stopped, true))));
        // A start that never completed, after the caller waited for it.
        assert!(starts_on_demand(Some(&status(WorkerPhase::Starting, true))));
        assert!(!starts_on_demand(Some(&status(
            WorkerPhase::Stopped,
            false
        ))));
        assert!(!starts_on_demand(Some(&status(WorkerPhase::Failed, true))));
        assert!(!starts_on_demand(None));
    }
}
