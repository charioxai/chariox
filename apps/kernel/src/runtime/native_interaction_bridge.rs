use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, NativeProviderInteractionResolution,
    RequestNativeProviderInteractionRequest,
};
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderProcessServiceStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{RuntimeInteraction, RuntimeInteractionKind};
use crate::transport::relay_peer::RemoteNativeInteractionContext;

const REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER: Duration = Duration::from_secs(15);

struct RuntimeStateNativeInteractionBridge {
    handle: Handle,
    state: KernelRuntimeState,
}

impl ProviderNativeInteractionBridge for RuntimeStateNativeInteractionBridge {
    fn request_blocking(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        let session_id = session_id.to_string();
        let interaction_agent_id = interaction.agent_id().to_string();
        let state = self.state.clone();
        let remote_target = self.handle.block_on(async {
            state
                .remote_native_interaction_context(&session_id, &interaction_agent_id)
                .await
        })?;
        if let Some((config, target_daemon_id, context)) = remote_target {
            let response_timeout = remote_native_interaction_response_timeout(
                &interaction,
                config.relay_request_timeout_ms,
            );
            let response = self.handle.block_on(async move {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &config,
                    chariox_relay::protocol::ClientTarget {
                        daemon_id: Some(target_daemon_id),
                        daemon_alias: None,
                    },
                    crate::transport::relay_peer::RelayPeerRequest::ForwardNativeInteraction {
                        context,
                        interaction,
                    },
                    response_timeout,
                )
                .await
            })?;
            return match response {
                crate::transport::relay_peer::RelayPeerResponse::NativeInteractionResolved {
                    resolution,
                } => Ok(resolution),
                other => Err(DaemonError::LocalTransport {
                    operation: "provider_native_interaction_bridge",
                    message: format!(
                        "unexpected relay response for remote native interaction: {other:?}"
                    ),
                }),
            };
        }
        let state = self.state.clone();
        let resolution = self.handle.block_on(async move {
            let receiver = state
                .create_runtime_interaction(&session_id, interaction)
                .await?;
            receiver.await.map_err(|error| DaemonError::LocalTransport {
                operation: "provider_native_interaction_bridge",
                message: format!("interaction dropped before resolution: {error}"),
            })
        })?;
        Ok(ProviderNativeInteractionResolution {
            status: resolution.status.to_string(),
            choice_id: resolution.choice_id,
            reply: resolution.reply,
        })
    }
}

fn remote_native_interaction_response_timeout(
    interaction: &RuntimeInteraction,
    relay_request_timeout_ms: u64,
) -> Duration {
    interaction
        .timeout_sec()
        .map(Duration::from_secs)
        .map(|timeout| timeout.saturating_add(REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER))
        .unwrap_or_else(|| Duration::from_millis(relay_request_timeout_ms))
}

pub(crate) fn install_provider_native_interaction_bridge(
    state: KernelRuntimeState,
    provider_store: &ProviderProcessServiceStore,
) -> Option<Arc<dyn ProviderNativeInteractionBridge>> {
    let handle = Handle::try_current().ok()?;
    let bridge: Arc<dyn ProviderNativeInteractionBridge> =
        Arc::new(RuntimeStateNativeInteractionBridge { handle, state });
    // The router owns this bridge. A strong reference in the provider store
    // would retain the runtime, which itself owns that same provider store.
    provider_store.set_native_interaction_bridge(bridge.clone());
    Some(bridge)
}

pub(crate) async fn execute_native_provider_interaction_request(
    state: &KernelRuntimeState,
    request: RequestNativeProviderInteractionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let session_id = request.session_id.clone();
    let interaction = RuntimeInteraction::new(
        request.interaction_id,
        request.agent_id,
        RuntimeInteractionKind::Permission,
        request.level,
        request.title,
        request.message,
        request.choices,
        request.custom_choice,
        request.timeout_sec,
        request.default_on_timeout,
    );
    let resolution = request_runtime_interaction_with_timeout(
        state,
        &session_id,
        interaction,
        "native_provider_interaction_request",
    )
    .await?;
    Ok(LocalDaemonResponse::NativeProviderInteractionResolved {
        resolution: NativeProviderInteractionResolution {
            status: resolution.status,
            choice_id: resolution.choice_id,
            reply: resolution.reply,
        },
    })
}

pub(crate) async fn forward_relay_native_interaction(
    state: &KernelRuntimeState,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let interaction = interaction.with_agent_id(context.home_agent_id.clone());
    request_runtime_interaction_with_timeout(
        state,
        &context.home_session_id,
        interaction,
        "relay_forward_native_interaction",
    )
    .await
}

async fn request_runtime_interaction_with_timeout(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let timeout = interaction.timeout_sec().map(Duration::from_secs);
    let timeout_session_id = session_id.to_string();
    let timeout_interaction_id = interaction.id().to_string();
    let receiver = state
        .create_runtime_interaction(session_id, interaction)
        .await?;
    if let Some(timeout) = timeout {
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let _ = state
                .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                .await;
        });
    }
    let resolution = receiver
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("interaction dropped before resolution: {error}"),
        })?;
    Ok(ProviderNativeInteractionResolution {
        status: resolution.status.to_string(),
        choice_id: resolution.choice_id,
        reply: resolution.reply,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{RuntimeInteractionChoice, RuntimeInteractionLevel};

    fn interaction(timeout_sec: Option<u64>) -> RuntimeInteraction {
        RuntimeInteraction::new(
            "interaction-1",
            "agent-1",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            None,
            "Approve?",
            Vec::<RuntimeInteractionChoice>::new(),
            None,
            timeout_sec,
            None,
        )
    }

    #[test]
    fn remote_response_timeout_outlives_the_user_interaction_deadline() {
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(Some(300)), 60_000),
            Duration::from_secs(315)
        );
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(Some(5)), 60_000),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn remote_response_timeout_uses_relay_default_without_interaction_deadline() {
        assert_eq!(
            remote_native_interaction_response_timeout(&interaction(None), 42_000),
            Duration::from_secs(42)
        );
    }

    #[tokio::test]
    async fn native_interaction_bridge_releases_runtime_after_last_router_drops() {
        let config = crate::config::DaemonConfig::for_tests();
        let app = Arc::new(tokio::sync::Mutex::new(
            crate::app::DaemonApp::bootstrap(config.clone()).unwrap(),
        ));
        let weak_app = Arc::downgrade(&app);
        let router =
            crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 1);
        let clone = router.clone();
        assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
        drop(router);
        assert!(weak_app.upgrade().is_some());
        assert!(crate::app::DaemonApp::bootstrap(config.clone()).is_err());
        drop(clone);
        assert!(
            weak_app.upgrade().is_none(),
            "permission bridge must not retain its own runtime"
        );
        assert!(crate::app::DaemonApp::bootstrap(config).is_ok());
    }
}
