use super::*;
use std::path::Path;
use std::process::Command;

#[test]
fn room_environment_worker_session_alias_attaches_default_agent_to_slice() {
    run_test(session_alias_attaches_default_agent_to_slice);
}

async fn session_alias_attaches_default_agent_to_slice() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let created = dispatch_json(
        &fixture.home,
        json!({"CreateSession": {
            "workspace_id":"workspace", "worktree_id":"worktree",
            "agent_defaults":{"provider":"managed-dev-stub", "model":"default"},
            "kernel_ref":"desktop-worker", "worktree_placement":placement
        }}),
    )
    .await
    .expect("create a Room through its known slice worker alias");
    let session_id = created["SessionCreated"]["session"]["id"].as_str().unwrap();
    let agent = &created["SessionCreated"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    let attached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    let other_claim = dispatch_json(&fixture.home, bind(&fixture.rooms[1], "desktop")).await;
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":session_id,"agent_id":agent_id}}),
    )
    .await
    .expect("delete the Room's leased default agent");
    fixture.stop().await;
    assert_eq!(
        agent["remote_execution"]["worker_kernel_id"],
        "environment-worker"
    );
    assert_eq!(
        attached["Slice"]["slice"]["agent_ids"],
        json!([agent_id]),
        "session creation must preserve the canonical slice attachment"
    );
    assert!(
        other_claim.is_err(),
        "another Room cannot claim the occupied slice"
    );
}

#[test]
fn room_environment_standard_worker_does_not_infer_project_transfer() {
    run_test(standard_worker_does_not_infer_project_transfer);
}

async fn standard_worker_does_not_infer_project_transfer() {
    let _env_guard = crate::env_lock::lock();
    let mut fixture = LiveWorker::start_with_fresh_worker_identity().await;
    let source_repository = fixture.home_state.root.join("selected-repository");
    let supporting_directory = fixture.home_state.root.join("selected-directory");
    let fresh_worker = fixture._worker_state.root.join("fresh-worker");
    let target_worktree = fresh_worker.join("primary-worktree");
    let unrelated_worker_state = fixture._worker_state.root.join("unrelated.txt");
    let source_repository_id = source_repository.display().to_string();
    let supporting_directory_id = supporting_directory.display().to_string();
    let target_worktree_id = target_worktree.display().to_string();
    let persisted_pairing_path = fixture
        .home_state
        .root
        .join("ambient-home")
        .join("daemon")
        .join("config.json");

    init_test_repository(&source_repository, "selected.txt", "selected source\n");
    std::fs::create_dir_all(&supporting_directory).expect("selected directory should exist");
    std::fs::write(
        supporting_directory.join("supporting.txt"),
        "selected supporting source\n",
    )
    .expect("selected non-Git file should exist");
    std::fs::write(&unrelated_worker_state, "untouched worker state\n")
        .expect("unrelated worker state should exist");
    assert!(
        !target_worktree.exists(),
        "the explicit worker destination must start absent"
    );

    // LiveWorker is an ordinary relay-connected worker: it has no disposable-worker receipt,
    // Cloud allocation binding, or confirmed managed-kernel context registration. Keep this
    // guard on the public path so a future materialization hook cannot silently change the
    // standard manually-managed worker contract.
    {
        let worker = fixture.worker.app.lock().await;
        assert!(
            worker.managed_kernel_registration().is_none(),
            "the standard worker fixture must not masquerade as a managed allocation"
        );
        assert_eq!(
            worker
                .config_projection_store()
                .snapshot()
                .kernel_runtime_role,
            crate::config::KernelRuntimeRole::General
        );
    }

    // Seed a named Project and select both a Git repository and a supported plain directory
    // through the public home API. CreateSession has no separate source-worktree field: the
    // workspace_id below proves Project membership, while worktree_id is the destination sent
    // to the worker. The destination must not be inferred as a source transfer request.
    let seeded = dispatch_json(
        &fixture.home,
        json!({"CreateSession": {
            "workspace_id":source_repository_id.clone(),
            "worktree_id":source_repository_id.clone(),
            "project_selection":{"kind":"new"},
            "agent_defaults":{"provider":"managed-dev-stub", "model":"default"}
        }}),
    )
    .await
    .expect("seed a named Project through the public home API");
    let seed_session_id = seeded["SessionCreated"]["session"]["id"]
        .as_str()
        .expect("seed session id");
    let seed_agent_id = seeded["SessionCreated"]["agent"]["id"]
        .as_str()
        .expect("seed agent id");
    let project_id = seeded["SessionCreated"]["session"]["project_id"]
        .as_str()
        .expect("seed project id")
        .to_string();
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent": {
            "session_id":seed_session_id,
            "agent_id":seed_agent_id
        }}),
    )
    .await
    .expect("remove the seed agent before the remote public path");

    let updated_project = dispatch_json(
        &fixture.home,
        json!({"UpdateProjectWorkspaces": {
            "project_id":project_id.clone(),
            "workspace_ids":[source_repository_id.clone(), supporting_directory_id.clone()]
        }}),
    )
    .await
    .expect("select the Git and non-Git workspaces through the public API");
    assert_eq!(
        updated_project["ProjectWorkspacesUpdated"]["project"]["workspace_ids"],
        json!([source_repository_id, supporting_directory_id])
    );

    // The public request reaches the ordinary worker's real lease path. Since no managed
    // enrollment/context-transfer operation exists in this topology, the missing destination is
    // rejected rather than copied from the home Project. This is the preservation checkpoint;
    // the managed-disposable positive regression remains blocked on its real enrollment seam.
    let error = dispatch_json(
        &fixture.home,
        json!({"CreateSession": {
            "workspace_id":source_repository_id.clone(),
            "worktree_id":target_worktree_id.clone(),
            "project_selection":{"kind":"existing", "project_id":project_id.clone()},
            "agent_defaults":{"provider":"managed-dev-stub", "model":"default"},
            "kernel_ref":"desktop-worker"
        }}),
    )
    .await
    .expect_err("ordinary worker must reject an absent destination");
    assert!(
        error.to_string().contains("remote working directory")
            && error.to_string().contains("does not exist"),
        "the real worker path must reject the destination, not invent a transfer: {error}"
    );
    assert!(
        !target_worktree.exists(),
        "ordinary worker must not create a destination from home state"
    );
    assert_eq!(
        std::fs::read_to_string(source_repository.join("selected.txt"))
            .expect("read selected source"),
        "selected source\n"
    );
    assert_eq!(
        std::fs::read_to_string(&unrelated_worker_state).expect("read unrelated worker state"),
        "untouched worker state\n"
    );
    fixture.stop().await;
    assert!(
        crate::config::DaemonConfig::default_daemon_config_path() == persisted_pairing_path
            && persisted_pairing_path.exists(),
        "fresh worker pairing must be scoped below the disposable home fixture"
    );
    drop(fixture);
    assert!(
        !persisted_pairing_path.exists(),
        "fresh worker pairing must be removed with the disposable fixture"
    );
}

fn init_test_repository(root: &Path, filename: &str, contents: &str) {
    std::fs::create_dir_all(root).expect("test repository should exist");
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.email", "chariox@example.test"]);
    run_git(root, &["config", "user.name", "Chariox Test"]);
    std::fs::write(root.join(filename), contents).expect("test repository file should exist");
    run_git(root, &["add", filename]);
    run_git(root, &["commit", "-m", "selected source"]);
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git should be available for the placement regression");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
