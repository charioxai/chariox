use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, NativeProviderInteractionResolution,
    RequestNativeProviderTurnInteractionRequest,
};
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderProcessServiceStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::{RuntimeInteraction, RuntimeInteractionKind};
use crate::transport::relay_peer::RemoteNativeInteractionContext;

pub(crate) const REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER: Duration = Duration::from_secs(15);
// A Claude permission hook outwaits a forwarded interaction's timeout and relay
// buffer, with room for event pickup and the relay connection, so the deny it
// resolves to still reaches Claude.
const _: () = assert!(
    crate::provider::CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS
        >= crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS
            + REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER.as_secs()
            + 15
);

struct RuntimeStateNativeInteractionBridge {
    handle: Handle,
    state: KernelRuntimeState,
}

impl ProviderNativeInteractionBridge for RuntimeStateNativeInteractionBridge {
    fn capture_turn_origin(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Option<crate::session::NativeInteractionOrigin> {
        self.state
            .capture_native_interaction_origin(session_id, agent_id, provider_run_id)
    }

    fn request_blocking(
        &self,
        session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        let Some(origin) = interaction.native_origin().cloned() else {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            });
        };
        let session_id = session_id.to_string();
        let interaction_agent_id = interaction
            .agent_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "provider_native_interaction_bridge",
                message: "Provider interactions require an agent subject".into(),
            })?
            .to_string();
        let state = self.state.clone();
        let remote_target = self.handle.block_on(async {
            state
                .remote_native_interaction_context(&session_id, &interaction_agent_id, &origin)
                .await
        })?;
        if let Some((config, target_daemon_id, context)) = remote_target {
            return self.handle.block_on(forward_native_interaction_to_home(
                &state,
                config,
                target_daemon_id,
                context,
                interaction,
            ));
        }
        let state = self.state.clone();
        self.handle
            .block_on(request_runtime_interaction_with_timeout(
                &state,
                &session_id,
                interaction,
                "provider_native_interaction_bridge",
                None,
            ))
    }
}

async fn forward_native_interaction_to_home(
    state: &KernelRuntimeState,
    config: crate::config::DaemonConfig,
    target_daemon_id: String,
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let response_timeout =
        remote_native_interaction_response_timeout(&interaction, config.relay_request_timeout_ms);
    let response = state
        .send_worker_home_runtime_request(
            &config,
            &target_daemon_id,
            crate::transport::relay_peer::RelayPeerRequest::ForwardNativeTurnInteraction {
                context,
                interaction,
            },
            response_timeout,
        )
        .await?;
    match response {
        crate::transport::relay_peer::RelayPeerResponse::NativeInteractionResolved {
            resolution,
        } => Ok(resolution),
        other => Err(DaemonError::LocalTransport {
            operation: "provider_native_interaction_bridge",
            message: format!("unexpected relay response for remote native interaction: {other:?}"),
        }),
    }
}

/// Async counterpart of the provider bridge for runtime MCP tools: forwards a
/// leased worker agent's interaction to its home kernel, otherwise creates it
/// here. Dropping the future (the provider abandoned the call) closes a local
/// interaction through its timeout path.
pub(crate) async fn request_provider_native_interaction(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    let agent_id = interaction
        .agent_id()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation,
            message: "Provider interactions require an agent subject".into(),
        })?
        .to_string();
    let Some(origin) = interaction.native_origin().cloned() else {
        return Ok(ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        });
    };
    if let Some((config, target_daemon_id, context)) = state
        .remote_native_interaction_context(session_id, &agent_id, &origin)
        .await?
    {
        return forward_native_interaction_to_home(
            state,
            config,
            target_daemon_id,
            context,
            interaction,
        )
        .await;
    }
    request_runtime_interaction_with_timeout(state, session_id, interaction, operation, None).await
}

struct AbandonedInteractionGuard(Option<(KernelRuntimeState, String, String)>);

impl Drop for AbandonedInteractionGuard {
    fn drop(&mut self) {
        let Some((state, session_id, interaction_id)) = self.0.take() else {
            return;
        };
        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                let _ = state
                    .timeout_runtime_interaction(&session_id, &interaction_id)
                    .await;
            });
        }
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
    request: RequestNativeProviderTurnInteractionRequest,
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
    )
    .with_native_origin(Some(request.origin));
    let resolution = request_runtime_interaction_with_timeout(
        state,
        &session_id,
        interaction,
        "native_provider_interaction_request",
        None,
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
    use crate::session::NativeInteractionOrigin;
    if interaction
        .native_origin()
        .is_none_or(|origin| origin.provider_run_id() != context.worker_provider_run_id)
    {
        return Ok(ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        });
    }
    let origin = match interaction.native_origin() {
        Some(NativeInteractionOrigin::ProviderStartup { .. }) => {
            NativeInteractionOrigin::ProviderStartup {
                provider_run_id: context.worker_provider_run_id.clone(),
            }
        }
        Some(_) => NativeInteractionOrigin::Prompt {
            provider_run_id: context.worker_provider_run_id.clone(),
            prompt_id: context.home_prompt_id.clone().unwrap_or_default(),
        },
        None => {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            })
        }
    };
    let interaction = interaction
        .with_agent_id(context.home_agent_id.clone())
        .with_native_origin(Some(origin));
    request_runtime_interaction_with_timeout(
        state,
        &context.home_session_id,
        interaction,
        "relay_forward_native_interaction",
        Some(&context),
    )
    .await
}

