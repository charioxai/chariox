use super::recovery::MAX_POLL_FAILURES;
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

pub(super) struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        // Staged App releases are sealed read-only; an unprivileged user can
        // delete them only after their directories are writable again.
        make_dirs_writable(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_dirs_writable(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).is_err() {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            make_dirs_writable(&entry.path());
        }
    }
}

fn fixture() -> (KernelRuntimeState, String, Cleanup) {
    fixture_for_owner("local")
}

pub(super) fn fixture_for_owner(owner: &str) -> (KernelRuntimeState, String, Cleanup) {
    let root =
        std::env::temp_dir().join(format!("chariox-cold-pump-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let cleanup = Cleanup(root.clone());
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
        .create_session(
            CreateSessionRequest::new("cold-pump", "cold-pump").with_owner_user_id(owner),
        )
        .unwrap();
    let session = session.id().to_owned();
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
    (runtime, session, cleanup)
}

#[test]
fn mp08_late_browser_pump_failure_keeps_saved_app_tabs() {
    let (runtime, _, _cleanup) = fixture();
    let owner = "local";
    let views = runtime.app_control().user_views();
    let view = views
        .open(
            owner,
            AppViewBinding {
                owner: owner.into(),
                installation: "todo".into(),
                generation: 1,
                panel: Default::default(),
                logical_tab: None,
            },
            "a",
        )
        .unwrap();
    views
        .bind_browser(
            owner,
            &view.view_id,
            "target".into(),
            crate::local::UserAppViewBrowser {
                tab_id: "host-tab-saved".into(),
                generation: 1,
            },
        )
        .unwrap();
    runtime.remember_user_app_tabs(owner);
    let path = runtime
        .owned
        .kernel_browser_host
        .profile_root(&runtime.provider_account_authority_owner_user_id(owner))
        .join("app-tabs.json");
    let saved = std::fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&saved).contains("todo"));
    // Shutdown retires instances before stopping the controller. A poll
    // already in flight then reports its failed drain against the old view.
    views.shutdown();
    runtime.suspend_user_app_view(owner, &view.view_id);
    assert_eq!(
        std::fs::read(path).unwrap(),
        saved,
        "a late pump result cannot overwrite the owner's durable Tab intent"
    );
}

