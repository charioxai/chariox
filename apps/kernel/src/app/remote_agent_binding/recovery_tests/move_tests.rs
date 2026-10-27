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
            abort_pending: false,
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
            abort_pending: false,
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
    // A crash may already have closed the worker connector. It is joined below.
    let _ = fixture.shutdown[0].send(true);
    fixture.tasks[1].abort();
    // An offline-peer cell may already have joined this connector.
    if !fixture.tasks[1].is_finished() {
        let _ = (&mut fixture.tasks[1]).await;
    }
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
            abort_pending: false,
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
    // Reopen both roles at every home boundary: target on Move, source on Return.
    reboot_worker(&mut fixture).await;
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

// MP-08 / MP-10 / MP-11: fail between a role's durable write and its acknowledgement.
async fn crash_role_boundary(phase: &'static str, forward: bool, worker: bool) {
    let (mut fixture, session, original) = crash_fixture().await;
    if !forward {
        move_request(&fixture, &session, true).await;
    }
    let source = if forward {
        original.clone()
    } else {
        let home = fixture.home.lock().await;
        let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
        let remote = agent.remote_execution().unwrap();
        let worker = fixture.workers[0].lock().await;
        let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        PathBuf::from(
            worker
                .agents()
                .get_agent(&backing.backing_agent_id)
                .unwrap()
                .worktree_id()
                .unwrap(),
        )
    };
    std::fs::write(source.join("input"), "latest source boundary\n").unwrap();
    let crash_app = if worker {
        &fixture.workers[0]
    } else {
        &fixture.home
    };
    let journal = crash_app
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&journal).unwrap();
    let marker = journal.join("crash-after-phase-for-test");
    std::fs::write(&marker, phase).unwrap();
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
            abort_pending: false,
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
        })
    };
    let home = fixture.home.clone();
    let task = tokio::spawn(async move {
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8);
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-role-crash",
            None,
            None,
            &request,
        );
        router.dispatch(command, request).await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while marker.exists() {
            assert!(
                !task.is_finished(),
                "public Move ended before role boundary {phase}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("public Move must reach its durable role boundary");
    // The home request loses its peer reply when the other process crashes.
    task.abort();
    let crashed = task.await;
    let before = move_journal(&fixture, &session).await;
    let operation_id = before["identity"]["operation_id"].clone();
    let generation = before["identity"]["generation"].clone();
    assert!(
        !marker.exists(),
        "public Move must reach role boundary {phase}"
    );
    assert!(
        !matches!(crashed, Ok(Ok(_))),
        "role crash must interrupt its acknowledgement: {phase}"
    );
    reboot_app(&fixture.home).await;
    reboot_worker(&mut fixture).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    move_request(&fixture, &session, forward).await;
    let operation = move_journal(&fixture, &session).await;
    assert_eq!(operation["phase"]["phase"], "complete", "{phase}");
    assert_eq!(
        operation["identity"]["operation_id"], operation_id,
        "{phase} must retain its capture"
    );
    assert_eq!(operation["identity"]["generation"], generation);
    let home = fixture.home.lock().await;
    assert!(home.providers().list_runs().is_empty());
    let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    let path = if forward {
        let worker = fixture.workers[0].lock().await;
        assert!(worker.providers().list_runs().is_empty());
        let remote = agent.remote_execution().unwrap();
        let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        PathBuf::from(
            worker
                .agents()
                .get_agent(&backing.backing_agent_id)
                .unwrap()
                .worktree_id()
                .unwrap(),
        )
    } else {
        PathBuf::from(agent.worktree_id().unwrap())
    };
    assert_ne!(path, source);
    assert_eq!(
        std::fs::read(path.join("input")).unwrap(),
        b"latest source boundary\n",
        "{phase}"
    );
    assert_eq!(
        std::fs::read(source.join("input")).unwrap(),
        b"latest source boundary\n"
    );
}

macro_rules! role_crash_test {
    ($name:ident, $phase:literal, $forward:literal, $worker:literal) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $name() {
            crash_role_boundary($phase, $forward, $worker).await;
        }
    };
}
role_crash_test!(
    mk03_forward_source_restart_prepared,
    "source:prepared",
    true,
    false
);
role_crash_test!(
    mk03_forward_source_restart_stopped,
    "source:stopped",
    true,
    false
);
role_crash_test!(
    mk03_forward_source_restart_captured,
    "source:captured",
    true,
    false
);
role_crash_test!(
    mk03_forward_source_restart_retired,
    "source:retired",
    true,
    false
);
role_crash_test!(
    mk03_forward_target_restart_reserved,
    "target:reserved",
    true,
    true
);
role_crash_test!(
    mk03_forward_target_restart_materialized,
    "target:materialized",
    true,
    true
);
role_crash_test!(
    mk03_forward_target_restart_attached,
    "target:attached",
    true,
    true
);
role_crash_test!(
    mk03_forward_target_restart_ready,
    "target:ready",
    true,
    true
);
role_crash_test!(
    mk03_forward_target_restart_active,
    "target:active",
    true,
    true
);
role_crash_test!(
    mk03_return_source_restart_prepared,
    "source:prepared",
    false,
    true
);
role_crash_test!(
    mk03_return_source_restart_stopped,
    "source:stopped",
    false,
    true
);
role_crash_test!(
    mk03_return_source_restart_captured,
    "source:captured",
    false,
    true
);
role_crash_test!(
    mk03_return_source_restart_retired,
    "source:retired",
    false,
    true
);
role_crash_test!(
    mk03_return_target_restart_reserved,
    "target:reserved",
    false,
    false
);
role_crash_test!(
    mk03_return_target_restart_materialized,
    "target:materialized",
    false,
    false
);
role_crash_test!(
    mk03_return_target_restart_attached,
    "target:attached",
    false,
    false
);
role_crash_test!(
    mk03_return_target_restart_ready,
    "target:ready",
    false,
    false
);
role_crash_test!(
    mk03_return_target_restart_active,
    "target:active",
    false,
    false
);
role_crash_test!(
    mk03_return_source_target_retirement_restart,
    "target:retired",
    false,
    true
);

#[cfg(unix)]
async fn rollback_crash_boundary(phase: &'static str) {
    let (mut fixture, session, original) = crash_fixture().await;
    let outside = fixture.workspace_root.join("rollback-outside");
    std::fs::write(&outside, b"preserve outside").unwrap();
    std::fs::remove_file(original.join("input")).unwrap();
    std::os::unix::fs::symlink(&outside, original.join("input")).unwrap();
    let journal = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&journal).unwrap();
    let marker = journal.join("crash-after-phase-for-test");
    std::fs::write(&marker, phase).unwrap();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let home = fixture.home.clone();
    let retry = request.clone();
    let task = tokio::spawn(async move {
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8);
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-rollback-crash",
            None,
            None,
            &request,
        );
        router.dispatch(command, request).await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while marker.exists() {
            assert!(
                !task.is_finished(),
                "public Move ended before rollback boundary {phase}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("public Move must reach its durable rollback boundary");
    task.abort();
    let _ = task.await;
    reboot_app(&fixture.home).await;
    reboot_worker(&mut fixture).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-rollback-recovery",
        None,
        None,
        &retry,
    );
    assert!(router.dispatch(command, retry).await.is_err());
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "rolled_back"
    );
    {
        let home = fixture.home.lock().await;
        assert!(!home
            .agent_moves
            .dispatch_fenced(&session, &fixture.agent_id));
        assert!(home
            .agents()
            .get_agent(&fixture.agent_id)
            .unwrap()
            .remote_execution()
            .is_none());
        assert!(home.providers().list_runs().is_empty());
    }
    assert_eq!(std::fs::read_link(original.join("input")).unwrap(), outside);
    assert_eq!(std::fs::read(&outside).unwrap(), b"preserve outside");
    std::fs::remove_file(original.join("input")).unwrap();
    std::fs::write(original.join("input"), b"repaired source").unwrap();
    move_request(&fixture, &session, true).await;
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "complete"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn mk03_rollback_home_restart() {
    rollback_crash_boundary("rolled_back").await;
}
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn mk03_rollback_source_release_restart() {
    rollback_crash_boundary("source:released").await;
}

