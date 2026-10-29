//! The Room that owns a DEV installation gets its kernel development tool.
use super::*;

pub(super) fn spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: "app_dev_push".into(),
        description: "Build and push App source through the same kernel operation as /app dev. Available only in the developer Room. A failed build or declined capability expansion preserves the working generation. Source directory must exist on this kernel. Reuse the returned request_id to follow a build or shared approval; omit it for a new source revision.".into(),
        input_schema: serde_json::json!({"type":"object","properties":{"directory":{"type":"string","description":"Absolute App source directory on this kernel."},"request_id":{"type":"string","description":"Continue a prior push; omit for a new revision."}},"required":["directory"],"additionalProperties":false}),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    directory: String,
    request_id: Option<String>,
}

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch::app) fn app_dev_push_available(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<bool, DaemonError> {
        if !self.app_install_session_member(agent.session_id(), agent.owner_user_id()) {
            return Ok(false);
        }
        if self
            .app_control()
            .dev_push()
            .room_available(agent.owner_user_id(), agent.session_id())
        {
            return Ok(true);
        }
        Ok(self
            .owned
            .durable_state_store
            .active_dev_app_installations()?
            .iter()
            .any(|(owner, room, _)| owner == agent.owner_user_id() && room == agent.session_id()))
    }

    pub(super) async fn dispatch_app_dev_push(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        input: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let input: Input = serde_json::from_value(input).map_err(|_| invalid_input())?;
        let agent = self.app_agent_for_provider_run(run)?;
        let state = self.clone();
        let expected = agent.clone();
        let available =
            tokio::task::spawn_blocking(move || state.app_dev_push_available(&expected))
                .await
                .map_err(|_| unavailable())??;
        if !available {
            return Err(unavailable());
        }
        let id = input
            .request_id
            .unwrap_or_else(|| format!("dev-agent-{:016x}", rand::random::<u64>()));
        let mut response = self
            .app_control()
            .dev_push()
            .request(
                agent.owner_user_id().into(),
                agent.session_id().into(),
                id.clone(),
                Some(input.directory),
            )
            .await;
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let pending = match &response {
                crate::local::LocalDaemonResponse::AppDevPushBuilding { .. } => true,
                crate::local::LocalDaemonResponse::AppInstallOperationStatus { operation } => {
                    matches!(
                        operation.phase,
                        crate::local::AppInstallOperationPhase::Preparing
                            | crate::local::AppInstallOperationPhase::Starting
                            | crate::local::AppInstallOperationPhase::Queued
                    )
                }
                _ => false,
            };
            if !pending || tokio::time::Instant::now() >= until {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let current = self.app_agent_for_provider_run(run)?;
            if current.session_id() != agent.session_id()
                || current.owner_user_id() != agent.owner_user_id()
            {
                return Err(unavailable());
            }
            response = self
                .app_control()
                .dev_push()
                .request(
                    agent.owner_user_id().into(),
                    agent.session_id().into(),
                    id.clone(),
                    None,
                )
                .await;
        }
        let ok = !matches!(
            response,
            crate::local::LocalDaemonResponse::AppDevPushFailed { .. }
                | crate::local::LocalDaemonResponse::AppRequestFailed { .. }
        ) && !matches!(&response, crate::local::LocalDaemonResponse::AppInstallOperationStatus { operation } if matches!(operation.phase, crate::local::AppInstallOperationPhase::Failed | crate::local::AppInstallOperationPhase::Cancelled));
        Ok(RuntimeToolResult {
            ok,
            payload: serde_json::to_value(response).map_err(|_| unavailable())?,
        })
    }
}

fn invalid_input() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app_dev_push",
        message: "Use an absolute directory and an optional request_id from a previous push."
            .into(),
    }
}
