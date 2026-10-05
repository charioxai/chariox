use super::session_authority::{external_command, runtime};
use super::worker_spy::WorkerSpy;
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::runtime::router::CommandRouter;
use crate::session::{
    agent_environment_actor_id, ActionAdmission, CanonicalViewport, EnvironmentActionRequest,
    EnvironmentActionTerminal, EnvironmentLifecycle,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;
use tokio::time::timeout;

macro_rules! capability_regression {
    ($name:ident, $kind:expr) => {
        #[test]
        fn $name() {
            std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .max_blocking_threads(1)
                        .build()
                        .unwrap()
                        .block_on(revoked_capability($kind));
                })
                .unwrap()
                .join()
                .unwrap();
        }
    };
}
capability_regression!(
    kernel_access_capability_shell_rechecks_blocking_queue,
    "shell"
);
capability_regression!(
    kernel_access_capability_edit_rechecks_blocking_queue,
    "edit"
);
capability_regression!(
    kernel_access_capability_transfer_rechecks_blocking_queue,
    "transfer"
);

async fn revoked_capability(kind: &str) {
    let worktree = crate::test_support::TestWorktree::new("access-capability");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "capability-owner",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app, 32);
    let state = router.runtime_state();
    let grant = state.insert_access_grant_for_test(session.id());
    let marker = worktree.path().join("effect.txt");
    let source = worktree.path().join("source.txt");
    std::fs::write(&source, "transfer fixture").unwrap();
    let transfer_root =
        crate::app::attachment_artifact_root(session.id(), attachment.id(), "transfers");
    let request = match kind {
        "shell" => LocalDaemonRequest::RunShellCommand(crate::local::RunShellCapabilityRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf effect > effect.txt".into()],
            working_directory: None,
            timeout_ms: Some(1000),
        }),
        "edit" => LocalDaemonRequest::EditFile(crate::local::EditFileCapabilityRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            path: "effect.txt".into(),
            contents: "effect".into(),
        }),
        "transfer" => LocalDaemonRequest::StoreTransferredFile(
            crate::local::StoreTransferredFileCapabilityRequest {
                session_id: session.id().into(),
                attachment_id: attachment.id().into(),
                source_path: source,
                display_name: Some("effect.txt".into()),
            },
        ),
        _ => unreachable!(),
    };
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        blocked.recv().unwrap();
    });
    ready.await.unwrap();
    let pending = router.dispatch(external_command(&request, &grant), request.clone());
    tokio::pin!(pending);
    // The current-thread runtime polls dispatch through spawn_blocking; the only
    // blocking thread is held, so the capability closure cannot have started.
    tokio::select! { biased; result = &mut pending => panic!("capability did not queue: {}", result.is_ok()), _ = tokio::task::yield_now() => {} }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    release.send(()).unwrap();
    blocker.await.unwrap();
    let result = timeout(Duration::from_secs(3), pending).await.unwrap();
    let mutated = marker.exists() || transfer_root.exists();
    // This exact random session/attachment artifact root belongs to this fixture.
    if transfer_root.exists() {
        std::fs::remove_dir_all(&transfer_root).unwrap();
    }
    assert!(
        !mutated,
        "revoked capability executed its file/process effect"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    router
        .dispatch(
            crate::runtime::command::KernelCommand::from_local_request(
                "terminal-capability",
                None,
                None,
                &request,
            ),
            request,
        )
        .await
        .unwrap();
    assert!(
        marker.exists() || transfer_root.exists(),
        "ordinary terminal capability did not execute"
    );
    if transfer_root.exists() {
        std::fs::remove_dir_all(&transfer_root).unwrap();
    }
}

