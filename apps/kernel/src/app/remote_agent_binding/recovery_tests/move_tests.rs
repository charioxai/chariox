use super::*;
#[tokio::test(flavor = "multi_thread")]
async fn mk03_public_move_materializes_current_project_and_return_uses_fresh_root() {
    let fixture = Fixture::with_relay_fixture(false, false, false, false).await;
    let (session_id, original, notes) = {
        let home = fixture.home.lock().await;
        let agent = home
            .agents()
            .clear_remote_execution(&fixture.agent_id)
            .unwrap();
        let session = home.sessions().get_session(agent.session_id()).unwrap();
        let original = PathBuf::from(session.worktree_id());
        let common = fixture.workspace_root.join("common");
        std::fs::create_dir_all(&common).unwrap();
        let git = |directory: &std::path::Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(directory)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Git fixture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&common, &["init", "--quiet"]);
        git(&common, &["config", "user.name", "Fixture"]);
        git(
            &common,
            &["config", "user.email", "fixture@example.invalid"],
        );
        std::fs::write(common.join("input"), "home before move\n").unwrap();
        git(&common, &["add", "input"]);
        git(&common, &["commit", "--quiet", "-m", "fixture"]);
        git(
            &common,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "linked",
                original.to_str().unwrap(),
            ],
        );
        home.agents()
            .update_agent_config(
                agent.id(),
                None,
                None,
                Some(Some(common.display().to_string())),
                Some(Some(original.display().to_string())),
            )
            .unwrap();
        std::fs::write(original.join("input"), "home before move\n").unwrap();
        let notes = fixture.workspace_root.join("notes");
        std::fs::create_dir_all(notes.join("nested")).unwrap();
        std::fs::write(notes.join("nested/document"), "support before move\n").unwrap();
        home.sessions
            .write()
            .update_project_workspaces(
                session.project_id(),
                vec![
                    session.workspace_id().to_string(),
                    notes.display().to_string(),
                ],
                agent.owner_user_id(),
            )
            .unwrap();
        (session.id().to_string(), original, notes)
    };
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::clone(&fixture.home),
        8,
    );
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session_id.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let envelope = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-forward",
        None,
        None,
        &request,
    );
    let moved = router.dispatch(envelope, request).await.unwrap();
    assert!(matches!(
        moved,
        crate::local::LocalDaemonResponse::AgentMovedToRemote { .. }
    ));
    let (target_path, target_notes) = {
        let worker = fixture.workers[0].lock().await;
        let home = fixture.home.lock().await;
        let moved = home.agents().get_agent(&fixture.agent_id).unwrap();
        let remote = moved.remote_execution().unwrap();
        let lease = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        let backing = worker.agents().get_agent(&lease.backing_agent_id).unwrap();
        let session = worker.sessions().get_session(backing.session_id()).unwrap();
        let path = PathBuf::from(backing.worktree_id().unwrap_or(session.worktree_id()));
        assert_ne!(
            path, original,
            "Move must use the imported receipt, not the source path"
        );
        assert_eq!(
            std::fs::read(path.join("input")).unwrap(),
            b"home before move\n"
        );
        let project = worker.sessions().get_project(session.project_id()).unwrap();
        let supporting = project
            .workspace_ids()
            .iter()
            .map(PathBuf::from)
            .find(|path| path.join("nested/document").is_file())
            .expect("directory workspace must remain a Project member");
        (path, supporting)
    };
    std::fs::write(target_path.join("input"), "latest target work\n").unwrap();
    std::fs::write(
        target_notes.join("nested/document"),
        "latest target notes\n",
    )
    .unwrap();
    std::fs::write(original.join("input"), "divergent home work\n").unwrap();
    let request =
        crate::local::LocalDaemonRequest::MoveAgentToLocal(crate::local::MoveAgentToLocalRequest {
            session_id: session_id.clone(),
            agent_ref: fixture.agent_id.clone(),
        });
    let envelope = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-return",
        None,
        None,
        &request,
    );
    let returned = router.dispatch(envelope, request).await.unwrap();
    assert!(matches!(
        returned,
        crate::local::LocalDaemonResponse::AgentMovedToLocal { .. }
    ));
    let home = fixture.home.lock().await;
    let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    let returned_path = PathBuf::from(agent.worktree_id().expect("returned execution workspace"));
    assert_ne!(returned_path, original);
    assert_ne!(returned_path, target_path);
    assert_eq!(
        std::fs::read(returned_path.join("input")).unwrap(),
        b"latest target work\n"
    );
    assert_eq!(
        std::fs::read(original.join("input")).unwrap(),
        b"divergent home work\n"
    );
    assert_eq!(
        std::fs::read(notes.join("nested/document")).unwrap(),
        b"support before move\n"
    );
}

