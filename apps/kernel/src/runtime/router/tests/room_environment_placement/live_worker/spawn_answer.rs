//! The answer to `agent spawn --slice` names the folder the worker runs the agent in,
//! not the home Room's. Real relay and worker kernel with the managed-dev-stub provider.
use super::*;
use futures_util::FutureExt;

#[test]
fn a_slice_spawn_answer_names_the_worker_folder_not_the_home_room_folder() {
    run_test(slice_spawn_answer_names_the_worker_folder);
}

async fn slice_spawn_answer_names_the_worker_folder() {
    let mut fixture = LiveWorker::start_configured_with_home_vault_and_worker_id(
        false,
        false,
        None,
        false,
        "unused-until-public-slice-creation".into(),
        true,
        true,
        None,
    )
    .await;
    let result = std::panic::AssertUnwindSafe(check_spawn_answer(&fixture))
        .catch_unwind()
        .await;
    for app in [&fixture.worker.app, &fixture.home.app] {
        app.lock()
            .await
            .teardown_provider_processes(Some("managed-dev-stub"), true)
            .expect("clean fixture-owned provider processes");
    }
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_spawn_answer(fixture: &LiveWorker) {
    let room = &fixture.rooms[0];
    dispatch_json(&fixture.home, bind(room, "desktop"))
        .await
        .unwrap();
    fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .set_status(
            "desktop",
            SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .unwrap();
    let worker_folder = fixture._worker_state.worktree.path().display().to_string();
    let home_folder = fixture.home_state.worktree.path().display().to_string();
    assert_ne!(worker_folder, home_folder);
    let token = {
        let mut app = fixture.home.app.lock().await;
        let spawner = spawn_test_agent(&mut app, room, "spawner", "dev-stub");
        let spawner = app
            .agents_mut()
            .activate_agent_meta_mode(spawner.id(), None)
            .unwrap();
        launch_test_provider(&mut app, room, spawner.id(), "dev-stub", "dev-stub", "test")
            .runtime_mcp_auth_token()
            .unwrap()
            .to_string()
    };

    let spawned = fixture
        .home
        .dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            json!({"command": "agent spawn kid --provider managed-dev-stub --slice desktop"}),
        )
        .await
        .expect("the slice spawn dispatches");
    assert!(spawned.ok, "{:?}", spawned.payload);

    let answer = &spawned.payload["response"]["agent"];
    let stored = fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .get_agent(answer["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        stored
            .remote_execution()
            .and_then(|binding| binding.worker_worktree_id.as_deref()),
        Some(worker_folder.as_str()),
        "the binding holds the worker's folder"
    );
    assert_eq!(answer["worktree"], worker_folder, "{answer}");
    assert_eq!(answer["branch"], serde_json::Value::Null, "{answer}");
}
