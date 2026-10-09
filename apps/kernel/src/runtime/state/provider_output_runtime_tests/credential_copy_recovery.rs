//! MP-08/MP-10/MP-11: fake OAuth drill; no real accounts or endpoints.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_mp11_fake_oauth_copy_notice_login_and_resume_stay_on_worker() {
    isolated_case(RecoveryCase::Prompt).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_mp11_credential_copy_startup_login_recovery_stays_on_worker() {
    isolated_case(RecoveryCase::Startup).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_mp11_login_relaunch_failure_settles_prompt_and_advances_queue() {
    isolated_case(RecoveryCase::RelaunchFailure).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp08_mp10_mp11_login_relaunch_stall_settles_prompt_without_late_spawn() {
    isolated_case(RecoveryCase::RelaunchStall).await;
}

#[derive(Clone, Copy)]
enum RecoveryCase {
    Prompt,
    Startup,
    RelaunchFailure,
    RelaunchStall,
}

// Run the async login seam in a separate process. EnvGuard intentionally restores
// variables on drop; the child must inherit its fake binaries before taking that guard.
async fn isolated_case(recovery: RecoveryCase) {
    let case = match recovery {
        RecoveryCase::Prompt => {
            "mp08_mp10_mp11_fake_oauth_copy_notice_login_and_resume_stay_on_worker"
        }
        RecoveryCase::Startup => {
            "mp08_mp10_mp11_credential_copy_startup_login_recovery_stays_on_worker"
        }
        RecoveryCase::RelaunchFailure => {
            "mp08_mp10_mp11_login_relaunch_failure_settles_prompt_and_advances_queue"
        }
        RecoveryCase::RelaunchStall => {
            "mp08_mp10_mp11_login_relaunch_stall_settles_prompt_without_late_spawn"
        }
    };
    if std::env::var("CHARIOX_CREDWARN_FIXTURE_CHILD")
        .ok()
        .as_deref()
        == Some(case)
    {
        run_case(recovery).await;
        return;
    }
    let result = tokio::task::spawn_blocking(move || {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "chariox-credwarn-fixture-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("ambient")).unwrap();
        let fixture = root.join("bin/codex");
        std::fs::write(
            &fixture,
            include_str!("credential_copy_recovery_fixture.py"),
        )
        .unwrap();
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
        let test = format!(
            "runtime::state::provider_output_runtime_tests::credential_copy_recovery::{case}"
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--test-threads=1", "--nocapture"])
            .env("CHARIOX_CREDWARN_FIXTURE_CHILD", case)
            .env("CHARIOX_CODEX_BIN", &fixture)
            .env("CODEX_HOME", root.join("ambient"))
            .env("CHARIOX_HOME", root.join("state"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("RUST_MIN_STACK", "16777216")
            .output()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        output
    })
    .await
    .unwrap();
    assert!(
        result.status.success(),
        "isolated fake OAuth case failed: {}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn run_case(recovery: RecoveryCase) {
    let startup_failure = matches!(recovery, RecoveryCase::Startup);
    let relaunch_failure = matches!(recovery, RecoveryCase::RelaunchFailure);
    let relaunch_stall = matches!(recovery, RecoveryCase::RelaunchStall);
    use std::os::unix::fs::PermissionsExt;
    let workspace = crate::test_support::TestWorktree::new("credential-copy-recovery");
    let _env = crate::env_lock::lock();
    let mut config = crate::config::DaemonConfig::for_tests();
    if relaunch_stall {
        config.provider_runtime_init_delay_ms = 65_000;
    }
    let fixture = config
        .private_runtime_state_root()
        .join("codex-auth-fixture.py");
    std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    std::fs::write(
        &fixture,
        include_str!("credential_copy_recovery_fixture.py"),
    )
    .unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old_bin = std::env::var_os("CHARIOX_CODEX_BIN");
    let old_home = std::env::var_os("CODEX_HOME");
    std::env::set_var("CHARIOX_CODEX_BIN", &fixture);
    std::env::set_var("CODEX_HOME", workspace.path().join("ambient-synthetic"));
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let registry = app.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let source = crate::account_profile::ProviderAccountProfileRegistry::open(
        workspace.path().join("source/accounts.json"),
    )
    .unwrap()
    .with_machine_identity("synthetic-source-machine", "synthetic-source-kernel");
    let profile = source
        .create_managed(owner, "codex", "Synthetic copied login")
        .unwrap();
    let environment = source
        .resolve_environment(owner, "codex", &profile.profile_id)
        .unwrap();
    let source_auth = std::path::Path::new(&environment["CODEX_HOME"]).join("auth.json");
    let original = br#"{"tokens":{"refresh_token":"synthetic-source-login"}}"#;
    std::fs::write(&source_auth, original).unwrap();
    let materialization = source
        .export_materialization(owner, "codex", &profile.profile_id)
        .unwrap();
    let copied = registry
        .materialize_replica(owner, &materialization)
        .unwrap();
    registry
        .record_received_account_copy(
            owner,
            &materialization,
            &copied.profile_id,
            crate::account_profile::ProviderAccountMaterializationTargetKind::Worker,
        )
        .unwrap();
    crate::test_support::authenticate_provider_account(
        &registry,
        owner,
        "codex",
        &copied.profile_id,
    )
    .unwrap();
    let target_environment = registry
        .resolve_environment(owner, "codex", &copied.profile_id)
        .unwrap();
    let target_home = std::path::PathBuf::from(&target_environment["CODEX_HOME"]);
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex")
                .with_model("fixture-model")
                .with_account_profile(copied.profile_id.clone()),
        )
        .unwrap();
    app.focus_agent(session.id(), agent.id()).unwrap();
    let mut attachments = Vec::new();
    for client in ["client-one", "client-two"] {
        attachments.push(
            crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    session.id(),
                    client,
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap(),
        );
    }
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        &copied.profile_id,
        "fixture-model",
    )
    .with_agent_id(agent.id());
    let failed_launch = if startup_failure {
        Some(app.start_provider_launch(request).unwrap())
    } else {
        app.launch_provider(request).unwrap();
        app.submit_prompt(
            session.id(),
            attachments[0].id(),
            Some(agent.id()),
            "synthetic turn",
            Vec::new(),
        )
        .unwrap();
        None
    };
    let queued_id = if relaunch_failure || relaunch_stall {
        let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
            .submit_prompt(
                session.id(),
                attachments[0].id(),
                Some(agent.id()),
                "queued after recovery",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("second prompt should queue");
        };
        Some(prompt.id().to_string())
    } else {
        None
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    drop(_env); // Official login runs on spawn_blocking and needs the launch environment lock.
    let run = runtime
        .owned
        .provider_store
        .get_run_for_agent(session.id(), agent.id())
        .unwrap();
    assert_eq!(run.account_profile(), copied.profile_id);
    if let Some(started) = failed_launch.as_ref() {
        runtime
            .fail_provider_launch(
                started,
                &DaemonError::ProviderProtocol {
                    provider_run_id: run.id().into(),
                    operation: "initialize fixture provider",
                    message: "refresh_token_reused".into(),
                },
            )
            .await;
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if startup_failure {
                runtime.publish_credential_copy_notices(session.id());
            } else {
                let _ = runtime
                    .pump_owned_provider_output(
                        session.id(),
                        run.id(),
                        attachments.iter().map(|a| a.id().to_string()).collect(),
                        true,
                    )
                    .await;
            }
            let snapshot = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if let Some(interaction) = snapshot.active_interaction_for_agent(agent.id()) {
                assert_eq!(interaction.title(), Some("Log in to Codex on this machine"));
                runtime
                    .resolve_runtime_interaction(session.id(), interaction.id(), "login", None)
                    .await
                    .unwrap();
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("renewal failure should raise the kernel interaction");
    let challenge = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if let Some(login) = snapshot
                .active_interaction_for_agent(agent.id())
                .and_then(|i| i.provider_login())
                .cloned()
            {
                break login;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("official login should expose its challenge");
    let durable = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .durable_runtime_snapshot();
    assert!(durable.active_interactions().is_empty());
    assert!(!serde_json::to_string(&durable)
        .unwrap()
        .contains("SYNTHETIC"));
    assert_eq!(challenge.kernel_id, "daemon-test");
    assert_eq!(challenge.login.login_kind, "chatgptDeviceCode");
    assert!(
        challenge.login.user_code.as_deref() == Some("SYNTHETIC"),
        "Only synthetic provider login is permitted"
    );
    assert_eq!(challenge.login.account_profile, copied.profile_id);
    assert_eq!(
        runtime
            .owned
            .provider_account_profiles
            .resolve_environment(owner, "codex", &copied.profile_id)
            .unwrap(),
        target_environment
    );
    assert!(target_home
        .join("synthetic-official-login-invoked")
        .exists());
    assert!(!source_auth
        .parent()
        .unwrap()
        .join("synthetic-official-login-invoked")
        .exists());
    let admitted_id = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(
            &runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap(),
            agent.id(),
        )
        .map(|p| p.id().to_string());
    if relaunch_failure {
        // Official login is already running. Fail replacement initialization once.
        std::fs::write(target_home.join("synthetic-relaunch-failure"), "once").unwrap();
    }
    let verification_url = challenge.login.verification_url.unwrap();
    assert!(
        verification_url.starts_with("http://127.0.0.1:"),
        "Only the loopback fake OAuth server is permitted"
    );
    let url = verification_url.replace("/device", "/consent");
    tokio::task::spawn_blocking(move || ureq::get(&url).call().unwrap())
        .await
        .unwrap();
    if relaunch_failure || relaunch_stall {
        assert!(
            admitted_id.is_some(),
            "recovery must retain the admitted prompt"
        );
        let mut stalled_run_id = None;
        tokio::time::timeout(Duration::from_secs(70), async {
            loop {
                if relaunch_stall && stalled_run_id.is_none() {
                    stalled_run_id = runtime
                        .owned
                        .provider_store
                        .get_run_for_agent(session.id(), agent.id())
                        .filter(|replacement| replacement.id() != run.id())
                        .map(|replacement| replacement.id().to_string());
                }
                let snapshot = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap();
                let active = runtime
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&snapshot, agent.id());
                if active.as_ref().is_none_or(|p| Some(p.id()) != admitted_id.as_deref())
                    && (!relaunch_stall
                        || stalled_run_id.as_deref().is_some_and(|id| {
                            runtime.owned.provider_store.get_run(id).is_ok_and(|run| {
                                run.state() == crate::provider::ProviderRunState::Ended
                            })
                        }))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            let snapshot = runtime.owned.session_store.get_session(session.id()).unwrap();
            let active = runtime.owned.prompt_state_owner.active_prompt_for_agent(&snapshot, agent.id());
            let queued = runtime.owned.prompt_state_owner.peek_next_queued_prompt(&snapshot, agent.id());
            let current = runtime.owned.provider_store.get_run_for_agent(session.id(), agent.id());
            panic!("failed relaunch must settle and advance: active={:?}, queued={:?}, run={:?}, native_dispatch={}, original_queue_id={:?}, notices={:?}",
                active.as_ref().map(|p| p.id()), queued.as_ref().map(|p| p.id()),
                current.as_ref().map(|r| (r.id(), r.state())), target_home.join("synthetic-resumed").exists(), queued_id, runtime.owned.terminal_stream.notice_records());
        });
        // The stalled replacement consumes the 60s recovery deadline. Give
        // the next provider its own readiness budget instead of sharing the
        // remaining 10s with process startup and queued dispatch under load.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let snapshot = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap();
                if runtime
                    .owned
                    .prompt_state_owner
                    .peek_next_queued_prompt(&snapshot, agent.id())
                    .is_none()
                    && target_home.join("synthetic-resumed").exists()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("queued prompt must dispatch after recovery settles the failed turn");
        let snapshot = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        assert!(
            runtime
                .owned
                .prompt_state_owner
                .peek_next_queued_prompt(&snapshot, agent.id())
                .is_none(),
            "recovery failure must advance the queue"
        );
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&snapshot, agent.id());
        // Queue activation assigns a fresh mirror ID; match the queued content.
        assert!(active
            .as_ref()
            .is_none_or(|p| p.prompt().trim() == "queued after recovery"));
        // Completion can race the snapshot. Prove actual dispatch, including its
        // exactly-once count, rather than requiring a transient Working state.
        let turns: serde_json::Value = serde_json::from_slice(
            &std::fs::read(target_home.join("synthetic-turns.json")).unwrap(),
        )
        .unwrap();
        let queued_starts = turns["threads"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|thread| thread.as_array().unwrap())
            .filter(|turn| {
                turn["input"].as_array().unwrap().iter().any(|input| {
                    input["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("queued after recovery"))
                })
            })
            .count();
        assert_eq!(
            queued_starts, 1,
            "the queued prompt must be dispatched exactly once"
        );
        if relaunch_stall {
            let stalled = stalled_run_id.expect("replacement must start before stalling");
            tokio::time::sleep(Duration::from_secs(6)).await;
            assert_eq!(
                runtime
                    .owned
                    .provider_store
                    .get_run(&stalled)
                    .unwrap()
                    .state(),
                crate::provider::ProviderRunState::Ended
            );
            assert!(
                !app.lock().await.pty().has_process(&stalled),
                "retired replacement spawned after its deadline"
            );
        }
    } else {
        tokio::time::timeout(Duration::from_secs(20), async {
            let marker = if startup_failure {
                "synthetic-ready"
            } else {
                "synthetic-resumed"
            };
            while !target_home.join(marker).exists()
                || runtime
                    .owned
                    .provider_store
                    .get_run_for_agent(session.id(), agent.id())
                    .is_none_or(|current| {
                        current.id() == run.id()
                            || current.state() != crate::provider::ProviderRunState::Running
                    })
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the same admitted turn should resume after login");
    }
    assert_eq!(std::fs::read(&source_auth).unwrap(), original);
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .active_interactions()
        .is_empty());
    for attachment in &attachments {
        let notices = runtime
            .owned
            .terminal_stream
            .drain_notice_records(session.id(), attachment.id());
        assert_eq!(
            notices
                .iter()
                .filter(|n| n.message.contains("was copied from"))
                .count(),
            1
        );
    }
    assert!(registry
        .take_credential_copy_notice(owner, "codex", &copied.profile_id, "machine-test")
        .unwrap()
        .is_none());
    // Synthetic login details must stay out of durable history.
    let history = app
        .lock()
        .await
        .load_session_history_entries(
            &runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap(),
            Some(agent.id()),
        )
        .unwrap();
    assert!(!history
        .iter()
        .any(|entry| entry.text.contains("SYNTHETIC") || entry.text.contains("/device")));
    app.lock().await.shutdown_cleanup().unwrap();
    let _env = crate::env_lock::lock();
    match old_bin {
        Some(value) => std::env::set_var("CHARIOX_CODEX_BIN", value),
        None => std::env::remove_var("CHARIOX_CODEX_BIN"),
    }
    match old_home {
        Some(value) => std::env::set_var("CODEX_HOME", value),
        None => std::env::remove_var("CODEX_HOME"),
    }
}
