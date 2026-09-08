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
            .app_control()
            .active_app_lease(agent.owner_user_id(), &tool.name)
            .ok_or_else(unavailable)?;
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