async fn crash_fixture() -> (Fixture, String, PathBuf) {
    let fixture = Fixture::with_relay_fixture(false, false, false, false).await;
    let (session, path) = {
        let home = fixture.home.lock().await;
        let agent = home
            .agents()
            .clear_remote_execution(&fixture.agent_id)
            .unwrap();
        let session = home.sessions().get_session(agent.session_id()).unwrap();
        let path = PathBuf::from(session.worktree_id());
        std::fs::write(path.join("input"), "checkpointed workspace\n").unwrap();
        home.save_durable_state_snapshot().unwrap();
        (session.id().to_string(), path)
    };
    (fixture, session, path)
}

async fn move_request(
    fixture: &Fixture,
    session: &str,
    forward: bool,
) -> crate::local::LocalDaemonResponse {
    let request = if forward {
        crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id: session.into(),
                agent_ref: fixture.agent_id.clone(),
                machine_ref: fixture.presences[0].machine_id.clone(),
            },
        )
    } else {
        crate::local::LocalDaemonRequest::MoveAgentToLocal(crate::local::MoveAgentToLocalRequest {
            session_id: session.into(),
            agent_ref: fixture.agent_id.clone(),
        })
    };
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let envelope = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-crash",
        None,
        None,
        &request,
    );
    router.dispatch(envelope, request).await.unwrap()
}

async fn reboot_app(app: &Arc<Mutex<DaemonApp>>) {
    let mut app = app.lock().await;
    let placeholder = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let old = std::mem::replace(&mut *app, placeholder);
    let config = old.config.clone();
    drop(old);
    *app = DaemonApp::bootstrap(config).expect("kernel must reopen its durable Move records");
}

async fn reboot_worker(fixture: &mut Fixture) {
    fixture.shutdown[0].send(true).unwrap();
    fixture.tasks[1].abort();
    let _ = (&mut fixture.tasks[1]).await;
    reboot_app(&fixture.workers[0]).await;
    let state = fixture.workers[0].lock().await.relay_client_state();
    let (tx, rx) = watch::channel(false);
    fixture.shutdown[0] = tx;
    fixture.tasks[1] = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
        fixture.workers[0].clone(),
        state,
        rx,
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;
}

