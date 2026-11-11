//! The answer to `agent spawn --slice` names the folder the worker runs the agent in,
//! not the home Room's. Real relay and worker kernel with the managed-dev-stub provider.
use super::*;
use futures_util::FutureExt;

#[test]
fn a_slice_spawn_answer_names_the_worker_folder_not_the_home_room_folder() {
    run_test(slice_spawn_answer_names_the_worker_folder);
}

#[test]
fn a_slice_worktree_spawn_answer_names_the_checkout_the_worker_created() {
    run_test(slice_worktree_spawn_answer_names_the_created_checkout);
}

#[test]
fn a_kernel_spawn_without_a_folder_runs_in_the_worker_folder_not_the_parent_home_folder() {
    run_test(kernel_spawn_without_a_folder_runs_in_the_worker_folder);
}

async fn start_fixture() -> LiveWorker {
    LiveWorker::start_configured_with_home_vault_and_worker_id(
        false,
        false,
        None,
        false,
        "unused-until-public-slice-creation".into(),
        true,
        true,
        None,
    )
    .await
}

async fn finish(mut fixture: LiveWorker, result: std::thread::Result<()>) {
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

async fn slice_spawn_answer_names_the_worker_folder() {
    let fixture = start_fixture().await;
    let result = std::panic::AssertUnwindSafe(check_spawn_answer(&fixture))
        .catch_unwind()
        .await;
    finish(fixture, result).await;
}

async fn kernel_spawn_without_a_folder_runs_in_the_worker_folder() {
    let mut fixture = start_fixture().await;
    let (_agent_worker_state, agent_worker) =
        super::room_remote_agent_home_slice::start_remote_agent_worker(&mut fixture).await;
    super::room_remote_agent_home_slice::wait_for_remote_agent_worker(&fixture).await;
    let result = std::panic::AssertUnwindSafe(check_kernel_spawn_folder(&fixture, &agent_worker))
        .catch_unwind()
        .await;
    agent_worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("clean the agent worker's provider processes");
    finish(fixture, result).await;
}

async fn slice_worktree_spawn_answer_names_the_created_checkout() {
    let mut fixture = start_fixture().await;
    let checkout = fixture.placement()["target_directory"]
        .as_str()
        .expect("the fixture checkout path")
        .to_string();
    let result = std::panic::AssertUnwindSafe(check_created_checkout_answer(&fixture, &checkout))
        .catch_unwind()
        .await;
    finish(fixture, result).await;
}

/// Binds the slice to the first Room and spawns `kid` with the command `spawn_command` builds
/// from the slice's worker kernel ref. Returns the answer's agent and the stored agent.
async fn spawn_kid(
    fixture: &LiveWorker,
    spawn_command: impl FnOnce(&str) -> String,
) -> (serde_json::Value, crate::agent::AgentInstance) {
    let room = &fixture.rooms[0];
    dispatch_json(&fixture.home, bind(room, "desktop"))
        .await
        .unwrap();
    let worker_ref = {
        let app = fixture.home.app.lock().await;
        app.slices()
            .set_status(
                "desktop",
                SliceStatus::Running,
                crate::session::unix_epoch_ms(),
            )
            .unwrap();
        app.slices().resolve("desktop").unwrap().worker_kernel_ref
    };
    spawn_from_room(fixture, &spawn_command(&worker_ref), None).await
}

/// A Room agent of the first Room, working in `spawner_folder` when given, runs `command`
/// through the room tool. Returns the answer's agent and the stored agent.
async fn spawn_from_room(
    fixture: &LiveWorker,
    command: &str,
    spawner_folder: Option<&str>,
) -> (serde_json::Value, crate::agent::AgentInstance) {
    let room = &fixture.rooms[0];
    let token = {
        let mut app = fixture.home.app.lock().await;
        let spawner = match spawner_folder {
            Some(folder) => crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    crate::agent::CreateAgentRequest::new(room, "dev-stub")
                        .with_alias("spawner")
                        .with_worktree(folder),
                )
                .expect("agent should spawn"),
            None => spawn_test_agent(&mut app, room, "spawner", "dev-stub"),
        };
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
            json!({"command": command}),
        )
        .await
        .expect("the spawn dispatches");
    assert!(spawned.ok, "{:?}", spawned.payload);

    let answer = spawned.payload["response"]["agent"].clone();
    let stored = fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .get_agent(answer["id"].as_str().unwrap())
        .unwrap();
    (answer, stored)
}

