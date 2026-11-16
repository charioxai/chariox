use super::*;

#[tokio::test]
async fn revoke_after_pending_read_does_not_leave_a_stale_file_interaction() {
    use crate::durable_state::app_file_grants::{FileGrantCommand, FilePick, PickState};
    let worktree = crate::test_support::TestWorktree::new("file-revoke-prompt-race");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let owner = session.owner_user_id().to_owned();
    let session_id = session.id().to_owned();
    let store = app.durable_state_store();
    rusqlite::Connection::open(store.path()).unwrap().execute(
        "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json) VALUES('docs','com.example.docs',?1,3,3,'{}')", [&owner]
    ).unwrap();
    let now = crate::session::unix_epoch_ms();
    store
        .app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "revoke-race".into(),
            owner: owner.clone(),
            installation: "docs".into(),
            generation: 3,
            accept: vec![],
            multiple: false,
            state: PickState::Pending,
            expires_ms: now + 300_000,
            grants: vec![],
        }))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let read = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let pump_runtime = runtime.clone();
    let pump_read = read.clone();
    let pump_resume = resume.clone();
    let pump = tokio::spawn(async move {
        pump_runtime
            .app_file_pick_pass_with_read_barrier(now, pump_read, pump_resume)
            .await;
    });
    read.notified().await;
    assert_eq!(
        runtime
            .revoke_app_file_grants(
                owner.clone(),
                "docs".into(),
                crate::local::RevokeAppFileGrantsRequest {
                    installation_id: "docs".into(),
                    operation_id: Some("revoke-race".into()),
                }
            )
            .await
            .unwrap(),
        crate::local::LocalDaemonResponse::AppFileGrantsRevoked {
            installation_id: "docs".into(),
            requests: 1,
            files: 0
        }
    );
    resume.notify_one();
    pump.await.unwrap();
    let snapshot = runtime.owned.session_snapshot(&session_id).unwrap();
    let active = snapshot
        .active_interactions()
        .iter()
        .any(|interaction| interaction.id() == "app_file_pick_revoke-race");
    // Close a failed baseline's synthetic interaction before reporting its result.
    if active {
        runtime
            .timeout_runtime_interaction(&session_id, "app_file_pick_revoke-race")
            .await
            .unwrap();
    }
    assert!(
        !active,
        "a revoked pick must not reappear after stale pending registration"
    );
    assert!(
        runtime
            .app_control()
            .validation_prompt_session("revoke-race")
            .is_none(),
        "ended pick releases its prompt slot"
    );
}

#[tokio::test]
async fn a_request_that_ended_without_the_owner_closes_its_prompt_at_the_next_pass() {
    use crate::durable_state::app_file_grants::{FileGrantCommand, FilePick, PickState};
    let worktree = crate::test_support::TestWorktree::new("file-ended-prompt");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let owner = session.owner_user_id().to_owned();
    let session_id = session.id().to_owned();
    let store = app.durable_state_store();
    rusqlite::Connection::open(store.path()).unwrap().execute(
        "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json) VALUES('docs','com.example.docs',?1,3,3,'{}')", [&owner]
    ).unwrap();
    let now = crate::session::unix_epoch_ms();
    store
        .app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "file-pick-ended".into(),
            owner: owner.clone(),
            installation: "docs".into(),
            generation: 3,
            accept: vec![],
            multiple: false,
            state: PickState::Pending,
            expires_ms: now + 300_000,
            grants: vec![],
        }))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let shown = |runtime: &KernelRuntimeState| {
        runtime
            .owned
            .session_snapshot(&session_id)
            .unwrap()
            .active_interactions()
            .iter()
            .any(|interaction| interaction.id() == "app_file_pick_file-pick-ended")
    };
    runtime.app_file_pick_pass(now).await;
    assert!(
        shown(&runtime),
        "the owner is asked while the request is open"
    );
    // The call that waited for the choice went away: the App ends the request, the owner has not answered.
    store
        .revoke_app_file_grants(FileGrantCommand::Revoke {
            owner,
            installation: "docs".into(),
            operation_id: Some("file-pick-ended".into()),
            now_ms: now,
        })
        .unwrap();
    runtime.app_file_pick_pass(now + 1).await;
    let still_shown = shown(&runtime);
    if still_shown {
        runtime
            .timeout_runtime_interaction(&session_id, "app_file_pick_file-pick-ended")
            .await
            .unwrap();
    }
    assert!(!still_shown, "an ended request no longer asks the owner");
}
