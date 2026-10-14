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
        let turn_cancelled = Arc::new(self.app_call_turn_cancellation(agent));
        let observe_turn = turn_cancelled.clone();
        let cancelled = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        // MP-08/MP-11: every wait observes the captured binding generation,
        // including startup and reply admission, not just App execution.
        let authority_lost = || async {
            while !turn_cancelled()
                && self
                    .owned
                    .agent_store
                    .get_agent(agent.id())
                    .is_ok_and(|current| same_binding(&current, agent, &tool.name))
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let lease_result = tokio::select! {
            biased;
            _ = authority_lost() => return Err(unavailable()),
            result = self.app_lease_on_demand(agent.owner_user_id(), &tool.name) => result,
        };
        let lease = match lease_result {
            Ok(lease) => lease,
            Err(_) if self.app_updating(agent.owner_user_id(), &tool.name).await => {
                return Err(coded(app_call_errors::updating()));
            }
            Err(error) => return Err(error),
        };
        // Readiness has its own budget; execution starts its budget afterward.
        let observe = cancelled.0.clone();
        let budget =
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(move || {
                observe.load(Ordering::Acquire) || observe_turn()
            });
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
                room_id: Some(current.session_id().into()),
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
        // The MCP connection can remain open after its turn is cancelled.
        // Dropping the response future asks the existing worker peer to abort
        // the handler, preserving that peer's cancellation/receipt ownership.
        // MP-08/MP-11: revoking the binding wakes and fails the waiting call too.
        let reply = tokio::select! {
            biased;
            _ = authority_lost() => return Err(unavailable()),
            reply = response.receive() => reply.map_err(worker_call_error)?,
        };
        let permit = tokio::select! {
            biased;
            _ = authority_lost() => return Err(unavailable()),
            permit = self.app_control().admit_reply(reply.remaining(crate::session::unix_epoch_ms())) => {
                permit.map_err(|_| coded(app_call_errors::reply_unrecorded()))?
            }
        };
        let owned = self.owned.clone();
        let payload = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if turn_cancelled() {
                return Err(unavailable());
            }
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

    fn app_call_turn_cancellation(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> impl Fn() -> bool + Send + Sync + 'static {
        let prompts = self.owned.prompt_state_owner.clone();
        let agent_id = agent.id().to_owned();
        let turn = self
            .owned
            .session_store
            .get_session(agent.session_id())
            .ok()
            .and_then(|session| {
                prompts
                    .active_prompt_for_agent_snapshot(&session, &agent_id)
                    .map(|prompt| (session, prompt.id().to_owned()))
            });
        // Bind once, before any await. A later turn or focus change cannot
        // retarget accepted work, and calls between turns keep their old path.
        move || {
            turn.as_ref().is_some_and(|(session, turn_id)| {
                prompts
                    .active_prompt_for_agent_snapshot(session, &agent_id)
                    .is_none_or(|prompt| {
                        prompt.id() != turn_id
                            || !matches!(
                                prompt.status(),
                                crate::session::PromptStatus::Dispatching
                                    | crate::session::PromptStatus::Running
                            )
                    })
            })
        }
    }
}

impl KernelRuntimeState {
    pub(crate) async fn app_lease_on_demand(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<crate::runtime::app_worker::AppWorkerLease, DaemonError> {
        crate::runtime::app_on_demand::app_lease_on_demand(
            self.app_control(),
            &self.owned.durable_state_store,
            owner,
            installation,
        )
        .await
    }
    pub(crate) async fn evict_idle_app(&self, owner: &str, installation: &str) {
        crate::runtime::app_on_demand::evict_idle_app(
            self.app_control(),
            &self.owned.durable_state_store,
            owner,
            installation,
        )
        .await;
    }
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
        || !same_binding(current, expected, &tool.name)
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

// MP-11: revoke/regrant cannot revive a retained call from the old grant.
fn same_binding(
    current: &crate::agent::AgentInstance,
    expected: &crate::agent::AgentInstance,
    name: &str,
) -> bool {
    let grant = |agent: &crate::agent::AgentInstance| {
        agent
            .extension_grants()
            .iter()
            .find(|grant| grant.kind == ExtensionKind::App && grant.name == name)
            .map(|grant| grant.app_grant.clone())
    };
    current.has_extension_grant(ExtensionKind::App, name)
        && current.remote_execution() == expected.remote_execution()
        && grant(current) == grant(expected)
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
}