#[tokio::test]
async fn kernel_access_room_stop_rechecks_worker_discovery() {
    let worktree = crate::test_support::TestWorktree::new("access-room-stop");
    let worker = WorkerSpy::new(true);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(worker.url.clone());
    config.relay_token = Some("room-stop-fixture".into());
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let slice: crate::slice::SliceRecord = serde_json::from_value(serde_json::json!({
        "id":"room-slice", "name":"room-slice", "owner_kernel_id":"fixture-home", "owner_machine_id":"fixture-machine", "environment_session_id":session.id(),
        "backend":"ssh_docker", "os":"linux", "display_mode":"headed", "status":"running", "workspace_mount":null,
        "worker_kernel_ref":worker.id, "worker_kernel_id":worker.id, "relay_endpoint":{"url":worker.url,"private":false}, "providers":[], "provider_auth":[], "created_at_ms":1, "updated_at_ms":1
    })).unwrap();
    app.slices().restore_records(vec![slice]);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app.clone(), 32);
    let state = router.runtime_state();
    state
        .start_room_environment(
            session.id(),
            CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
        )
        .unwrap();
    state
        .transition_room_environment(session.id(), EnvironmentLifecycle::Ready)
        .unwrap();
    let runtime = runtime(&state, &*app.lock().await);
    let grant = state.insert_access_grant_for_test(session.id());
    let request =
        LocalDaemonRequest::StopRoomEnvironment(crate::local::StopRoomEnvironmentRequest {
            session_id: session.id().into(),
        });
    let pending =
        runtime.dispatch_session_command(external_command(&request, &grant), request.clone());
    tokio::pin!(pending);
    tokio::select! { biased; result = &mut pending => panic!("Room stop did not wait: {result:?}"), signal = timeout(Duration::from_secs(3), worker.discovery_started.notified()) => { signal.unwrap(); } }
    let before = state.room_environment_snapshot(session.id()).unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    worker.release_discovery.notify_one();
    let result = timeout(Duration::from_secs(3), pending).await.unwrap();
    assert_eq!(
        worker.requests.load(Ordering::SeqCst),
        0,
        "revoked Room stop reached worker"
    );
    let after = state.room_environment_snapshot(session.id()).unwrap();
    assert_eq!(before.lifecycle, EnvironmentLifecycle::Stopping);
    assert_eq!(
        after.lifecycle,
        EnvironmentLifecycle::Failed,
        "denied release must settle the admitted stop under kernel authority"
    );
    assert!(after.event_cursor > before.event_cursor);
    // Revocation still forbids worker effects. Only terminal bookkeeping is allowed.
    let mut expected = before;
    expected.lifecycle = EnvironmentLifecycle::Failed;
    expected.event_cursor = after.event_cursor;
    let controller = expected
        .health
        .iter_mut()
        .find(|health| health.component == crate::session::EnvironmentComponent::BrowserController)
        .unwrap();
    controller.state = crate::session::EnvironmentComponentHealthState::Unavailable;
    controller.diagnostic_code = Some("controller_stop_failed".into());
    assert_eq!(
        after, expected,
        "denied stop changed more than terminal bookkeeping"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    runtime
        .dispatch_session_command(
            crate::runtime::command::KernelCommand::from_local_request(
                "terminal-room-stop",
                None,
                None,
                &request,
            ),
            request,
        )
        .await
        .unwrap();
    assert_eq!(worker.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn kernel_access_browser_mutation_rechecks_execution_gate() {
    revoked_browser_mutation(false).await;
}
#[tokio::test]
async fn kernel_access_browser_mutation_rechecks_action_admission() {
    revoked_browser_mutation(true).await;
}

async fn revoked_browser_mutation(queued: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-browser-wait");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity_from_app(app, 32);
    let state = router.runtime_state();
    state
        .start_room_environment(
            session.id(),
            CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
        )
        .unwrap();
    state
        .transition_room_environment(session.id(), EnvironmentLifecycle::Ready)
        .unwrap();
    let environment = state
        .reconcile_room_environment_actors(session.id(), None)
        .unwrap();
    let action_request = || {
        EnvironmentActionRequest::computer_mutation(
            agent_environment_actor_id(agent.id()),
            environment.runtime_generation,
            "pointer_move",
            None,
        )
    };
    let active = if queued {
        let (ActionAdmission::Accepted { action_id }, _) = state
            .submit_room_environment_action(session.id(), action_request())
            .unwrap()
        else {
            panic!("fixture action was not accepted")
        };
        Some(action_id)
    } else {
        None
    };
    let gate = if queued {
        None
    } else {
        Some(
            state
                .owned
                .environment_execution_gates
                .for_room(session.id())
                .write_owned()
                .await,
        )
    };
    let grant = state.insert_access_grant_for_test(session.id());
    let request =
        LocalDaemonRequest::StopRoomEnvironment(crate::local::StopRoomEnvironmentRequest {
            session_id: session.id().into(),
        });
    let authorized = state.with_external_command_authority(Some((&grant, &request)));
    let executed = AtomicBool::new(false);
    let pending =
        authorized.execute_browser_mutation(session.id(), action_request(), None, async {
            executed.store(true, Ordering::SeqCst);
            Ok(())
        });
    tokio::pin!(pending);
    tokio::select! { biased; result = &mut pending => panic!("browser mutation did not wait: {}", result.is_ok()), _ = tokio::task::yield_now() => {} }
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    drop(gate);
    if let Some(active) = active {
        state
            .finish_room_environment_action(
                session.id(),
                &active,
                EnvironmentActionTerminal::Completed,
            )
            .unwrap();
    }
    let result = timeout(Duration::from_secs(3), pending).await.unwrap();
    assert!(
        !executed.load(Ordering::SeqCst),
        "revoked browser action executed"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert!(
        state
            .room_environment_snapshot(session.id())
            .unwrap()
            .actions
            .iter()
            .all(|action| !matches!(
                action.state,
                crate::session::EnvironmentActionState::Running
                    | crate::session::EnvironmentActionState::Queued
            )),
        "revoked action remains eligible for execution"
    );
    state
        .execute_browser_mutation(session.id(), action_request(), None, async {
            executed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
    assert!(executed.load(Ordering::SeqCst));
}