async fn crash_boundary(phase: &'static str, forward: bool) {
    let (mut fixture, session, original) = crash_fixture().await;
    if !forward {
        move_request(&fixture, &session, true).await;
    }
    let expected = {
        let home = fixture.home.lock().await;
        let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
        if let Some(remote) = agent.remote_execution() {
            let worker = fixture.workers[0].lock().await;
            let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
            let agent = worker
                .agents()
                .get_agent(&backing.backing_agent_id)
                .unwrap();
            let path = PathBuf::from(agent.worktree_id().unwrap());
            std::fs::write(path.join("input"), "current target workspace\n").unwrap();
            b"current target workspace\n".to_vec()
        } else {
            b"checkpointed workspace\n".to_vec()
        }
    };
    let journal_root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&journal_root).unwrap();
    std::fs::write(journal_root.join("crash-after-phase-for-test"), phase).unwrap();
    let task_home = fixture.home.clone();
    let request = if forward {
        crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id: session.clone(),
                agent_ref: fixture.agent_id.clone(),
                machine_ref: fixture.presences[0].machine_id.clone(),
            },
        )
    } else {
        crate::local::LocalDaemonRequest::MoveAgentToLocal(crate::local::MoveAgentToLocalRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
        })
    };
    let crashed = tokio::spawn(async move {
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(task_home, 8);
        let envelope = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-injected-crash",
            None,
            None,
            &request,
        );
        router.dispatch(envelope, request).await
    })
    .await;
    match crashed {
        Err(error) if error.is_panic() => (),
        Ok(Err(error)) => panic!("Move failed before {phase}: {error}"),
        _ => panic!("the public move must reach {phase}"),
    }
    reboot_app(&fixture.home).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    // Bound exercises a lost unacknowledged lease. Committed exercises the
    // durable target backing, with its original caller and lease timestamps.
    if forward && matches!(phase, "bound" | "committed") {
        reboot_worker(&mut fixture).await;
    }
    let operation = move_journal(&fixture, &session).await;
    assert_eq!(
        operation["phase"]["phase"].as_str() == Some("complete"),
        phase == "complete"
    );
    if phase != "complete" {
        let home = fixture.home.lock().await;
        let error = home
            .providers()
            .launch_run_detached(
                crate::provider::LaunchProviderRequest::new(
                    &session, "dev-stub", "dev-stub", "default", "default",
                )
                .with_agent_id(&fixture.agent_id),
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Agent move holds provider admission"));
    }
    move_request(&fixture, &session, forward).await;
    let operation = move_journal(&fixture, &session).await;
    let home = fixture.home.lock().await;
    assert_eq!(operation["phase"]["phase"], "complete");
    assert!(
        home.providers().list_runs().is_empty(),
        "recovery cannot create a provider run"
    );
    let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    if forward {
        let worker = fixture.workers[0].lock().await;
        let remote = agent.remote_execution().unwrap();
        let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        let target = worker
            .agents()
            .get_agent(&backing.backing_agent_id)
            .unwrap();
        assert_eq!(
            std::fs::read(PathBuf::from(target.worktree_id().unwrap()).join("input")).unwrap(),
            expected
        );
        assert!(worker.providers().list_runs().is_empty());
    } else {
        let path = PathBuf::from(agent.worktree_id().unwrap());
        assert_ne!(path, original);
        assert_eq!(std::fs::read(path.join("input")).unwrap(), expected);
        assert_eq!(
            std::fs::read(original.join("input")).unwrap(),
            b"checkpointed workspace\n"
        );
    }
}

macro_rules! move_crash_test {
    ($name:ident, $phase:literal, $forward:literal) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $name() {
            crash_boundary($phase, $forward).await;
        }
    };
}
move_crash_test!(mk03_forward_restart_fenced, "fenced", true);
move_crash_test!(mk03_forward_restart_source_stopped, "source_stopped", true);
move_crash_test!(mk03_forward_restart_captured, "captured", true);
move_crash_test!(mk03_forward_restart_materialized, "materialized", true);
move_crash_test!(
    mk03_forward_restart_bound_with_worker_restart,
    "bound",
    true
);
move_crash_test!(
    mk03_forward_restart_committed_with_worker_restart,
    "committed",
    true
);
move_crash_test!(mk03_return_restart_fenced, "fenced", false);
move_crash_test!(mk03_return_restart_captured, "captured", false);
move_crash_test!(mk03_return_restart_materialized, "materialized", false);
move_crash_test!(mk03_return_restart_bound, "bound", false);
move_crash_test!(mk03_return_restart_committed, "committed", false);

move_crash_test!(mk03_forward_restart_complete, "complete", true);
move_crash_test!(mk03_return_restart_complete, "complete", false);

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn mk03_failed_snapshot_rolls_back_without_binding_or_changing_source() {
    let (fixture, session, original) = crash_fixture().await;
    std::fs::remove_file(original.join("input")).unwrap();
    let outside = fixture.workspace_root.join("outside");
    std::fs::write(&outside, b"preserve outside").unwrap();
    std::os::unix::fs::symlink(&outside, original.join("input")).unwrap();
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let envelope = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-bad-source",
        None,
        None,
        &request,
    );
    assert!(router.dispatch(envelope, request).await.is_err());
    let operation = move_journal(&fixture, &session).await;
    let home = fixture.home.lock().await;
    assert_eq!(operation["phase"]["phase"], "rolled_back");
    assert!(home
        .agents()
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .is_none());
    assert_eq!(std::fs::read(outside).unwrap(), b"preserve outside");
    assert!(fixture.workers[0].lock().await.leased_agents.is_empty());
}

async fn move_journal(fixture: &Fixture, session: &str) -> serde_json::Value {
    let root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".home.json"))
        .map(|entry| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(entry.path()).unwrap())
                .unwrap()
        })
        .filter(|record| {
            record["identity"]["home_session_id"] == session
                && record["identity"]["home_agent_id"] == fixture.agent_id
        })
        .max_by_key(|record| record["identity"]["generation"].as_u64().unwrap())
        .expect("public Move must persist a generation")
}
