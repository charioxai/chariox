use super::*;
use crate::config::DaemonConfig;
use crate::session::{
    human_environment_actor_id, ActionAdmission, CanonicalViewport, CreateSessionRequest,
    EnvironmentActionRequest, EnvironmentActionState, EnvironmentActorKind, EnvironmentLifecycle,
    DEFAULT_LOCAL_USER_ID,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn browser_import_recovery_blocks_admission_and_execution_until_cleanup() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(exercise_recovery_gate());
        })
        .unwrap()
        .join()
        .unwrap();
}

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn exercise_recovery_gate() {
    let config = DaemonConfig::for_tests();
    let database_path = config.user_config.state.path.clone().unwrap();
    let _scratch = Scratch(
        std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
            .parent()
            .unwrap()
            .to_path_buf(),
    );
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let session = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "import-recovery-gate",
            "worktree",
        ))
        .unwrap()
        .0;
    let other_room = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("unaffected-room", "worktree"))
        .unwrap()
        .0;
    let app = Arc::new(Mutex::new(app));
    let state = owned_runtime_state(&app).await;
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
        .reconcile_room_environment_actors(session.id(), Some(DEFAULT_LOCAL_USER_ID))
        .unwrap();
    let request = || {
        EnvironmentActionRequest::computer_mutation(
            human_environment_actor_id(DEFAULT_LOCAL_USER_ID),
            environment.runtime_generation,
            "pointer_move",
            None,
        )
    };
    state
        .owned
        .durable_state_store
        .begin_browser_import_recovery(
            &environment.environment_id,
            "interrupted-import",
            DEFAULT_LOCAL_USER_ID,
            session.id(),
        )
        .unwrap();
    state
        .ensure_browser_import_execution_allowed(other_room.id())
        .unwrap();
    for recovered in [false, true] {
        if recovered {
            state
                .owned
                .durable_state_store
                .mark_browser_import_recovered(&environment.environment_id, "interrupted-import")
                .unwrap();
        }
        let error = state.submit_room_environment_action(session.id(), request())
            .expect_err("pending import must prevent Action admission, including after recovery before journal cleanup");
        assert_eq!(error.code(), "environment_browser_import_recovery_required");
        assert!(state
            .room_environment_snapshot(session.id())
            .unwrap()
            .actions
            .is_empty());
        let error = state
            .room_browser_controller_command(session.id(), RoomBrowserControllerCommand::Acquire)
            .await
            .expect_err("pending import must prevent controller acquisition");
        assert!(error
            .to_string()
            .contains("environment_browser_import_recovery_required"));
        let polled = AtomicBool::new(false);
        let result = state
            .await_cancellable_browser_action(session.id(), "queued-action", "execution", async {
                polled.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert!(
            result.is_err(),
            "an already-admitted executor must fail before its first poll"
        );
        assert!(!polled.load(Ordering::SeqCst));
        assert_cleanup_allowed(&state, session.id()).await;
    }
    state
        .owned
        .durable_state_store
        .clear_recovered_browser_import(&environment.environment_id, "interrupted-import")
        .unwrap();
    let ActionAdmission::Accepted {
        action_id: active_id,
    } = state
        .submit_room_environment_action(session.id(), request())
        .unwrap()
        .0
    else {
        panic!("execution should reopen after cleanup");
    };

    // Both waiters must cancel queued work, not leave it eligible for later
    // promotion after the import barrier is lifted.
    let mut queued = Vec::new();
    let agent = environment
        .actors
        .iter()
        .find(|actor| actor.kind == EnvironmentActorKind::Agent)
        .unwrap();
    for queued_request in [
        request(),
        EnvironmentActionRequest::computer_mutation(
            &agent.actor_id,
            environment.runtime_generation,
            "pointer_move",
            None,
        ),
    ] {
        let admission = state
            .submit_room_environment_action(session.id(), queued_request)
            .unwrap()
            .0;
        let ActionAdmission::Queued { action_id, .. } = admission else {
            panic!("expected queued action, got {admission:?}");
        };
        queued.push(action_id);
    }
    state
        .owned
        .durable_state_store
        .begin_browser_import_recovery(
            &environment.environment_id,
            "queued-import",
            DEFAULT_LOCAL_USER_ID,
            session.id(),
        )
        .unwrap();
    let actor = environment
        .actors
        .iter()
        .find(|actor| actor.actor_id == human_environment_actor_id(DEFAULT_LOCAL_USER_ID))
        .unwrap();
    let human_error = state
        .wait_for_human_action_admission(session.id(), actor, &queued[0])
        .await
        .unwrap_err();
    assert!(human_error
        .to_string()
        .contains("environment_browser_import_recovery_required"));
    let agent_error = state
        .wait_for_environment_action_admission(session.id(), &queued[1])
        .await
        .unwrap_err();
    assert!(agent_error
        .to_string()
        .contains("environment_browser_import_recovery_required"));
    let snapshot = state.room_environment_snapshot(session.id()).unwrap();
    for id in queued {
        assert_eq!(
            snapshot
                .actions
                .iter()
                .find(|action| action.action_id == id)
                .unwrap()
                .state,
            EnvironmentActionState::Cancelled,
            "blocked waiter must retire its queued action"
        );
    }
    // The waiter can observe promotion to Running before it notices recovery.
    // Since it has not dispatched yet, it must retire that Action too.
    state
        .wait_for_human_action_admission(session.id(), actor, &active_id)
        .await
        .unwrap_err();
    assert_eq!(
        state
            .room_environment_snapshot(session.id())
            .unwrap()
            .actions
            .iter()
            .find(|action| action.action_id == active_id)
            .unwrap()
            .state,
        EnvironmentActionState::Cancelled
    );

    // A Room binding survives replacement of its current Environment identity.
    state
        .owned
        .durable_state_store
        .begin_browser_import_recovery(
            "previous-environment",
            "old-import",
            DEFAULT_LOCAL_USER_ID,
            other_room.id(),
        )
        .unwrap();
    assert_eq!(
        state
            .ensure_browser_import_execution_allowed(other_room.id())
            .unwrap_err()
            .code(),
        "environment_browser_import_recovery_required"
    );

    // A failed durable read must deny dispatch, but must never deny cleanup.
    let database = rusqlite::Connection::open(database_path).unwrap();
    database
        .execute("DROP TABLE durable_browser_import", [])
        .unwrap();
    assert_eq!(
        state
            .submit_room_environment_action(session.id(), request())
            .unwrap_err()
            .code(),
        "environment_browser_import_recovery_state_unavailable"
    );
    let error = state
        .room_browser_controller_command(session.id(), RoomBrowserControllerCommand::Acquire)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("environment_browser_import_recovery_state_unavailable"));
    assert_cleanup_allowed(&state, session.id()).await;
}

async fn assert_cleanup_allowed(state: &KernelRuntimeState, room: &str) {
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;
    assert!(matches!(
        state
            .room_browser_controller_command(
                room,
                RoomBrowserControllerCommand::CancelAction {
                    execution_id: "absent-execution".into()
                }
            )
            .await
            .unwrap(),
        RoomBrowserControllerResult::CancellationRequested { accepted: false }
    ));
    assert!(matches!(
        state
            .room_browser_controller_command(room, RoomBrowserControllerCommand::Release)
            .await
            .unwrap(),
        RoomBrowserControllerResult::Process { snapshot: None }
    ));
}