async fn captured_peer_failure_keeps_operation(phase: &'static str) {
    let (mut fixture, session, _) = crash_fixture().await;
    let root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), phase).unwrap();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let home = fixture.home.clone();
    let first = request.clone();
    let crashed = tokio::spawn(async move {
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8);
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-captured-peer-crash",
            None,
            None,
            &first,
        );
        router.dispatch(command, first).await
    })
    .await;
    assert!(
        crashed.is_err(),
        "Move must reach its durable home boundary"
    );
    let before = move_journal(&fixture, &session).await;
    assert_eq!(before["phase"]["phase"], phase);
    reboot_app(&fixture.home).await;
    let _ = fixture.shutdown[0].send(true);
    fixture.tasks[1].abort();
    let _ = (&mut fixture.tasks[1]).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-captured-peer-unavailable",
        None,
        None,
        &request,
    );
    assert!(router.dispatch(command, request).await.is_err());
    let pending = move_journal(&fixture, &session).await;
    assert_eq!(
        pending["phase"]["phase"], phase,
        "a lost peer cannot discard a captured Move"
    );
    assert_eq!(
        pending["identity"]["operation_id"],
        before["identity"]["operation_id"]
    );
    assert!(fixture
        .home
        .lock()
        .await
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
    reboot_worker(&mut fixture).await;
    move_request(&fixture, &session, true).await;
    let completed = move_journal(&fixture, &session).await;
    assert_eq!(completed["phase"]["phase"], "complete");
    assert_eq!(
        completed["identity"]["operation_id"],
        before["identity"]["operation_id"]
    );
    assert_eq!(
        completed["identity"]["generation"],
        before["identity"]["generation"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_captured_peer_disconnect_keeps_the_same_move() {
    captured_peer_failure_keeps_operation("captured").await;
}
#[tokio::test(flavor = "multi_thread")]
async fn mk03_materialized_peer_disconnect_keeps_the_same_move() {
    captured_peer_failure_keeps_operation("materialized").await;
}

// MP-08 / MP-09 / MP-10 / MP-11: reject unsupported history before stopping its run.
#[tokio::test(flavor = "multi_thread")]
async fn mk03_unsupported_history_refuses_before_fencing_or_stopping() {
    for (provider, interface) in [
        ("claude", "chariox"),
        ("opencode", "chariox"),
        ("codex", "native_tui"),
    ] {
        let (fixture, session, _) = crash_fixture().await;
        let run = {
            let home = fixture.home.lock().await;
            let run = home
                .providers()
                .launch_run_detached(
                    crate::provider::LaunchProviderRequest::new(
                        &session, "dev-stub", "dev-stub", "default", "default",
                    )
                    .with_agent_id(&fixture.agent_id),
                )
                .unwrap();
            let mut value = serde_json::to_value(&run).unwrap();
            value["provider"] = serde_json::json!(provider);
            value["adapter_key"] = serde_json::json!(provider);
            value["client_interface"] = serde_json::json!(interface);
            let mut run: crate::provider::RuntimeProviderRun =
                serde_json::from_value(value).unwrap();
            run.set_provider_session_id(Some("native-history".into()));
            home.providers().write().insert_run_for_test(run.clone());
            run
        };
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            fixture.home.clone(),
            8,
        );
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id: session.clone(),
                agent_ref: fixture.agent_id.clone(),
                machine_ref: fixture.presences[0].machine_id.clone(),
            },
        );
        let envelope = crate::runtime::command::KernelCommand::from_local_request(
            "unsupported-history",
            None,
            None,
            &request,
        );
        let error = router.dispatch(envelope, request).await.unwrap_err();
        assert!(
            error.to_string().contains("native checkpoint support")
                || error.to_string().contains("managed Chariox Codex"),
            "{error}"
        );
        let home = fixture.home.lock().await;
        assert_eq!(
            home.providers().get_run(run.id()).unwrap().state(),
            run.state(),
            "unsupported history must retain its usable provider run"
        );
        assert!(
            !home
                .agent_moves
                .dispatch_fenced(&session, &fixture.agent_id),
            "preflight refusal must not fence the agent"
        );
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: source failure must release both admission fences.
#[tokio::test(flavor = "multi_thread")]
async fn mk03_return_snapshot_failure_restores_remote_source_admission() {
    let (fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    let (remote, backing_id, backing_session, source_path) = {
        let home = fixture.home.lock().await;
        let remote = home
            .agents()
            .get_agent(&fixture.agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .clone();
        let worker = fixture.workers[0].lock().await;
        let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        let agent = worker
            .agents()
            .get_agent(&backing.backing_agent_id)
            .unwrap();
        (
            remote,
            agent.id().to_string(),
            agent.session_id().to_string(),
            PathBuf::from(agent.worktree_id().unwrap()),
        )
    };
    let outside = fixture.workspace_root.join("return-outside");
    std::fs::write(&outside, "preserve outside").unwrap();
    std::os::unix::fs::symlink(&outside, source_path.join("bad-link")).unwrap();
    let error = dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal": {"session_id": session, "agent_ref": fixture.agent_id}})).await.unwrap_err();
    assert!(error.to_string().contains("symbolic links"), "{error}");
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "rolled_back"
    );
    let home = fixture.home.lock().await;
    assert_eq!(
        home.agents()
            .get_agent(&fixture.agent_id)
            .unwrap()
            .remote_execution(),
        Some(&remote)
    );
    assert!(!home
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
    let worker = fixture.workers[0].lock().await;
    assert!(!worker
        .agent_moves
        .dispatch_fenced(&backing_session, &backing_id));
    worker
        .providers()
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                &backing_session,
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(&backing_id),
        )
        .expect("source admission must be restored after failed Return");
}

async fn dispatch_move_value(
    fixture: &Fixture,
    value: serde_json::Value,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(value).unwrap();
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk03-review",
        None,
        None,
        &request,
    );
    crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8)
        .dispatch(command, request)
        .await
}

// A worker retains post-turn resume state, but no provider process, across restart.
#[tokio::test(flavor = "multi_thread")]
async fn mk03_return_after_target_resume_state_and_restart_keeps_source_usable() {
    let (mut fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    {
        let operation = move_journal(&fixture, &session).await;
        let home = fixture.home.lock().await;
        let remote = home
            .agents()
            .get_agent(&fixture.agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .clone();
        let worker = fixture.workers[0].lock().await;
        let backing = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
        let mut agent = worker
            .agents()
            .get_agent(&backing.backing_agent_id)
            .unwrap();
        agent.set_provider_resume_state(
            crate::provider::ProviderResumeState::from_codex_thread_id(
                "00000000-0000-0000-0000-000000000008",
            ),
        );
        worker.agents().restore_agent(agent);
        worker
            .retain_agent_move_backing(
                operation["identity"]["operation_id"].as_str().unwrap(),
                &remote.leased_agent_id,
            )
            .unwrap();
    }
    reboot_worker(&mut fixture).await;
    let error = dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal": {"session_id": session, "agent_ref": fixture.agent_id}})).await.unwrap_err();
    assert!(error.to_string().contains("after restart"), "{error}");
    let home = fixture.home.lock().await;
    assert!(
        !home
            .agent_moves
            .dispatch_fenced(&session, &fixture.agent_id),
        "a restarted source must remain able to resume the missing provider run"
    );
}

async fn interrupt_public_move(fixture: &Fixture, session: &str, phase: &str) {
    let journal = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&journal).unwrap();
    std::fs::write(journal.join("crash-after-phase-for-test"), phase).unwrap();
    let home = fixture.home.clone();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.into(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let crashed = tokio::spawn(async move {
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-abort-boundary",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    match crashed {
        Err(error) if error.is_panic() => (),
        Ok(Err(error)) => panic!("Move failed before {phase}: {error}"),
        _ => panic!("Move must reach its requested durable boundary"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_abort_captured_move_releases_source_and_allows_new_generation() {
    let (fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "captured").await;
    let reply = dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal": {"session_id": session, "agent_ref": fixture.agent_id, "abort_pending": true}})).await.expect("explicit abort must release a captured operation before the target has a lease");
    assert!(serde_json::to_value(reply)
        .unwrap()
        .get("AgentMoveAborted")
        .is_some());
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "rolled_back"
    );
    assert!(!fixture
        .home
        .lock()
        .await
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
    move_request(&fixture, &session, true).await;
    assert_eq!(
        move_journal(&fixture, &session).await["identity"]["generation"],
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_abort_refuses_after_target_activation() {
    let (fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "bound").await;
    let operation = move_journal(&fixture, &session).await["identity"]["operation_id"]
        .as_str()
        .unwrap()
        .to_string();
    {
        // Lost activation reply: the target committed while the home journal is still Bound.
        let worker = fixture.workers[0].lock().await;
        let mut target = worker.agent_moves.target(&operation).unwrap();
        target.active = true;
        worker.agent_moves.save_target(target).unwrap();
    }
    let error = dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal": {"session_id": session, "agent_ref": fixture.agent_id, "abort_pending": true}})).await.unwrap_err();
    assert!(error.to_string().contains("activated"), "{error}");
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "bound"
    );
    {
        let worker = fixture.workers[0].lock().await;
        let mut target = worker.agent_moves.target(&operation).unwrap();
        target.active = false;
        worker.agent_moves.save_target(target).unwrap();
    }
    move_request(&fixture, &session, true).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: compatibility is checked on the encrypted peer path.
#[tokio::test(flavor = "multi_thread")]
async fn mk03_peer_probe_refuses_native_incompatibility_before_staging() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let (fixture, session, _) = crash_fixture().await;
    let config = fixture.home.lock().await.config.clone();
    let target = fixture.workers[0].lock().await.config.clone();
    let thumb = crate::runtime::terminal_pairings::public_key_thumbprint;
    let identity = serde_json::json!({"operation_id":"compatibility-probe", "generation":1,
        "home_kernel_id":config.daemon_id, "home_session_id":session, "home_agent_id":fixture.agent_id,
        "owner_user_id":"local", "source_kernel_id":config.daemon_id,"target_kernel_id":target.daemon_id,
        "home_key_thumbprint":thumb(&config.relay_public_key), "source_key_thumbprint":thumb(&config.relay_public_key),
        "target_key_thumbprint":thumb(&target.relay_public_key), "target_public_key":target.relay_public_key});
    let version = fixture.workspace_root.join("version-probe");
    std::fs::write(&version, "#!/bin/sh\nprintf 'codex-cli 0.0.0\\n'\n").unwrap();
    std::fs::set_permissions(&version, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &version);
    for (provider, interface, os, cli, reason) in [
        (
            "claude",
            "chariox",
            "linux",
            "codex-cli 0.159.3",
            "native checkpoint support",
        ),
        (
            "codex",
            "native_tui",
            "linux",
            "codex-cli 0.159.3",
            "managed Chariox Codex",
        ),
        (
            "codex",
            "chariox",
            "darwin",
            "codex-cli 0.159.3",
            "operating-system pair",
        ),
        (
            "codex",
            "chariox",
            "linux",
            "codex-cli 0.0.0",
            "CLI version",
        ),
        (
            "codex",
            "chariox",
            "linux",
            "codex-cli 0.159.3",
            "CLI version",
        ),
    ] {
        let request = serde_json::from_value(serde_json::json!({"action":"probe", "identity":identity,
            "native":{"provider":provider, "interface":interface, "source_os":os, "cli_version":cli, "account_sha256":"0".repeat(64)}})).unwrap();
        let error = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &config, ClientTarget {daemon_id:Some(target.daemon_id.clone()), daemon_alias:None},
            crate::transport::relay_peer::RelayPeerRequest::AgentMove {request}, Duration::from_secs(15),
        ).await.unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
        assert!(fixture.workers[0]
            .lock()
            .await
            .agent_moves
            .target("compatibility-probe")
            .is_none());
        assert!(!fixture
            .home
            .lock()
            .await
            .agent_moves
            .dispatch_fenced(&session, &fixture.agent_id));
    }
    std::env::remove_var("CHARIOX_CODEX_BIN");
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_abort_retires_empty_move_lease_and_rejects_delayed_creation() {
    let (fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "materialized").await;
    let operation = move_journal(&fixture, &session).await;
    let identity: crate::agent_move::AgentMoveIdentity =
        serde_json::from_value(operation["identity"].clone()).unwrap();
    let (lease, ordinary) = {
        let mut worker = fixture.workers[0].lock().await;
        let lease = worker
            .create_agent_move_lease(&identity, false, None)
            .unwrap();
        assert_eq!(
            worker
                .create_agent_move_lease(&identity, false, None)
                .unwrap()
                .id,
            lease.id,
            "lease preparation must be idempotent"
        );
        let ordinary = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .create_execution_lease(
                &identity.home_kernel_id,
                &session,
                &fixture.agent_id,
                false,
                &identity.owner_user_id,
            )
            .unwrap();
        (lease, ordinary)
    };
    dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id,"abort_pending":true}})).await.unwrap();
    {
        let worker = fixture.workers[0].lock().await;
        assert!(
            !worker.execution_leases.contains_key(&lease.id),
            "abort must retire its empty move lease"
        );
        assert!(
            worker.execution_leases.contains_key(&ordinary.id),
            "abort must preserve independent ordinary leases"
        );
    }
    move_request(&fixture, &session, true).await;
    let mut worker = fixture.workers[0].lock().await;
    let error = worker
        .create_agent_move_lease(&identity, false, None)
        .unwrap_err();
    assert!(
        error.to_string().contains("cancelled"),
        "a delayed create from the old generation must stay refused after a newer Move: {error}"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: ordinary prompts arriving behind the Move barrier resume automatically.
async fn queued_mid_move(abort: bool) {
    let (fixture, session, _) = crash_fixture().await;
    let attachment = {
        let mut home = fixture.home.lock().await;
        let configured = &home.config.user_config.credential_vault.path;
        let isolated_home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let vault = configured
            .strip_prefix("~/")
            .map(|path| isolated_home.join(path))
            .unwrap_or_else(|| PathBuf::from(configured));
        assert!(
            vault.starts_with(&isolated_home),
            "the queue fixture must use its isolated Vault"
        );
        crate::secret::create_chariox_encrypted_vault_for_test(&vault, "queue fixture").unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &vault,
            "queue fixture",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let mut agent = home.agents().get_agent(&fixture.agent_id).unwrap();
        agent.set_provider("dev-stub");
        agent.set_model(Some("queue-test-model".into()));
        agent.set_account_profile(None);
        home.agents().restore_agent(agent);
        crate::app::KernelSessionService::new(&mut home)
            .attach(crate::attachment::AttachRequest::new(
                &session,
                "move-queue-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap()
            .id()
            .to_string()
    };
    interrupt_public_move(&fixture, &session, "captured").await;
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        fixture.home.clone(),
        8,
    );
    async fn dispatch(
        router: &crate::runtime::router::CommandRouter,
        value: serde_json::Value,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let request: crate::local::LocalDaemonRequest = serde_json::from_value(value).unwrap();
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-queue",
            None,
            None,
            &request,
        );
        router.dispatch(command, request).await
    }
    // Match the live transport: one retained router and its independent readiness consumer.
    let output_runtime = router.runtime_state();
    let output_pump = tokio::spawn(async move { output_runtime.run_provider_output_pump().await });
    dispatch(&router, serde_json::json!({"SubmitPrompt": {"session_id": session, "attachment_id": attachment, "target_agent_id":fixture.agent_id, "prompt":"queued during Move"}})).await.unwrap();
    {
        let mut home = fixture.home.lock().await;
        // Dispatch peeking deliberately hides fenced work. Inspect the queue
        // itself so the assertion does not mistake that barrier for data loss.
        let room = home.sessions().get_session(&session).unwrap();
        assert_eq!(
            home.prompt_state_owner()
                .state_parts(&room, &fixture.agent_id)
                .1
                .len(),
            1
        );
        assert!(home
            .prompt_owner_active_prompt_for_agent(&session, &fixture.agent_id)
            .unwrap()
            .is_none());
    }
    if abort {
        dispatch(&router, serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id,"abort_pending":true}})).await.unwrap();
    } else {
        dispatch(&router, serde_json::json!({"MoveAgentToRemote":{"session_id":session,"agent_ref":fixture.agent_id,"machine_ref":fixture.presences[0].machine_id}})).await.unwrap();
    }
    let promoted = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (empty, remote, local_started) = {
                let home = fixture.home.lock().await;
                let room = home.sessions().get_session(&session).unwrap();
                let empty = home
                    .prompt_state_owner()
                    .state_parts(&room, &fixture.agent_id)
                    .1
                    .is_empty();
                let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
                (
                    empty,
                    agent.remote_execution().cloned(),
                    home.providers
                        .get_run_for_agent(&session, &fixture.agent_id)
                        .is_some(),
                )
            };
            let started = if let Some(remote) = remote {
                let worker = fixture.workers[0].lock().await;
                let lease = worker.leased_agents.get(&remote.leased_agent_id).unwrap();
                worker
                    .providers
                    .get_run_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
                    .is_some()
            } else {
                local_started
            };
            if empty && started {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    output_pump.abort();
    let _ = output_pump.await;
    let home = fixture.home.lock().await;
    let room = home.sessions().get_session(&session).unwrap();
    let notices: Vec<_> = home
        .load_session_history_entries(&room, None)
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::Notice)
        .map(|entry| entry.text)
        .collect();
    let latest = home
        .providers
        .get_latest_run_for_agent(&session, &fixture.agent_id)
        .map(|run| (run.provider().to_string(), run.state()));
    let fenced = home
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id);
    let preparing = home
        .prompt_state_owner
        .has_idle_queue_promotion(&session, &fixture.agent_id);
    let interactions: Vec<_> = room
        .active_interactions()
        .iter()
        .filter_map(|interaction| interaction.title().map(str::to_string))
        .collect();
    assert!(promoted.is_ok(), "a prompt queued during Move must advance when the operation settles; fenced={fenced}, preparing={preparing}, interactions={interactions:?}, run={latest:?}, notices={notices:?}");
}
#[tokio::test(flavor = "multi_thread")]
async fn mk03_prompt_queued_mid_move_advances_after_complete() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    queued_mid_move(false).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn mk03_prompt_queued_mid_move_advances_after_abort() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    queued_mid_move(true).await;
}

async fn slice_membership_survives_completion_crash(forward: bool) {
    let fixture = Fixture::with_relay_fixture(false, false, false, true).await;
    let session = {
        let home = fixture.home.lock().await;
        home.agents()
            .clear_remote_execution(&fixture.agent_id)
            .unwrap()
            .session_id()
            .to_string()
    };
    fixture
        .home
        .lock()
        .await
        .save_durable_state_snapshot()
        .unwrap();
    let forward_request = serde_json::json!({"MoveAgentToRemote":{"session_id":session,"agent_ref":fixture.agent_id,"machine_ref":format!("slice:{}", fixture.slices[0].name)}});
    if !forward {
        dispatch_move_value(&fixture, forward_request.clone())
            .await
            .unwrap();
    }
    let journal = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&journal).unwrap();
    std::fs::write(journal.join("crash-after-phase-for-test"), "complete").unwrap();
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(if forward {
        forward_request
    } else {
        serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id}})
    })
    .unwrap();
    let home = fixture.home.clone();
    let crashed = tokio::spawn(async move {
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-slice-completion-crash",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(crashed.is_err_and(|error| error.is_panic()));
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "complete"
    );
    reboot_app(&fixture.home).await;
    let home = fixture.home.lock().await;
    let slice = home.slices().resolve(&fixture.slices[0].id).unwrap();
    assert_eq!(
        slice.agent_ids.contains(&fixture.agent_id),
        forward,
        "slice membership must be durable at Complete, before the request handler returns"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_slice_attach_survives_completion_crash() {
    slice_membership_survives_completion_crash(true).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn mk03_slice_detach_survives_completion_crash() {
    slice_membership_survives_completion_crash(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_forward_commit_retires_home_desktop_grant() {
    let (fixture, session, _) = crash_fixture().await;
    {
        let home = fixture.home.lock().await;
        home.kernel_browser_host
            .grant_desktop("local", &fixture.agent_id, "test-surface");
        assert!(home
            .kernel_browser_host
            .grant_holders()
            .iter()
            .any(|(_, agent)| agent == &fixture.agent_id));
    }
    move_request(&fixture, &session, true).await;
    assert!(
        !fixture
            .home
            .lock()
            .await
            .kernel_browser_host
            .grant_holders()
            .iter()
            .any(|(_, agent)| agent == &fixture.agent_id),
        "moving execution must retire its home user-domain grant"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_return_commit_retires_mirrored_worker_run() {
    let (fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    let projected_id = {
        let home = fixture.home.lock().await;
        let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
        let remote = agent.remote_execution().unwrap();
        let id = crate::provider::projected_leased_provider_run_id(
            &remote.leased_agent_id,
            "worker-run",
        );
        let request = crate::provider::LaunchProviderRequest::new(
            &session, "dev-stub", "dev-stub", "default", "default",
        )
        .with_agent_id(&fixture.agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "mirrored worker".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        home.update_provider_run_projection(run);
        home.agents()
            .set_remote_execution_active_worker_provider_run_id(
                &fixture.agent_id,
                Some("worker-run".into()),
            )
            .unwrap();
        home.sessions
            .set_active_provider_run(&session, Some(id.clone()))
            .unwrap();
        id
    };
    move_request(&fixture, &session, false).await;
    let home = fixture.home.lock().await;
    assert_eq!(
        home.provider_run_projection_store()
            .get(&projected_id)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended,
        "Return must retire its mirrored worker projection before publishing local execution"
    );
    assert!(home
        .sessions
        .get_session(&session)
        .unwrap()
        .active_provider_run_id()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_abort_captured_move_when_target_disappears() {
    let (mut fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "captured").await;
    fixture.shutdown[0].send(true).unwrap();
    (&mut fixture.tasks[1]).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id,"abort_pending":true}}))).await
        .expect("a target with no possible lease cannot hold source cancellation open")
        .expect("cancellation before lease creation must not need the target online");
    assert!(serde_json::to_value(reply)
        .unwrap()
        .get("AgentMoveAborted")
        .is_some());
    assert!(!fixture
        .home
        .lock()
        .await
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_cancel_move_keeps_ordinary_lease_creation_available() {
    let (fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "materialized").await;
    dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id,"abort_pending":true}})).await.unwrap();
    let operation = move_journal(&fixture, &session).await;
    let mut worker = fixture.workers[0].lock().await;
    crate::app::RemoteLeaseRuntime::new(&mut worker)
        .create_execution_lease(
            operation["identity"]["home_kernel_id"].as_str().unwrap(),
            &session,
            &fixture.agent_id,
            false,
            operation["identity"]["owner_user_id"].as_str().unwrap(),
        )
        .expect("cancelling a Move must not disable ordinary leases for that agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_abort_recovers_after_source_release_before_terminal_write() {
    let (fixture, session, _) = crash_fixture().await;
    interrupt_public_move(&fixture, &session, "captured").await;
    let journal = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::write(
        journal.join("crash-after-phase-for-test"),
        "source:released",
    )
    .unwrap();
    let home = fixture.home.clone();
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(serde_json::json!({"MoveAgentToLocal":{"session_id":session,"agent_ref":fixture.agent_id,"abort_pending":true}})).unwrap();
    let crashed = tokio::spawn(async move {
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk03-cancel-crash",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(crashed.is_err_and(|error| error.is_panic()));
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "cancelling"
    );
    reboot_app(&fixture.home).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    let error = dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToRemote":{"session_id":session,"agent_ref":fixture.agent_id,"machine_ref":fixture.presences[0].machine_id}})).await.unwrap_err();
    assert!(
        error.to_string().contains("cancelled"),
        "restart must finish the user's cancellation: {error}"
    );
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "rolled_back"
    );
    assert!(!fixture
        .home
        .lock()
        .await
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
    move_request(&fixture, &session, true).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: repeated large Moves retain live checkouts, not transfer archives.
async fn assert_no_workspace_archives(fixture: &Fixture) {
    let roots = [
        fixture
            .home
            .lock()
            .await
            .config
            .private_runtime_state_root()
            .join("agent-moves"),
        fixture.workers[0]
            .lock()
            .await
            .config
            .private_runtime_state_root()
            .join("agent-moves"),
    ];
    for root in roots {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path().join("workspace.archive");
            assert!(
                !path.exists(),
                "completed Move retained workspace archive: {}",
                path.display()
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_repeated_large_project_moves_release_transfer_archives() {
    use rand::RngCore;
    let (fixture, session, original) = crash_fixture().await;
    let mut bytes = vec![0; 8 * 1024 * 1024];
    rand::thread_rng().fill_bytes(&mut bytes);
    for byte in &mut bytes {
        *byte = b'a' + *byte % 26;
    }
    std::fs::write(original.join("large-input"), &bytes).unwrap();
    for _ in 0..3 {
        move_request(&fixture, &session, true).await;
        {
            let home = fixture.home.lock().await;
            let operation = home
                .agent_moves
                .for_agent(&session, &fixture.agent_id)
                .unwrap();
            let worker = fixture.workers[0].lock().await;
            worker
                .agent_moves
                .write_chunk(
                    &operation.identity.operation_id,
                    crate::agent_move::AgentMoveArtifactKind::Workspace,
                    0,
                    b"retry",
                )
                .expect_err("an active target must refuse late chunks without recreating staging");
        }
        assert_no_workspace_archives(&fixture).await;
        move_request(&fixture, &session, false).await;
        assert_no_workspace_archives(&fixture).await;
        let home = fixture.home.lock().await;
        let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
        assert!(
            std::fs::read(PathBuf::from(agent.worktree_id().unwrap()).join("large-input")).unwrap()
                == bytes,
            "the large Project must survive every Move and Return"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mk03_restart_removes_completed_workspace_archives_but_keeps_native() {
    use crate::agent_move::AgentMoveArtifactKind;
    let (mut fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    let operation = fixture
        .home
        .lock()
        .await
        .agent_moves
        .for_agent(&session, &fixture.agent_id)
        .unwrap();
    let mut retained_native = Vec::new();
    // Recreate the crash window after the terminal record is synced but before unlink.
    for app in [&fixture.home, &fixture.workers[0]] {
        let app = app.lock().await;
        let store = &app.agent_moves;
        let path = store.artifact_path(
            &operation.identity.operation_id,
            AgentMoveArtifactKind::Workspace,
        );
        std::fs::write(path, b"archive left at crash").unwrap();
        let native = store.artifact_path(
            &operation.identity.operation_id,
            AgentMoveArtifactKind::Native,
        );
        std::fs::write(&native, b"native state must survive").unwrap();
        retained_native.push(native);
        app.save_durable_state_snapshot().unwrap();
    }
    reboot_app(&fixture.home).await;
    reboot_worker(&mut fixture).await;
    assert_no_workspace_archives(&fixture).await;
    for native in retained_native {
        assert_eq!(std::fs::read(native).unwrap(), b"native state must survive");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_missing_home_mcp_rejects_before_move_activation() {
    let (fixture, session, original) = crash_fixture().await;
    fixture
        .home
        .lock()
        .await
        .agents()
        .grant_mcp(&fixture.agent_id, "mk04-missing-package".into())
        .unwrap();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk04-missing",
        None,
        None,
        &request,
    );
    let result = router.dispatch(command, request).await;
    assert!(
        result.is_err(),
        "Move must reject a missing granted home MCP before activation"
    );
    let home = fixture.home.lock().await;
    assert!(home
        .agents()
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .is_none());
    assert_eq!(
        std::fs::read(original.join("input")).unwrap(),
        b"checkpointed workspace\n"
    );
    assert!(fixture.workers[0]
        .lock()
        .await
        .agent_moves
        .targets()
        .iter()
        .all(|target| !target.active));
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_changed_grants_invalidate_prepared_move_after_restart() {
    let (fixture, session, _) = crash_fixture().await;
    let root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), "materialized").unwrap();
    let home = fixture.home.clone();
    let agent_id = fixture.agent_id.clone();
    let session_id = session.clone();
    let machine_ref = fixture.presences[0].machine_id.clone();
    let crashed = tokio::spawn(async move {
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id,
                agent_ref: agent_id,
                machine_ref,
            },
        );
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8);
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk04-prepared",
            None,
            None,
            &request,
        );
        router.dispatch(command, request).await
    })
    .await;
    assert!(crashed.is_err(), "Move must reach the prepared boundary");
    reboot_app(&fixture.home).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    fixture
        .home
        .lock()
        .await
        .agents()
        .grant_mcp(&fixture.agent_id, "mk04-new-grant".into())
        .unwrap();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session,
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk04-changed",
        None,
        None,
        &request,
    );
    let error = router
        .dispatch(command, request)
        .await
        .expect_err("Changed grants must invalidate readiness");
    assert!(
        error.to_string().contains("changed during Move"),
        "Must reject the changed requirements, got: {error}"
    );
    assert!(fixture.workers[0]
        .lock()
        .await
        .agent_moves
        .targets()
        .iter()
        .all(|target| !target.active));
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_move_records_exact_resource_readiness_and_commit_receipts() {
    let (fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    let journal = move_journal(&fixture, &session).await;
    let composition = &journal["composition"];
    assert!(composition["requirements"].is_object());
    assert!(composition["readiness"].is_object());
    assert_eq!(composition["commit"]["readiness"], composition["readiness"]);
    assert_eq!(composition["readiness"]["identity"], journal["identity"]);
    assert_eq!(
        composition["readiness"]["workspace_sha256"],
        journal["phase"]["receipt"]["archive_sha256"]
    );
    let worker = fixture.workers[0].lock().await;
    let targets = worker.agent_moves.targets();
    let target = targets
        .iter()
        .find(|target| {
            target.capture.identity.operation_id
                == journal["identity"]["operation_id"].as_str().unwrap()
        })
        .unwrap();
    assert!(target.active);
    assert_eq!(
        serde_json::to_value(&target.composition.readiness).unwrap(),
        composition["readiness"]
    );
    assert_eq!(
        serde_json::to_value(&target.composition.commit).unwrap(),
        composition["commit"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_capability_preparation_cannot_mutate_another_lease() {
    let (fixture, session, _) = crash_fixture().await;
    let root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), "bound").unwrap();
    let home = fixture.home.clone();
    let agent_ref = fixture.agent_id.clone();
    let session_id = session.clone();
    let machine_ref = fixture.presences[0].machine_id.clone();
    let crashed = tokio::spawn(async move {
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id,
                agent_ref,
                machine_ref,
            },
        );
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk04-lease-boundary",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(crashed.is_err(), "Move must reach its bound preparation");
    let requirements = fixture
        .home
        .lock()
        .await
        .agent_moves
        .for_agent(&session, &fixture.agent_id)
        .unwrap()
        .composition
        .requirements
        .unwrap();
    let mut worker = fixture.workers[0].lock().await;
    let target = worker
        .agent_moves
        .targets()
        .into_iter()
        .find(|target| !target.active)
        .unwrap();
    let identity = &target.capture.identity;
    let worktree = fixture.workspace_root.join("unrelated-worker");
    std::fs::create_dir_all(&worktree).unwrap();
    let unrelated = {
        let mut runtime = crate::app::RemoteLeaseRuntime::new(&mut worker);
        let lease = runtime
            .create_execution_lease(
                &identity.home_kernel_id,
                &identity.home_session_id,
                &identity.home_agent_id,
                false,
                &identity.owner_user_id,
            )
            .unwrap();
        runtime
            .create_leased_agent(
                &lease.id,
                "dev-stub",
                "default",
                Some("default".into()),
                None,
                None,
                None,
                None,
                Some(worktree.display().to_string()),
                None,
            )
            .unwrap()
    };
    worker
        .agents()
        .grant_skill(&unrelated.backing_agent_id, "keep-unrelated-grant".into())
        .unwrap();
    let error = crate::app::RemoteLeaseRuntime::new(&mut worker)
        .prepare_agent_move_capabilities(identity, &unrelated.id, &requirements)
        .unwrap_err();
    assert!(error.to_string().contains("exact backing lease"));
    assert_eq!(
        worker
            .agents()
            .get_agent(&unrelated.backing_agent_id)
            .unwrap()
            .skill_grants(),
        vec!["keep-unrelated-grant"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_home_package_hash_change_invalidates_preparation_after_restart() {
    let (fixture, session, _) = crash_fixture().await;
    let registry = {
        let home = fixture.home.lock().await;
        let session = home.sessions().get_session(&session).unwrap();
        crate::mcp::CharioxMcpRegistry::new(vec![crate::mcp::CharioxMcpRegistry::project_root(
            session.workspace_id(),
        )])
    };
    let mut package = crate::mcp::CharioxMcpServerConfig::stdio(
        "mk04-hash",
        "/bin/echo",
        vec!["original".into()],
    );
    registry.install(&package).unwrap();
    let root = {
        let home = fixture.home.lock().await;
        home.agents()
            .grant_mcp(&fixture.agent_id, package.name.clone())
            .unwrap();
        home.save_durable_state_snapshot().unwrap();
        home.config.private_runtime_state_root().join("agent-moves")
    };
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), "materialized").unwrap();
    let home = fixture.home.clone();
    let agent_ref = fixture.agent_id.clone();
    let session_id = session.clone();
    let machine_ref = fixture.presences[0].machine_id.clone();
    let crashed = tokio::spawn(async move {
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id,
                agent_ref,
                machine_ref,
            },
        );
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk04-hash-capture",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(crashed.is_err(), "Move must reach its prepared boundary");
    reboot_app(&fixture.home).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    package.transport =
        crate::mcp::CharioxMcpServerConfig::stdio("mk04-hash", "/bin/echo", vec!["changed".into()])
            .transport;
    registry.update(&package).unwrap();
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session,
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk04-hash-check",
        None,
        None,
        &request,
    );
    let error =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8)
            .dispatch(command, request)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("changed during Move"), "{error}");
    assert!(fixture.workers[0]
        .lock()
        .await
        .agent_moves
        .targets()
        .iter()
        .all(|target| !target.active));
}

async fn committed_grant_recovery(already_activated: bool) {
    let (mut fixture, session, _) = crash_fixture().await;
    let root = {
        let home = fixture.home.lock().await;
        let room = home.sessions().get_session(&session).unwrap();
        let registry = crate::mcp::CharioxMcpRegistry::new(vec![
            crate::mcp::CharioxMcpRegistry::project_root(room.workspace_id()),
        ]);
        registry
            .install(&crate::mcp::CharioxMcpServerConfig::stdio(
                "mk04-revoked",
                "/bin/echo",
                Vec::new(),
            ))
            .unwrap();
        home.agents()
            .grant_mcp(&fixture.agent_id, "mk04-revoked".into())
            .unwrap();
        home.save_durable_state_snapshot().unwrap();
        home.config.private_runtime_state_root().join("agent-moves")
    };
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), "committed").unwrap();
    let home = fixture.home.clone();
    let agent_ref = fixture.agent_id.clone();
    let session_id = session.clone();
    let machine_ref = fixture.presences[0].machine_id.clone();
    let crashed = tokio::spawn(async move {
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id,
                agent_ref,
                machine_ref,
            },
        );
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk04-commit-crash",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(
        crashed.is_err(),
        "Move must durably commit before simulating a lost acknowledgment"
    );
    let operation = fixture
        .home
        .lock()
        .await
        .agent_moves
        .for_agent(&session, &fixture.agent_id)
        .unwrap();
    let mut original_commit = None;
    if already_activated {
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            fixture.workers[0].clone(),
            8,
        );
        let result = router
            .relay_agent_move(
                crate::agent_move::peer::AgentMovePeerRequest::ActivateTarget {
                    identity: operation.identity.clone(),
                    readiness: Box::new(operation.composition.readiness.clone().unwrap()),
                },
                None,
            )
            .await
            .unwrap();
        let crate::agent_move::peer::AgentMovePeerResponse::Activated { receipt } = result else {
            panic!("Target must durably activate");
        };
        original_commit = Some(*receipt);
        drop(router);
        reboot_worker(&mut fixture).await;
    }
    reboot_app(&fixture.home).await;
    {
        let home = fixture.home.lock().await;
        home.remote_relay_inventory_projection_store()
            .update(Vec::new(), fixture.presences.clone());
        home.agents()
            .revoke_mcp(&fixture.agent_id, "mk04-revoked")
            .unwrap();
    }
    move_request(&fixture, &session, true).await;
    let home = fixture.home.lock().await;
    let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    assert!(
        agent.mcp_grants().is_empty(),
        "Recovering a commit must preserve the current revocation"
    );
    let settled = home
        .agent_moves
        .for_agent(&session, &fixture.agent_id)
        .unwrap();
    assert!(matches!(
        settled.phase,
        crate::agent_move::HomeMovePhase::Complete { .. }
    ));
    if let Some(original) = original_commit {
        assert_eq!(settled.composition.commit.as_ref(), Some(&original));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_committed_move_recovers_after_grant_revocation() {
    committed_grant_recovery(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_lost_activation_reply_recovers_after_grant_revocation() {
    committed_grant_recovery(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_corrupt_captured_archive_rejects_before_target_activation() {
    let (fixture, session, original) = crash_fixture().await;
    let root = fixture
        .home
        .lock()
        .await
        .config
        .private_runtime_state_root()
        .join("agent-moves");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("crash-after-phase-for-test"), "captured").unwrap();
    let home = fixture.home.clone();
    let agent_ref = fixture.agent_id.clone();
    let session_id = session.clone();
    let machine_ref = fixture.presences[0].machine_id.clone();
    let crashed = tokio::spawn(async move {
        let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id,
                agent_ref,
                machine_ref,
            },
        );
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "mk04-archive-capture",
            None,
            None,
            &request,
        );
        crate::runtime::router::CommandRouter::with_interactive_capacity(home, 8)
            .dispatch(command, request)
            .await
    })
    .await;
    assert!(crashed.is_err());
    {
        let home = fixture.home.lock().await;
        let operation = home
            .agent_moves
            .for_agent(&session, &fixture.agent_id)
            .unwrap();
        let archive = home.agent_moves.artifact_path(
            &operation.identity.operation_id,
            crate::agent_move::AgentMoveArtifactKind::Workspace,
        );
        std::fs::write(archive, b"invalid archive input").unwrap();
    }
    reboot_app(&fixture.home).await;
    fixture
        .home
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .update(Vec::new(), fixture.presences.clone());
    let request = crate::local::LocalDaemonRequest::MoveAgentToRemote(
        crate::local::MoveAgentToRemoteRequest {
            session_id: session.clone(),
            agent_ref: fixture.agent_id.clone(),
            machine_ref: fixture.presences[0].machine_id.clone(),
        },
    );
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "mk04-archive-retry",
        None,
        None,
        &request,
    );
    assert!(
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8)
            .dispatch(command, request)
            .await
            .is_err()
    );
    assert!(fixture.workers[0]
        .lock()
        .await
        .agent_moves
        .targets()
        .iter()
        .all(|target| !target.active && target.composition.readiness.is_none()));
    assert_eq!(
        std::fs::read(original.join("input")).unwrap(),
        b"checkpointed workspace\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_locked_required_vault_rejects_before_publication_and_activation() {
    use crate::managed_context::development::*;
    use crate::project_environment::*;
    use std::collections::BTreeMap;
    let (mut fixture, session, source) = crash_fixture().await;
    std::fs::write(source.join("app.js"), "console.log(process.env.TOKEN)\n").unwrap();
    std::fs::write(
        source.join(".env.local"),
        "TOKEN=synthetic-selected-input\n",
    )
    .unwrap();
    let (mut source_config, owner, source_project_id) = {
        let home = fixture.home.lock().await;
        (
            home.config.clone(),
            home.agents()
                .get_agent(&fixture.agent_id)
                .unwrap()
                .owner_user_id()
                .to_string(),
            home.sessions()
                .get_session(&session)
                .unwrap()
                .project_id()
                .to_string(),
        )
    };
    source_config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    source_config.user_config.credential_vault.path = fixture
        .workspace_root
        .join("required-source-vault.json")
        .display()
        .to_string();
    crate::secret::create_chariox_encrypted_vault_for_test(
        std::path::Path::new(&source_config.user_config.credential_vault.path),
        "synthetic-fixture-passphrase",
    )
    .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &source_config.user_config.credential_vault.path,
        "synthetic-fixture-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let target_config = {
        let mut worker = fixture.workers[0].lock().await;
        worker.config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        worker.config.user_config.credential_vault.path = fixture
            .workspace_root
            .join("required-target-vault.json")
            .display()
            .to_string();
        worker.config.clone()
    };
    crate::secret::create_chariox_encrypted_vault_for_test(
        std::path::Path::new(&target_config.user_config.credential_vault.path),
        "synthetic-fixture-passphrase",
    )
    .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        &target_config.user_config.credential_vault.path,
        "synthetic-fixture-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    reboot_worker(&mut fixture).await;
    crate::secret::lock_chariox_encrypted_vault(&target_config.user_config.credential_vault.path)
        .unwrap();
    assert!(
        !crate::secret::chariox_encrypted_vault_status(
            &target_config.user_config.credential_vault.path
        )
        .unwrap()
        .unlocked
    );
    let roots = BTreeMap::from([(source.display().to_string(), source.clone())]);
    let index = index_project_environment(&roots, &BTreeMap::new()).unwrap();
    let project_id = format!("move-project-{:032x}", rand::random::<u128>());
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: project_id.clone(),
        evidence_digest: index.evidence.digest(),
        entries: index.references,
        private_files: Vec::new(),
        toolchain_hints: Vec::new(),
        package_hints: Vec::new(),
        service_hints: Vec::new(),
    };
    let vault = crate::secret::project_environment_vault(&source_config).unwrap();
    let resolved =
        resolve_project_environment(&manifest, &roots, &BTreeMap::new(), vault.as_ref()).unwrap();
    assert_eq!(
        resolved.values.len(),
        1,
        "A selected value must require the target Vault"
    );
    let operation = format!("move-{:032x}", rand::random::<u128>());
    let sealed = seal_project_environment(
        &operation,
        &source_config.daemon_id,
        &target_config.daemon_id,
        &source_config.relay_private_key,
        &target_config.relay_public_key,
        &manifest,
        &resolved,
    )
    .unwrap();
    let source_id = source.display().to_string();
    let exported = export_development_context_with_environment(
        DevelopmentContextExportRequest {
            project_id: project_id.clone(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: source_id.clone(),
                worktree_id: None,
                worktree_path: source.clone(),
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: fixture.workspace_root.join("required-environment.tar.gz"),
        },
        Some(DevelopmentProjectEnvironment {
            sealed,
            evidence: index.evidence,
            repository_workspaces: BTreeMap::new(),
        }),
    )
    .unwrap();
    let thumb = crate::runtime::terminal_pairings::public_key_thumbprint;
    let identity = crate::agent_move::AgentMoveIdentity {
        operation_id: operation,
        generation: 1,
        home_kernel_id: source_config.daemon_id.clone(),
        home_session_id: session,
        home_agent_id: fixture.agent_id.clone(),
        owner_user_id: owner,
        source_kernel_id: source_config.daemon_id.clone(),
        target_kernel_id: target_config.daemon_id.clone(),
        home_key_thumbprint: thumb(&source_config.relay_public_key),
        source_key_thumbprint: thumb(&source_config.relay_public_key),
        target_key_thumbprint: thumb(&target_config.relay_public_key),
        target_public_key: target_config.relay_public_key.clone(),
    };
    let bytes = std::fs::read(&exported.archive_path).unwrap();
    let capture = crate::agent_move::AgentMoveCapture {
        identity: identity.clone(),
        source_project_id,
        archive_project_id: project_id,
        source_provider_run_id: None,
        source_repositories: vec![DevelopmentSourceRepositoryBinding {
            workspace_id: source_id,
            worktree_id: None,
            role: DevelopmentRepositoryRole::Primary,
        }],
        workspace: crate::agent_move::AgentMoveArtifact {
            sha256: exported.archive_sha256,
            size_bytes: bytes.len() as u64,
        },
        native: None,
        checkpoint_binding: None,
    };
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        fixture.workers[0].clone(),
        8,
    );
    use crate::agent_move::peer::{AgentMoveChunk, AgentMovePeerRequest as Request};
    router
        .relay_agent_move(
            Request::BeginTarget {
                capture: Box::new(capture),
            },
            None,
        )
        .await
        .unwrap();
    for (index, chunk) in bytes
        .chunks(crate::agent_move::MOVE_CHUNK_BYTES)
        .enumerate()
    {
        router
            .relay_agent_move(
                Request::WriteChunk {
                    identity: identity.clone(),
                    kind: crate::agent_move::AgentMoveArtifactKind::Workspace,
                    chunk: AgentMoveChunk {
                        offset: (index * crate::agent_move::MOVE_CHUNK_BYTES) as u64,
                        bytes: chunk.to_vec(),
                    },
                },
                None,
            )
            .await
            .unwrap();
    }
    let error = router
        .relay_agent_move(
            Request::PublishTarget {
                identity: identity.clone(),
            },
            None,
        )
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.to_lowercase().contains("vault") && message.to_lowercase().contains("locked"),
        "Must reach the required Vault gate: {message}"
    );
    assert!(!message.contains("synthetic-selected-input"));
    let worker = fixture.workers[0].lock().await;
    let target = worker.agent_moves.target(&identity.operation_id).unwrap();
    assert!(!target.active && target.receipt.is_none() && target.composition.readiness.is_none());
    assert_eq!(
        std::fs::read_to_string(source.join(".env.local")).unwrap(),
        "TOKEN=synthetic-selected-input\n"
    );
}

#[path = "move_setup_tests.rs"]
mod move_setup_tests;

async fn dispatch_move(
    fixture: &Fixture,
    session: &str,
    abort: bool,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    let request = if abort {
        crate::local::LocalDaemonRequest::MoveAgentToLocal(crate::local::MoveAgentToLocalRequest {
            abort_pending: true,
            session_id: session.into(),
            agent_ref: fixture.agent_id.clone(),
        })
    } else {
        crate::local::LocalDaemonRequest::MoveAgentToRemote(
            crate::local::MoveAgentToRemoteRequest {
                session_id: session.into(),
                agent_ref: fixture.agent_id.clone(),
                machine_ref: fixture.presences[0].machine_id.clone(),
            },
        )
    };
    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(fixture.home.clone(), 8);
    let envelope = crate::runtime::command::KernelCommand::from_local_request(
        "mk05-wait",
        None,
        None,
        &request,
    );
    router.dispatch(envelope, request).await
}

async fn agent_is_remote(fixture: &Fixture) -> bool {
    fixture
        .home
        .lock()
        .await
        .agents()
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .is_some()
}

#[tokio::test(flavor = "multi_thread")]
async fn mk05_move_requested_during_a_turn_waits_for_it_and_can_be_cancelled() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    let (fixture, session, _) = crash_fixture().await;
    let room = fixture
        .home
        .lock()
        .await
        .sessions()
        .get_session(&session)
        .unwrap();
    let started = fixture
        .home
        .lock()
        .await
        .prompt_state_owner()
        .submit_prepared_prompt(
            &room,
            crate::session::PromptQueueItem::new(
                "turn-1",
                "client",
                &fixture.agent_id,
                "work",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
    assert!(matches!(
        started,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    // A Move requested during the turn waits; cancelling ends the wait and leaves the agent alone.
    let (moved, cancelled) = tokio::join!(dispatch_move(&fixture, &session, false), async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!agent_is_remote(&fixture).await);
        assert!(
            fixture
                .home
                .lock()
                .await
                .agent_moves
                .for_agent(&session, &fixture.agent_id)
                .is_none(),
            "a waiting Move must not fence the running agent"
        );
        dispatch_move(&fixture, &session, true).await
    });
    cancelled.expect("a waiting Move can be cancelled");
    let error = moved.expect_err("the cancelled request ends with a reason");
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(!agent_is_remote(&fixture).await);
    // The same request proceeds once the turn has ended.
    let (moved, ()) = tokio::join!(dispatch_move(&fixture, &session, false), async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!agent_is_remote(&fixture).await);
        fixture
            .home
            .lock()
            .await
            .prompt_state_owner()
            .complete_active_prompt_only(&room, &fixture.agent_id);
    });
    moved.expect("the Move runs after the turn ends");
    assert!(agent_is_remote(&fixture).await);
}
