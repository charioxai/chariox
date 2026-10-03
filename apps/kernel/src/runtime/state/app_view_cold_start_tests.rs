use super::*;
use crate::{
    app::{DaemonApp, KernelSessionService},
    config::{CredentialVaultBackend, DaemonConfig},
    runtime::router::CommandRouter,
    session::{CanonicalViewport, CreateSessionRequest, EnvironmentLifecycle},
    slice::{CreateSliceInput, SliceBackendKind, SliceDisplayMode, SliceStatus},
};
use std::sync::Arc;
use tokio::sync::Mutex;

/// The pump's idle poll cadence (`AppViewPoll`), which busy refusals keep.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[tokio::test]
async fn running_slice_start_does_not_spend_restore_or_poll_failure_budgets() {
    let root =
        std::env::temp_dir().join(format!("chariox-cold-pump-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.user_config.credential_vault.backend = CredentialVaultBackend::ProcessMemory;
    config.user_config.credential_vault.path = root.join("vault.json").display().to_string();
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, _) = KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("cold-pump", "cold-pump"))
        .unwrap();
    let session = session.id().to_owned();
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
    let store = &runtime.owned.slice_store;
    let slice = store
        .create(
            "test",
            "test",
            CreateSliceInput {
                source_slice_ref: None,
                name: "cold-pump".into(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: SliceDisplayMode::Headed,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap();
    store
        .bind_environment(&session, &slice.id, 1, |_| Ok(()))
        .unwrap();
    let startup = store.try_begin_operation(&slice.id, "slice.start").unwrap();
    store
        .set_status(&slice.id, SliceStatus::Running, 2)
        .unwrap();
    // The home Room can retain stale Ready metadata while agents relaunch.
    runtime
        .start_room_environment(
            &session,
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    runtime
        .transition_room_environment(&session, EnvironmentLifecycle::Ready)
        .unwrap();
    let views = runtime.app_control().views().clone();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
    };
    views.register(&session, "old", binding.clone());
    views.suspend_for_cold_start(&session);
    let pump = tokio::spawn(runtime.clone().pump_app_view_calls(session.clone()));
    // Longer than both the old three-attempt limit and MAX_POLL_FAILURES.
    tokio::time::sleep(POLL_INTERVAL * (MAX_POLL_FAILURES + 8)).await;
    assert!(!pump.is_finished());
    assert_eq!(views.cold_start_views(&session), vec![binding.clone()]);
    pump.abort();
    let _ = pump.await;
    drop(startup);
    // All admitted attempts are still available after the lifecycle releases.
    for _ in 0..3 {
        assert_eq!(
            views.next_cold_start_attempt(&session),
            Some(binding.clone())
        );
        views.fail_cold_start_view(&session, &binding);
    }
    assert!(views.next_cold_start_attempt(&session).is_none());
    // A permanently Failed Room must also retire genuine restore failures,
    // rather than a readiness gate trapping the pump and other view calls.
    runtime
        .transition_room_environment(&session, EnvironmentLifecycle::Failed)
        .unwrap();
    views.register(&session, "retry", binding.clone());
    views.suspend_for_cold_start(&session);
    tokio::time::timeout(
        Duration::from_secs(10),
        runtime.clone().pump_app_view_calls(session.clone()),
    )
    .await
    .unwrap();
    assert!(views.cold_start_views(&session).is_empty());
}