async fn check_spawn_answer(fixture: &LiveWorker) {
    let worker_folder = fixture._worker_state.worktree.path().display().to_string();
    let home_folder = fixture.home_state.worktree.path().display().to_string();
    assert_ne!(worker_folder, home_folder);

    let (answer, stored) = spawn_kid(fixture, |_| {
        "agent spawn kid --provider managed-dev-stub --slice desktop".to_string()
    })
    .await;

    assert_eq!(
        stored
            .remote_execution()
            .and_then(|binding| binding.worker_worktree_id.as_deref()),
        Some(worker_folder.as_str()),
        "the binding holds the worker's folder"
    );
    assert_eq!(answer["worktree"], worker_folder, "{answer}");
    let facts = fixture
        .home
        .runtime_state
        .spawned_agent_facts(&stored)
        .await
        .unwrap();
    assert_eq!(
        serde_json::json!(facts.worktree),
        worker_folder,
        "the facts the local answer and the footer read name the worker's folder"
    );
    assert_eq!(answer["branch"], serde_json::Value::Null, "{answer}");
}

/// The parent's folder is a path on the home machine: a child on another kernel that names no
/// folder starts in a folder that kernel makes for it, as `/agent spawn --kernel` does.
async fn check_kernel_spawn_folder(fixture: &LiveWorker, agent_worker: &CommandRouter) {
    let home_folder = fixture.home_state.worktree.path().display().to_string();

    let (_, stored) = spawn_from_room(
        fixture,
        "agent spawn kid --provider managed-dev-stub --kernel agent-worker",
        Some(&home_folder),
    )
    .await;

    assert_ne!(
        stored
            .remote_execution()
            .and_then(|binding| binding.worker_worktree_id.as_deref()),
        Some(home_folder.as_str()),
        "the worker is not sent the parent's home folder"
    );
    let folders = crate::app::RemoteLeaseRuntime::new(&mut *agent_worker.app.lock().await)
        .leased_agent_working_directories();
    assert!(
        matches!(folders.as_slice(), [folder] if folder != &home_folder && folder.contains("/chariox-workspaces/agent-")),
        "the child runs in its own folder on the worker: {folders:?}"
    );
}

/// The slice's published folder is where the worker is told to start from; `--worktree` makes
/// the worker create a checkout of its own, and that checkout is where the agent runs.
async fn check_created_checkout_answer(fixture: &LiveWorker, checkout: &str) {
    let worker_folder = fixture._worker_state.worktree.path().display().to_string();
    let home_folder = fixture.home_state.worktree.path().display().to_string();
    assert_ne!(worker_folder, home_folder);
    assert_ne!(checkout, worker_folder);
    assert_ne!(checkout, home_folder);

    let (answer, stored) = spawn_kid(fixture, |worker_ref| {
        format!(
            "agent spawn kid --provider managed-dev-stub --kernel {worker_ref} \
             --worktree {checkout} --branch feature/kid"
        )
    })
    .await;

    assert!(
        std::path::Path::new(checkout).join(".git").exists(),
        "the worker created the checkout"
    );
    assert_eq!(
        stored
            .remote_execution()
            .and_then(|binding| binding.worker_worktree_id.as_deref()),
        Some(checkout),
        "the binding holds the checkout the worker created, not the folder it started from"
    );
    assert_eq!(answer["worktree"], checkout, "{answer}");
    let facts = fixture
        .home
        .runtime_state
        .spawned_agent_facts(&stored)
        .await
        .unwrap();
    assert_eq!(
        serde_json::json!(facts.worktree),
        checkout,
        "the facts the local answer and the footer read name the checkout the worker created"
    );
    assert_eq!(answer["branch"], serde_json::Value::Null, "{answer}");
}
