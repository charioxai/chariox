use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::Mutex;

use crate::app::DaemonApp;
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

struct RuntimeStateNativeInteractionBridge {
    handle: Handle,
    app: Arc<Mutex<DaemonApp>>,
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
        let remote_target = self.handle.block_on(async {
            let mut app = self.app.lock().await;
            let target = crate::app::RemoteLeaseRuntime::new(&mut app)
                .native_interaction_context_for_backing_agent(
                    &session_id,
                    &interaction_agent_id,
                    "unknown",
                );
            Ok::<_, DaemonError>(
                target.map(|(daemon_id, context)| (app.config().clone(), daemon_id, context)),
            )
        })?;
        if let Some((config, target_daemon_id, context)) = remote_target {
            let response = self.handle.block_on(async move {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    arroba_relay::protocol::ClientTarget {
                        daemon_id: Some(target_daemon_id),
                        daemon_alias: None,
                    },
                    crate::transport::relay_peer::RelayPeerRequest::ForwardNativeInteraction {
                        context,
                        interaction,
                    },
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

pub(crate) fn install_provider_native_interaction_bridge(
    app: Arc<Mutex<DaemonApp>>,
    state: KernelRuntimeState,
    provider_store: &ProviderProcessServiceStore,
) {
    if let Ok(handle) = Handle::try_current() {
        provider_store.set_native_interaction_bridge(Arc::new(
            RuntimeStateNativeInteractionBridge { handle, app, state },
        ));
    }
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
