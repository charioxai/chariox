//! A shell or terminal of a Project gets its Variables and present Secrets; a locked Vault leaves only the Secrets out.
use super::*;
use crate::project_environment::*;

#[tokio::test]
async fn a_locked_vault_leaves_secrets_out_of_a_shell_and_a_terminal_but_keeps_the_variables() {
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let workspace = crate::test_support::TestWorktree::new("shell-project-environment");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = config
        .private_runtime_state_root()
        .join("locked-vault.json")
        .to_string_lossy()
        .into_owned();
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    ProjectEnvironmentStore::new(&app.config().private_runtime_state_root())
        .save(
            session.project_id(),
            0,
            ProjectEnvironmentRecord {
                schema: PROJECT_ENVIRONMENT_SCHEMA,
                variables: "LOG_LEVEL=debug".into(),
                secrets: vec!["NOTION_TOKEN".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let shell = runtime.project_environment_for_shell(session.id()).unwrap();
    assert_eq!(
        shell
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>(),
        vec!["LOG_LEVEL".to_string()]
    );
    let terminal = runtime
        .project_environment_for_terminal(session.id())
        .unwrap();
    assert_eq!(terminal.get("LOG_LEVEL").map(String::as_str), Some("debug"));
    assert!(!terminal.contains_key("NOTION_TOKEN"));
}

// MP-08 / MP-09 / MP-10 / MP-11: each new checkout retains the source Project, not the first Git sibling.
#[tokio::test]
async fn local_worktree_placement_keeps_the_source_project_environment() {
    assert_placed_project_environment(false).await;
}

#[tokio::test]
async fn leased_worktree_placement_keeps_the_source_project_environment() {
    assert_placed_project_environment(true).await;
}

async fn assert_placed_project_environment(remote: bool) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let root = crate::test_support::TestWorktree::new("placed-project-environment");
    let main = root.path().join("main");
    let linked = root.path().join("linked");
    let target = root.path().join("third");
    std::fs::create_dir_all(&main).unwrap();
    for args in [
        vec!["init"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Initial",
        ],
        vec!["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    ] {
        assert!(crate::process_spawn::command("git")
            .args(args)
            .current_dir(&main)
            .output()
            .unwrap()
            .status
            .success());
    }
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = root.path().join("vault.json").display().to_string();
    crate::secret::unlock_chariox_encrypted_vault(
        std::path::Path::new(&config.user_config.credential_vault.path),
        "fixture-pass",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    let mut app = crate::test_support::bootstrap_authenticated_app(config.clone()).unwrap();
    let store = app.session_state_store();
    store.restore_projects(vec![
        crate::session::RuntimeProject::new(
            "a-main",
            "owner",
            main.display().to_string(),
            "Main",
            crate::session::RuntimeProjectKind::Named,
        ),
        crate::session::RuntimeProject::new(
            "z-linked",
            "owner",
            linked.display().to_string(),
            "Linked",
            crate::session::RuntimeProjectKind::Named,
        ),
    ]);
    let environments = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    for (id, label) in [("a-main", "wrong"), ("z-linked", "source")] {
        environments
            .save(
                id,
                0,
                ProjectEnvironmentRecord {
                    schema: PROJECT_ENVIRONMENT_SCHEMA,
                    variables: format!("PROJECT_LABEL={label}"),
                    secrets: vec!["PROJECT_SECRET".into()],
                    files: vec![format!("{label}.local")],
                    ..Default::default()
                },
            )
            .unwrap();
        crate::secret::project_environment_vault(&config)
            .unwrap()
            .set_secret(
                &project_environment_vault_service(id),
                "PROJECT_SECRET",
                label,
            )
            .unwrap();
        std::fs::write(linked.join(format!("{label}.local")), "fixture").unwrap();
    }
    let placement = crate::agent::GitWorktreePlacement {
        target_directory: Some(target.display().to_string()),
        branch: Some("third".into()),
        from_ref: Some("HEAD".into()),
    };
    let (session, agent, runtime) = if remote {
        let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease("home", "home-session", "home-agent", false, "owner")
            .unwrap();
        let leased = crate::app::RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent_from_base_directory(
                &linked,
                &lease.id,
                "dev-stub",
                "default",
                None,
                None,
                None,
                None,
                None,
                None,
                Some(placement),
            )
            .unwrap();
        let session = store.get_session(&leased.backing_session_id).unwrap();
        let agent = app.agents().get_agent(&leased.backing_agent_id).unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        (session, agent, runtime)
    } else {
        let session = store
            .create_session(
                crate::session::CreateSessionRequest::new(
                    linked.display().to_string(),
                    linked.display().to_string(),
                )
                .with_owner_user_id("owner")
                .with_project_selection(
                    crate::session::SessionProjectSelection::Existing {
                        project_id: "z-linked".into(),
                    },
                ),
            )
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let agent = runtime
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("owner")
                    .with_worktree_placement(placement),
            )
            .await
            .unwrap();
        (session, agent, runtime)
    };
    // Provider activation mirrors the session back through the shared restore entry point.
    let session = store.restore_session(session);
    assert!(target.join("source.local").exists());
    assert!(!target.join("wrong.local").exists());
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "test",
    )
    .with_working_directory(target.clone());
    let request =
        attach_project_provider_environment(&config, &store, &session, Some(&agent), request)
            .unwrap();
    // Compare as booleans so even a failing assertion cannot echo Secret values.
    for name in ["PROJECT_LABEL", "PROJECT_SECRET"] {
        assert!(
            request
                .provider_credential_env
                .iter()
                .any(|(key, value)| key == name && value == "source"),
            "source Project input missing"
        );
    }
    assert_eq!(
        runtime.tool_project(&session, "owner").unwrap().id(),
        "z-linked"
    );
    assert_eq!(
        store.get_session(session.id()).unwrap().project_id(),
        "z-linked"
    );
    assert_eq!(
        store.get_project("z-linked").unwrap().workspace_ids(),
        &[linked.display().to_string()],
        "a leased checkout must not register a remote-lease workspace root"
    );
}

// MP-08 / MP-09 / MP-10 / MP-11: authenticated tools follow the calling provider's checkout.
#[tokio::test]
async fn environment_tools_use_the_authenticated_agent_checkout() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("environment-agent-checkout");
    let room = root.path().join("repo");
    let target = root.path().join("repo-task");
    std::fs::create_dir_all(&room).unwrap();
    for args in [
        vec!["init"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "Initial",
        ],
        vec!["worktree", "add", "-b", "task", target.to_str().unwrap()],
    ] {
        assert!(crate::process_spawn::command("git")
            .args(args)
            .current_dir(&room)
            .output()
            .unwrap()
            .status
            .success());
    }
    std::fs::write(room.join("private.local"), "room only").unwrap();
    std::fs::write(target.join(".env.example"), "AGENT_ONLY=example\n").unwrap();
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            room.display().to_string(),
            room.display().to_string(),
        ))
        .unwrap();
    let store = app.session_state_store();
    ProjectEnvironmentStore::new(&app.config().private_runtime_state_root())
        .save(
            session.project_id(),
            0,
            ProjectEnvironmentRecord {
                schema: PROJECT_ENVIRONMENT_SCHEMA,
                setup_script: "printf prepared > prepared.local".into(),
                files: vec!["private.local".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let agent = app
        .agents()
        .update_agent_config(
            agent.id(),
            None,
            None,
            None,
            Some(Some(target.display().to_string())),
        )
        .unwrap();
    assert_eq!(agent.worktree_id(), target.to_str());
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "test",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "environment-checkout-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "checkout tool regression".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(target.clone()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some("environment-checkout-token".into()));
    app.providers_mut().insert_run_for_test(run);
    let mut outside = crate::provider::RuntimeProviderRun::new(
        "environment-outside-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "outside checkout regression".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(root.path().to_owned()),
            structured_endpoint: None,
        },
    );
    outside.mark_running();
    outside.set_runtime_mcp_auth_token(Some("environment-outside-token".into()));
    app.providers_mut().insert_run_for_test(outside);

    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    assert_eq!(
        store.get_session(session.id()).unwrap().worktree_id(),
        room.to_str().unwrap()
    );
    for name in ["read", "update", "setup", "check"] {
        let refused = runtime
            .dispatch_authenticated_runtime_tool_call(
                "environment-outside-token",
                &format!("chariox.environment.{name}"),
                serde_json::json!({}),
            )
            .await;
        assert!(
            refused.is_err(),
            "a checkout outside the Project must be refused"
        );
    }
    let call = |name| {
        runtime.dispatch_authenticated_runtime_tool_call(
            "environment-checkout-token",
            name,
            serde_json::json!({}),
        )
    };
    let read = call("chariox.environment.read").await.unwrap();
    assert_eq!(
        read.payload["files"][0]["presence"], "missing",
        "read must inspect the agent checkout"
    );
    let check = call("chariox.environment.check").await.unwrap();
    assert!(
        check.payload["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["text"] == "AGENT_ONLY"),
        "check must inspect the agent files"
    );
    runtime.owned.notification_inventory_projection.update(
        vec![],
        vec![chariox_relay::protocol::RelayKernelPresence {
            kernel_id: "source-kernel".into(),
            machine_id: "source-machine".into(),
            machine_alias: None,
            kernel_alias: Some("Source".into()),
            relay_alias: None,
            available_providers: vec![],
            provider_accounts: vec![],
            capabilities: vec![],
            accepting_remote_leases: true,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: "00".repeat(32),
        }],
    );
    let update = runtime
        .dispatch_authenticated_runtime_tool_call(
            "environment-checkout-token",
            "chariox.environment.update",
            serde_json::json!({"from":"Source"}),
        )
        .await
        .unwrap();
    let update_id = update.payload["operationId"].as_str().unwrap();
    assert_eq!(
        runtime
            .environment_operation(update_id)
            .unwrap()
            .unwrap()
            .checkout_path,
        target.display().to_string()
    );
    runtime
        .cancel_environment_operation_by_id(update_id)
        .unwrap();
    let setup = call("chariox.environment.setup").await.unwrap();
    let id = setup.payload["operationId"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let op = runtime.environment_operation(id).unwrap().unwrap();
            if op.state != crate::local::EnvironmentOperationState::Running {
                assert_eq!(op.state, crate::local::EnvironmentOperationState::Succeeded);
                assert_eq!(op.checkout_path, target.display().to_string());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(target.join("prepared.local").exists());
    assert!(!room.join("prepared.local").exists());
}
