use super::*;
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
    pub(super) async fn invoke_bound_app_tool(
        &self,
        agent: &crate::agent::AgentInstance,
        tool: &RemoteExtensionTool,
        input: serde_json::Value,
        remote: Option<crate::transport::relay_peer::RemoteExtensionInvocationContext>,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if tool.kind != ExtensionKind::App {
            return Err(unavailable());
        }
        let cancelled = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        let observe = cancelled.0.clone();
        let budget =
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(move || {
                observe.load(Ordering::Acquire)
            });
        let lease = self
            .app_lease_on_demand(agent.owner_user_id(), &tool.name)
            .await?;
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
            .map_err(app_error)?;
        slot.validate_input(&tool.tool_name, &input)
            .map_err(app_error)?;
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
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
                turn_id: None,
            };
            let result = owned
                .durable_state_store
                .enqueue_app_tool(slot, &tool.tool_name, input, caller, budget)
                .map_err(app_error);
            drop(agents);
            result.map(|response| (response, expected, tool, remote))
        })
        .await
        .map_err(|_| unavailable())??;
        let (response, expected, tool, remote) = response;
        let reply = response.receive().await.map_err(app_error)?;
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let owned = self.owned.clone();
        let payload = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let agents = owned.agent_store.read();
            let current = agents.get_agent(expected.id())?;
            require_binding(&current, &expected, &tool, remote.as_ref())?;
            let result = owned
                .durable_state_store
                .accept_app_tool_reply(reply)
                .map_err(app_error);
            drop(agents);
            result
        })
        .await
        .map_err(|_| unavailable())??;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}

const ON_DEMAND_START: Duration = Duration::from_secs(20);
/// When the live-worker limit is full, a worker idle at least this long may
/// be stopped (and kept dormant) to admit an on-demand start.
const EVICTABLE_IDLE_MS: u64 = 60_000;

impl KernelRuntimeState {
    /// A dormant (idle-stopped) App starts on its next tool call. Only an App
    /// whose verified catalog is already dormant can be started this way.
    async fn app_lease_on_demand(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<crate::runtime::app_worker::AppWorkerLease, DaemonError> {
        let control = self.app_control();
        if let Some(lease) = control.active_app_lease(owner, installation) {
            return Ok(lease);
        }
        if !control.is_app_dormant(owner, installation) {
            return Err(unavailable());
        }
        let deadline = tokio::time::Instant::now() + ON_DEMAND_START;
        let mut started = false;
        let mut evicted = false;
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
                    // Transient contention (a concurrent start, preparation or
                    // admission) clears by itself: retry without evicting.
                    Err(crate::runtime::app_lifecycle::LifecycleError::Busy) => {}
                    // Every live slot is taken: make room once, then retry.
                    Err(crate::runtime::app_lifecycle::LifecycleError::LiveLimit) => {
                        if !evicted {
                            evicted = true;
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
    async fn evict_idle_app(&self, owner: &str, installation: &str) {
        let control = self.app_control().clone();
        let now = crate::session::unix_epoch_ms();
        let mut leases = control.active_app_leases(None, 16);
        let candidates: Vec<_> = leases
            .iter()
            .map(|lease| {
                (
                    lease.owner(),
                    lease.catalog().installation_id(),
                    lease.idle_ms(now),
                )
            })
            .collect();
        let Some(index) = eviction_victim(&candidates, (owner, installation)) else {
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
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || store.app_worker_status(&owner, &installation))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .is_some_and(|status| {
                status.phase == crate::durable_state::app_worker_lifecycle::WorkerPhase::Failed
            })
    }
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
/// `EVICTABLE_IDLE_MS`. Candidates are `(owner, installation, idle_ms)`.
fn eviction_victim(candidates: &[(&str, &str, u64)], target: (&str, &str)) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (owner, installation, idle))| {
            (*owner, *installation) != target && *idle >= EVICTABLE_IDLE_MS
        })
        .max_by_key(|(_, (_, _, idle))| *idle)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_picks_the_longest_idle_other_worker_past_the_threshold() {
        let old = EVICTABLE_IDLE_MS;
        let candidates = [
            ("alice", "todo", old * 5),
            ("alice", "docs", old - 1),
            ("bob", "slack", old * 2),
            ("bob", "todo", old * 3),
        ];
        // The target itself is never chosen, even when it is the idlest.
        assert_eq!(eviction_victim(&candidates, ("alice", "todo")), Some(3));
        assert_eq!(eviction_victim(&candidates, ("bob", "todo")), Some(0));
        // Nothing idle long enough: no eviction.
        assert_eq!(eviction_victim(&candidates[1..2], ("x", "y")), None);
        assert_eq!(eviction_victim(&[], ("x", "y")), None);
    }
}