async fn request_runtime_interaction_with_timeout(
    state: &KernelRuntimeState,
    session_id: &str,
    interaction: RuntimeInteraction,
    operation: &'static str,
    forwarding: Option<&RemoteNativeInteractionContext>,
) -> Result<ProviderNativeInteractionResolution, DaemonError> {
    // MP-08 / MP-10 / MP-11: parallel provider tools retain the one-decision
    // subject gate. Wait only for its temporary refusal; every attempt keeps
    // the original identity/origin and revalidates current runtime authority.
    let deadline = interaction
        .timeout_sec()
        .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds));
    let timeout_session_id = session_id.to_string();
    let timeout_interaction_id = interaction.id().to_string();
    let mut receiver = loop {
        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            });
        }
        match state
            .create_runtime_interaction_with_forwarding(session_id, interaction.clone(), forwarding)
            .await
        {
            Ok(receiver) => break receiver,
            Err(error)
                if state.runtime_interaction_admission_waits(
                    session_id,
                    interaction.id(),
                    &error,
                )? =>
            {
                let next = tokio::time::Instant::now() + Duration::from_millis(25);
                tokio::time::sleep_until(deadline.map_or(next, |deadline| deadline.min(next)))
                    .await;
            }
            Err(error) => return Err(error),
        }
    };
    // Cancellation can retire only an interaction this invocation admitted,
    // never the card whose subject/identity caused its admission to wait.
    let mut abandoned = AbandonedInteractionGuard(Some((
        state.clone(),
        timeout_session_id.clone(),
        timeout_interaction_id.clone(),
    )));
    // Waiting for another tool consumes this decision's original budget.
    let resolution = if let Some(deadline) = deadline {
        tokio::select! {
            biased;
            resolution = &mut receiver => resolution,
            _ = tokio::time::sleep_until(deadline) => {
                state.timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id).await?;
                receiver.await
            }
        }
    } else {
        receiver.await
    }.map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!("interaction dropped before resolution: {error}"),
    })?;
    abandoned.0 = None;
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

    async fn permission_queue_fixture() -> (
        crate::runtime::router::CommandRouter,
        String,
        String,
        std::path::PathBuf,
    ) {
        let root = crate::test_support::test_workspace_root().join(format!(
            "native-permission-queue-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let worktree = root.to_string_lossy().into_owned();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &worktree, &worktree,
            ))
            .unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        (router, session.id().into(), agent.id().into(), root)
    }

    fn queued_permission(id: &str, agent: &str, timeout: u64) -> RuntimeInteraction {
        RuntimeInteraction::new(
            id,
            agent,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve documentation fetch?".into()),
            "Fetch the requested public documentation",
            vec![
                RuntimeInteractionChoice::new("allow", "Allow once", "allow", None),
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
            ],
            None,
            Some(timeout),
            Some("deny".into()),
        )
    }

    #[tokio::test]
    async fn native_permission_queue_waits_for_the_existing_subject_decision() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let first = state
            .create_runtime_interaction(&session, queued_permission("first", &agent, 10))
            .await
            .unwrap();
        let pending_state = state.clone();
        let pending_session = session.clone();
        let second = tokio::spawn(async move {
            request_runtime_interaction_with_timeout(
                &pending_state,
                &pending_session,
                queued_permission("second", &agent, 5),
                "native_permission_queue_test",
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !second.is_finished(),
            "MP-10/MP-11 parallel permission must wait for the current decision"
        );
        state
            .resolve_runtime_interaction(&session, "first", "allow", None)
            .await
            .unwrap();
        assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if state
                .resolve_runtime_interaction(&session, "second", "deny", None)
                .await
                .is_ok()
            {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let resolution = second.await.unwrap().unwrap();
        assert_eq!(resolution.choice_id.as_deref(), Some("deny"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn native_permission_queue_counts_waiting_time_against_its_deadline() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let _first = state
            .create_runtime_interaction(&session, queued_permission("first", &agent, 10))
            .await
            .unwrap();
        let started = tokio::time::Instant::now();
        let resolution = request_runtime_interaction_with_timeout(
            &state,
            &session,
            queued_permission("second", &agent, 1),
            "native_permission_queue_test",
            None,
        )
        .await
        .unwrap();
        assert_eq!(resolution.status, "timed_out");
        assert!(started.elapsed() < Duration::from_millis(1500));
        state
            .resolve_runtime_interaction(&session, "first", "deny", None)
            .await
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn native_permission_queue_cancellation_keeps_the_existing_decision() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let first = state
            .create_runtime_interaction(&session, queued_permission("shared", &agent, 10))
            .await
            .unwrap();
        let waiting_state = state.clone();
        let waiting_session = session.clone();
        let waiting = tokio::spawn(async move {
            request_runtime_interaction_with_timeout(
                &waiting_state,
                &waiting_session,
                queued_permission("shared", &agent, 5),
                "native_permission_queue_test",
                None,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiting.is_finished());
        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        state
            .resolve_runtime_interaction(&session, "shared", "allow", None)
            .await
            .unwrap();
        assert_eq!(first.await.unwrap().choice_id.as_deref(), Some("allow"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn native_permission_queue_does_not_retry_foreign_session_admission() {
        let (router, session, agent, root) = permission_queue_fixture().await;
        let state = router.runtime_state();
        let started = tokio::time::Instant::now();
        let result = request_runtime_interaction_with_timeout(
            &state,
            &format!("{session}-foreign"),
            queued_permission("foreign", &agent, 30),
            "native_permission_queue_test",
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_millis(500));
        std::fs::remove_dir_all(root).unwrap();
    }
}