#[tokio::test]
async fn running_slice_start_does_not_spend_restore_or_poll_failure_budgets() {
    let (runtime, session, _cleanup) = fixture();
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
        logical_tab: None,
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

#[test]
fn positive_browser_loss_preserves_restore_attempts_but_real_errors_are_final() {
    for diagnostic in [
        "browser_debugger_unavailable",
        "browser_cdp_disconnected",
        "browser controller is not leased by Room room",
    ] {
        assert!(matches!(
            cold_restore_error(open_error(diagnostic)),
            ColdAppRestoreError::Busy
        ));
    }
    assert!(matches!(
        cold_restore_error(open_error("invalid asset")),
        ColdAppRestoreError::Failed(_)
    ));
}

#[test]
fn local_room_has_no_slice_restore_gate() {
    assert!(!cold_restore_waiting_for_slice(None));
    assert!(!cold_restore_waiting_for_slice(Some(SliceStatus::Running)));
    assert!(cold_restore_waiting_for_slice(Some(SliceStatus::Stopped)));
    assert!(cold_restore_waiting_for_slice(Some(SliceStatus::Unhealthy)));
    assert!(!browser_recovery_downtime("browser_controller_unreachable"));
}

#[tokio::test]
async fn stopped_local_room_does_not_acquire_browser_for_pending_restore() {
    let (runtime, session, _cleanup) = fixture();
    assert!(runtime
        .owned
        .slice_store
        .environment_slice(&session)
        .is_none());
    runtime
        .start_room_environment(
            &session,
            CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    runtime.stop_room_environment(&session).unwrap();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    assert!(matches!(
        runtime.restore_cold_app_view(&session, &binding).await,
        Err(ColdAppRestoreError::Failed(AppRequestErrorCode::NotFound))
    ));
    assert_eq!(
        runtime
            .room_environment_snapshot(&session)
            .unwrap()
            .lifecycle,
        EnvironmentLifecycle::Stopped
    );
}

#[tokio::test]
async fn inconclusive_identity_check_keeps_drained_live_calls_and_cancels_gone_documents() {
    let views = crate::runtime::app_views::AppViews::default();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    views.register("room", "gone", binding.clone());
    views.register("room", "live", binding);
    let gone = views.track_call("room", "gone", Some("old-document".into()));
    let batch = BrowserAppViewCalls {
        calls: vec![BrowserAppViewCall {
            installation_id: "todo".into(),
            target_id: "live".into(),
            document_id: Some("live-document".into()),
            call_id: "call-1".into(),
            method: "list_todos".into(),
            params: serde_json::json!({}),
            actor: None,
        }],
        open_targets: Some(vec!["live".into()]),
        documents: Some(std::collections::HashMap::from([(
            "live".into(),
            "live-document".into(),
        )])),
        app_panels: false,
    };
    retain_app_poll_targets(
        &views,
        "room",
        &batch,
        views.registrations("room"),
        &mut tokio::time::Instant::now(),
        async { Err::<(), _>("controller_busy") },
    )
    .await;
    // The missing binding is not pruned without proof of browser identity.
    assert!(views.binding_state("room", "gone").is_some());
    assert!(views.cold_start_views("room").is_empty());
    assert_eq!(batch.calls.len(), 1);
    let call = &batch.calls[0];
    let live = views.track_call("room", &call.target_id, call.document_id.clone());
    assert_eq!(
        views.cancel_gone_calls(
            "room",
            batch.open_targets.as_deref(),
            batch.documents.as_ref()
        ),
        1
    );
    assert!(gone.is_cancelled());
    assert!(!live.is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn inconclusive_identity_probes_are_throttled_without_slowing_live_calls() {
    let views = crate::runtime::app_views::AppViews::default();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    views.register("room", "missing", binding.clone());
    views.register(
        "room",
        "live",
        AppViewBinding {
            installation: "other".into(),
            ..binding
        },
    );
    let batch = BrowserAppViewCalls {
        calls: vec![BrowserAppViewCall {
            installation_id: "other".into(),
            target_id: "live".into(),
            document_id: None,
            call_id: "call-1".into(),
            method: "list_todos".into(),
            params: serde_json::json!({}),
            actor: None,
        }],
        open_targets: Some(vec!["live".into()]),
        documents: None,
        app_panels: false,
    };
    let attempts = std::cell::Cell::new(0);
    let mut probe_after = tokio::time::Instant::now();
    let began = probe_after;
    let mut polls = super::super::app_view_poll::AppViewPoll::new();
    polls.tick().await;
    for n in 0..=10 {
        assert_eq!(
            tokio::time::Instant::now() - began,
            Duration::from_millis(25) * n
        );
        retain_app_poll_targets(
            &views,
            "room",
            &batch,
            views.registrations("room"),
            &mut probe_after,
            async {
                attempts.set(attempts.get() + 1);
                Err::<(), _>("controller_busy")
            },
        )
        .await;
        assert_eq!(attempts.get(), if n < 10 { 1 } else { 2 });
        // Drains and call handling stay at the active floor throughout.
        polls.observed_calls(!batch.calls.is_empty());
        polls.tick().await;
    }
    assert!(views.binding_state("room", "missing").is_some());
}

#[test]
fn viewport_verification_downtime_preserves_restore_and_poll_authority() {
    let message = "browser controller `browser.reconcile` failed with viewport_apply_failed: canonical physical display apply failed";
    let local = DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.into(),
    };
    let relay = DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: "transport_error".into(),
        message: local.to_string(),
        retryable: true,
    };
    // App commands wrap the same fault in their existing unavailable envelope.
    for error in [local, relay] {
        let wrapped = open_error(&error.to_string());
        assert!(matches!(
            cold_restore_error(error),
            ColdAppRestoreError::ViewportUnavailable
        ));
        assert!(matches!(
            cold_restore_error(wrapped),
            ColdAppRestoreError::ViewportUnavailable
        ));
    }
    let unavailable = open_error("This Room has no browser controller available.");
    assert!(matches!(
        cold_restore_error(unavailable),
        ColdAppRestoreError::Failed(_)
    ));
}

fn viewport_fault() -> DaemonError {
    open_error("browser controller `browser.reconcile` failed with viewport_apply_failed: canonical physical display apply failed")
}

#[tokio::test(start_paused = true)]
async fn permanent_viewport_failure_eventually_retires_restore_and_poll_bindings() {
    let views = crate::runtime::app_views::AppViews::default();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    views.register("cold", "old", binding.clone());
    views.suspend_for_cold_start("cold");
    views.register("live", "target", binding.clone());
    let mut recovery = AppViewRecovery::default();
    let mut polling = AppViewRecovery::default();
    for _ in 0..MAX_POLL_FAILURES + 8 {
        recovery.restore_finished(
            &views,
            "cold",
            &binding,
            Err(cold_restore_error(viewport_fault())),
        );
        polling.poll_failed(&views, "live", &viewport_fault());
        assert_eq!(views.next_cold_start_attempt("cold"), Some(binding.clone()));
        assert!(views.keep_pumping("live"));
        tokio::time::advance(Duration::from_millis(250)).await;
    }
    tokio::time::advance(Duration::from_secs(120)).await;
    for _ in 0..3 {
        recovery.restore_finished(
            &views,
            "cold",
            &binding,
            Err(cold_restore_error(viewport_fault())),
        );
    }
    assert!(views.next_cold_start_attempt("cold").is_none());
    assert!(!views.keep_pumping("cold"));
    for _ in 0..MAX_POLL_FAILURES {
        polling.poll_failed(&views, "live", &viewport_fault());
    }
    assert!(!views.keep_pumping("live"));
}

#[tokio::test(start_paused = true)]
async fn successful_poll_rearms_a_new_bounded_viewport_window() {
    let views = crate::runtime::app_views::AppViews::default();
    let binding = AppViewBinding {
        owner: "local".into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    views.register("room", "old", binding.clone());
    views.suspend_for_cold_start("room");
    let mut recovery = AppViewRecovery::default();
    recovery.restore_finished(
        &views,
        "room",
        &binding,
        Err(cold_restore_error(viewport_fault())),
    );
    tokio::time::advance(Duration::from_secs(121)).await;
    recovery.poll_succeeded();
    for _ in 0..4 {
        recovery.restore_finished(
            &views,
            "room",
            &binding,
            Err(cold_restore_error(viewport_fault())),
        );
    }
    assert_eq!(views.next_cold_start_attempt("room"), Some(binding.clone()));
    tokio::time::advance(Duration::from_secs(120)).await;
    for _ in 0..3 {
        recovery.restore_finished(
            &views,
            "room",
            &binding,
            Err(cold_restore_error(viewport_fault())),
        );
    }
    assert!(views.next_cold_start_attempt("room").is_none());
}

/// #992 review F5: an agent without its own App grant reaches no App host action
/// (clipboard, links) either; the grant check runs before the host broker.
#[tokio::test]
async fn mp11_an_ungranted_agent_reaches_no_app_host_action() {
    let (runtime, session, _cleanup) = fixture();
    let agent = runtime
        .owned
        .agent_store
        .list_agents()
        .into_iter()
        .find(|agent| agent.session_id() == session)
        .unwrap();
    let binding = AppViewBinding {
        owner: agent.owner_user_id().into(),
        installation: "todo".into(),
        generation: 1,
        panel: Default::default(),
        logical_tab: None,
    };
    for tool in ["host.clipboard_write", "host.open_link"] {
        let mut tracked = runtime
            .app_control()
            .views()
            .track_call("view", "view", None);
        let refused = runtime
            .invoke_app_view_tool(
                ViewCaller::Agent(&agent),
                &binding,
                tool,
                serde_json::json!({}),
                &mut tracked,
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, "AGENT_APP_NOT_GRANTED", "{tool}");
    }
}

/// #992 review F3 follow-up (live r3b step 07): an App the owner reopens before the
/// first restore pass settles exactly one saved Tab, not two.
#[test]
fn mp08_an_app_reopened_before_seeding_settles_one_saved_tab() {
    let (runtime, _, _cleanup) = fixture();
    let owner = "local";
    let path = runtime
        .owned
        .kernel_browser_host
        .profile_root(&runtime.provider_account_authority_owner_user_id(owner))
        .join("app-tabs.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let saved = serde_json::json!({"apps": [
        {"owner": owner, "installation_id": "todo"},
        {"owner": owner, "installation_id": "todo"},
    ]});
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let views = runtime.app_control().user_views();
    let view = views
        .open(
            owner,
            AppViewBinding {
                owner: owner.into(),
                installation: "todo".into(),
                generation: 1,
                panel: Default::default(),
                logical_tab: None,
            },
            "a",
        )
        .unwrap();
    views
        .bind_browser(
            owner,
            &view.view_id,
            "target".into(),
            crate::local::UserAppViewBrowser {
                tab_id: "host-tab-reopened".into(),
                generation: 1,
            },
        )
        .unwrap();
    runtime.remember_user_app_tabs(owner);
    // An App is one Tab: the saved duplicate collapsed, and the explicit open settled it.
    assert!(views.restore_batch(owner).is_empty());
}

/// MP-08/MP-10/MP-11 #992: shutdown/restart while the second App is opening.
#[test]
fn mp08_inflight_restore_intent_survives_kernel_shutdown_and_restart() {
    let (runtime, _, _cleanup) = fixture();
    let owner = "local";
    let path = runtime
        .owned
        .kernel_browser_host
        .profile_root(owner)
        .join("app-tabs.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, br#"{"apps":[{"owner":"local","installation_id":"todo"},{"owner":"local","installation_id":"documents"}]}"#).unwrap();
    let views = runtime.app_control().user_views();
    runtime.remember_user_app_tabs(owner);
    assert!(views.begin_restore(owner));
    let batch = views.restore_batch(owner);
    assert_eq!(batch, vec!["todo", "documents"]);
    let first = views
        .open(
            owner,
            AppViewBinding {
                owner: owner.into(),
                installation: "todo".into(),
                generation: 1,
                panel: Default::default(),
                logical_tab: None,
            },
            "a",
        )
        .unwrap();
    views
        .bind_browser(
            owner,
            &first.view_id,
            "target".into(),
            crate::local::UserAppViewBrowser {
                tab_id: "host-tab-first".into(),
                generation: 1,
            },
        )
        .unwrap();
    runtime.remember_user_app_tabs(owner);
    let during = std::fs::read(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&during).unwrap();
    assert_eq!(
        saved["apps"].as_array().unwrap().len(),
        2,
        "the second App remains durable while opening"
    );
    views.shutdown();
    runtime.remember_user_app_tabs(owner); // A late restore task's final save.
    assert_eq!(
        std::fs::read(&path).unwrap(),
        during,
        "shutdown fences late saves"
    );
    let restarted = crate::runtime::user_app_views::UserAppViews::default();
    restarted.seed_restore(
        owner,
        saved["apps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|app| app["installation_id"].as_str().unwrap().to_owned())
            .collect(),
    );
    assert_eq!(restarted.restore_batch(owner), vec!["todo", "documents"]);
}
