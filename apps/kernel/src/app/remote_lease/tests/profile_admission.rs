use super::*;
use crate::app::remote_lease::provider_run::LeasedProviderRunMatch;
use std::future::{poll_fn, Future};
use std::task::Poll;

#[tokio::test]
async fn leased_prompt_replaces_idle_run_on_a_different_account() {
    assert_run_profile_reconciliation("account", None).await;
}

#[tokio::test]
async fn leased_prompt_replaces_idle_run_with_a_different_provider_model_or_effort() {
    for field in ["provider", "model", "effort"] {
        assert_run_profile_reconciliation(field, None).await;
    }
}

#[tokio::test]
async fn leased_prompt_preserves_busy_run_on_profile_mismatch() {
    for field in ["provider", "account", "model", "effort"] {
        assert_run_profile_reconciliation(field, Some(false)).await;
        assert_run_profile_reconciliation(field, Some(true)).await;
    }
}

// None means idle, Some(false) active, Some(true) queued for provider startup.
async fn assert_run_profile_reconciliation(field: &str, queued_work: Option<bool>) {
    crate::test_support::isolated_env_test!();
    let (mut app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let mut previous = profile.clone();
    match field {
        "provider" => previous.provider = "dev-stub".into(),
        "account" => previous.account_profile = "previous-account".into(),
        "model" => previous.model = Some("previous-model".into()),
        "effort" => previous.effort = Some("high".into()),
        _ => panic!("unknown profile field"),
    }
    let stale = app
        .providers_mut()
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                &lease.backing_session_id,
                "dev-stub",
                &previous.provider,
                &previous.account_profile,
                previous.model.as_deref().unwrap(),
            )
            .with_agent_id(&lease.backing_agent_id)
            .with_variant(previous.effort.clone())
            .with_resume_state(
                crate::provider::ProviderResumeState::from_codex_thread_id(
                    "previous-account-thread",
                ),
            ),
        )
        .unwrap();
    if queued_work == Some(false) {
        sync_active_prompt(&mut app, &lease);
    } else if queued_work == Some(true) {
        app.prompt_owner_submit_prepared_prompt(
            &lease.backing_session_id,
            crate::session::PromptQueueItem::new(
                "retained-queued-prompt",
                &lease.backing_attachment_id,
                &lease.backing_agent_id,
                "preserve this queued work",
                crate::session::PromptStatus::Queued,
            ),
            true,
        )
        .unwrap();
    }
    let queued_before = app
        .prompt_owner_peek_next_queued_prompt(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap()
        .map(|prompt| serde_json::to_value(prompt).unwrap());
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let result = router
        .relay_submit_leased_prompt(
            &lease.id,
            profile.clone(),
            None,
            "use my selected account",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await;
    let mut app = app.lock().await;
    if let Some(queued) = queued_work {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("differs from the selected profile"));
        assert_eq!(
            app.providers().get_run(stale.id()).unwrap().state(),
            stale.state()
        );
        if queued {
            assert_eq!(
                serde_json::to_value(
                    app.prompt_owner_peek_next_queued_prompt(
                        &lease.backing_session_id,
                        &lease.backing_agent_id,
                    )
                    .unwrap()
                    .unwrap()
                )
                .unwrap(),
                queued_before.unwrap()
            );
        } else {
            assert_eq!(
                app.prompt_owner_active_prompt_for_agent(
                    &lease.backing_session_id,
                    &lease.backing_agent_id,
                )
                .unwrap()
                .unwrap()
                .id(),
                "active-prompt"
            );
        }
        return;
    }
    let (run_id, _) = result.unwrap();
    let actual = app.providers().get_run(&run_id).unwrap();
    assert_eq!(actual.provider(), profile.provider);
    assert_eq!(actual.model(), profile.model.as_deref().unwrap());
    assert_eq!(actual.variant(), profile.effort.as_deref());
    assert_eq!(
        actual.account_profile(),
        profile.account_profile,
        "a compatible tool catalog must not authorize reuse of a different account"
    );
    assert_ne!(run_id, stale.id());
    assert!(
        actual.resume_state().is_empty(),
        "a new account must not inherit the old account's provider session"
    );
    assert_eq!(
        app.providers().get_run(stale.id()).unwrap().state(),
        crate::provider::ProviderRunState::Ended
    );
}

#[tokio::test]
async fn leased_prompt_admission_prevents_interleaved_profile_changes() {
    let mut config = crate::config::DaemonConfig::for_tests();
    // Hold provider initialization so a successful admission remains queued.
    config.provider_runtime_init_delay_ms = 60_000;
    let (app, lease) = leased_agent_fixture_with_config(false, config);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let app_guard = app.lock().await;
    let submission = router.relay_submit_leased_prompt(
        &lease.id,
        profile.clone(),
        None,
        "keep my admitted profile",
        "",
        Vec::new(),
        None,
        None,
        Vec::new(),
        None,
        crate::extension::RemoteExtensionManifest::default(),
        None,
    );
    tokio::pin!(submission);
    poll_fn(|cx| {
        assert!(submission.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let update = router.relay_update_leased_agent_profile(
        &lease.id,
        profile.provider.clone(),
        profile.account_profile.clone(),
        Some("interleaved-model".into()),
        Some("high".into()),
    );
    tokio::pin!(update);
    poll_fn(|cx| {
        assert!(update.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(app_guard);
    let (submission, update) = tokio::join!(submission, update);
    let (run_id, outcome) = submission.unwrap();
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    assert!(update.unwrap_err().to_string().contains("queued prompt"));
    let confirmed = router
        .relay_update_leased_agent_profile(
            &lease.id,
            profile.provider.clone(),
            profile.account_profile.clone(),
            profile.model.clone(),
            profile.effort.clone(),
        )
        .await
        .unwrap();
    assert_eq!(confirmed.account_profile, profile.account_profile);
    let app = app.lock().await;
    let run = app.providers().get_run(&run_id).unwrap();
    assert_eq!(run.model(), profile.model.as_deref().unwrap());
    assert_eq!(run.account_profile(), profile.account_profile);
    assert_ne!(run.state(), crate::provider::ProviderRunState::Ended);
}

#[tokio::test]
async fn cancelling_leased_prompt_admission_releases_profile_operations() {
    let (app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let app_guard = app.lock().await;
    let mut submission = Box::pin(router.relay_submit_leased_prompt(
        &lease.id,
        profile.clone(),
        None,
        "cancel before admission",
        "",
        Vec::new(),
        None,
        None,
        Vec::new(),
        None,
        crate::extension::RemoteExtensionManifest::default(),
        None,
    ));
    poll_fn(|cx| {
        assert!(submission.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(submission);
    drop(app_guard);
    let updated = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        router.relay_update_leased_agent_profile(
            &lease.id,
            profile.provider,
            profile.account_profile,
            Some("after-cancellation".into()),
            profile.effort,
        ),
    )
    .await
    .expect("cancelled admission must not retain its operation guard")
    .unwrap();
    assert_eq!(updated.model.as_deref(), Some("after-cancellation"));
    let mut app = app.lock().await;
    assert!(app
        .prompt_owner_active_prompt_for_agent(&lease.backing_session_id, &lease.backing_agent_id,)
        .unwrap()
        .is_none());
    assert!(app
        .prompt_owner_peek_next_queued_prompt(&lease.backing_session_id, &lease.backing_agent_id,)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn leased_prompt_reconciles_durable_home_profile_after_lost_ack_and_restart() {
    crate::test_support::isolated_env_test!();
    let (mut app, lease) = leased_agent_fixture(false);
    RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_profile(
            &lease.id,
            lease.provider.clone(),
            "home-selected-account".into(),
            lease.model.clone(),
            lease.effort.clone(),
        )
        .unwrap();
    let home_profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    // Model the only profile state retained by the home kernel across a crash:
    // its last committed durable selection. The pending transition and response
    // acknowledgement are deliberately absent after this contract round-trip.
    let durable_home_profile = serde_json::to_vec(&home_profile).unwrap();
    drop(home_profile);
    // The worker applies an update, but its acknowledgement never reaches home.
    RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_profile(
            &lease.id,
            lease.provider.clone(),
            "unacknowledged-worker-account".into(),
            Some("unacknowledged-model".into()),
            Some("high".into()),
        )
        .unwrap();
    let home_profile: crate::transport::relay_peer::RelayAgentExecutionProfile =
        serde_json::from_slice(&durable_home_profile).unwrap();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let (provider_run_id, _outcome) = router
        .relay_submit_leased_prompt(
            &lease.id,
            home_profile.clone(),
            None,
            "use the profile committed by home",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .unwrap();
    let app = app.lock().await;
    let run = app.providers().get_run(&provider_run_id).unwrap();
    assert_eq!(run.provider(), home_profile.provider);
    assert_eq!(run.account_profile(), home_profile.account_profile);
    assert_eq!(
        run.model(),
        home_profile.model.as_deref().unwrap(),
        "provider launch must use the model committed by home"
    );
    let actual = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    assert_eq!(
        actual, home_profile,
        "a prompt must not use an unacknowledged worker profile"
    );
}

// MP-08 / MP-10 / MP-11: a worker admission that fails after its provider run
// was assigned must settle the exact receipt. A receipt left in Dispatching is
// projected as a queued-steer phase, so home holds its prompt forever.
#[tokio::test]
async fn failed_leased_prompt_admission_after_run_assignment_settles_its_receipt() {
    crate::test_support::isolated_env_test!();
    let (mut app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    // Launch succeeds; the backing prompt admission that follows it fails.
    crate::app::KernelSessionService::new(&mut app)
        .detach(&lease.backing_attachment_id)
        .unwrap();
    let home_prompt_id = "home-prompt-admission-failure";
    let git_context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "home-session".to_string(),
        home_agent_id: lease.home_agent_id.clone(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: home_prompt_id.to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "admission failure".to_string(),
    };
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let submit = || {
        router.relay_submit_leased_prompt(
            &lease.id,
            profile.clone(),
            None,
            "call the capability loader",
            "",
            Vec::new(),
            None,
            Some(git_context.clone()),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
    };
    let admission_error = submit().await.unwrap_err();
    assert!(
        app.lock()
            .await
            .providers()
            .get_run_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
            .is_some(),
        "the fixture must fail after the worker assigned a provider run"
    );
    assert_eq!(
        router
            .relay_query_leased_prompt_receipt(&lease.id, home_prompt_id)
            .await
            .unwrap(),
        Some(crate::transport::relay_peer::LeasedPromptReceipt {
            home_prompt_id: home_prompt_id.to_string(),
            worker_provider_run_id: String::new(),
            phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected,
            target_home_prompt_id: None,
            execution_lease_id: Some(lease.lease_id.clone()),
        }),
        "home settles this exact shape as an unadmitted prompt; admission error: {admission_error}"
    );
    // The settled receipt fences the same home prompt from a second delivery.
    assert!(submit()
        .await
        .unwrap_err()
        .to_string()
        .contains("worker rejection receipt fences this home prompt"));
    assert!(app
        .lock()
        .await
        .prompt_owner_active_prompt_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap()
        .is_none());
}

// MP-08 / MP-10 / MP-11: a leased Claude replica runs on the home's per-launch
// credential; the worker's own Claude profile is never logged in.
#[tokio::test]
async fn leased_claude_prompt_admits_on_the_home_credential_run_without_a_worker_login() {
    assert_leased_claude_home_credential(false).await;
}

#[tokio::test]
async fn leased_claude_environment_change_cannot_relaunch_under_worker_account() {
    assert_leased_claude_home_credential(true).await;
}

async fn assert_leased_claude_home_credential(environment_changed: bool) {
    crate::test_support::isolated_env_test!();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let execution_lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &execution_lease.id,
            "claude-p",
            "default",
            Some("haiku".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let submit = || {
        router.relay_submit_leased_prompt(
            &lease.id,
            profile.clone(),
            None,
            "call the capability loader",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
    };
    // Without a live run, the worker still requires the home's credential.
    assert!(submit()
        .await
        .unwrap_err()
        .to_string()
        .contains(crate::transport::relay_peer::REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE));
    assert!(app
        .lock()
        .await
        .providers()
        .get_run_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
        .is_none());
    // The live run a credentialed lease launch produced for the admitted profile.
    let mut worker = app.lock().await;
    let admitted = worker.leased_agents.get(&lease.id).cloned().unwrap();
    let mut request = crate::provider::LaunchProviderRequest::new(
        &lease.backing_session_id,
        "dev-stub",
        &admitted.provider,
        &admitted.account_profile,
        "haiku",
    )
    .with_agent_id(&lease.backing_agent_id)
    .with_variant(admitted.effort.clone());
    if environment_changed {
        request.project_environment_revision = Some("old-project-revision".into());
    }
    let run = worker
        .providers_mut()
        .launch_run_detached(request.clone())
        .unwrap();
    let selected_agent = worker.agents().get_agent(&lease.backing_agent_id).unwrap();
    assert!(
        worker
            .reusable_leased_claude_run_for_agent(&lease.backing_session_id, &selected_agent)
            .unwrap()
            .is_none(),
        "an unmarked worker run is not proof of HOME credential authority"
    );
    worker
        .provider_run_projection
        .mark_leased_provider_run(run.id());
    assert!(
        worker
            .reusable_leased_claude_run_for_agent(&lease.backing_session_id, &selected_agent)
            .unwrap()
            .is_some(),
        "the admitted HOME-bound run is reusable"
    );
    if environment_changed {
        // MP-08/MP-10/MP-11: a prompt admitted before the Project change keeps
        // the live run that holds HOME's credential; nothing is deferred.
        assert_eq!(
            worker
                .ensure_prompt_provider_run_for_agent(
                    &lease.backing_session_id,
                    &lease.backing_agent_id,
                )
                .expect("the admitted prompt keeps the HOME-credentialed run"),
            run.id()
        );
        worker
            .prompt_owner_submit_prepared_prompt(
                &lease.backing_session_id,
                crate::session::PromptQueueItem::new(
                    "queued-after-project-refresh",
                    &lease.backing_attachment_id,
                    &lease.backing_agent_id,
                    "keep the HOME account",
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
        // Two pump ticks: queue recovery, then the HOME-bound projection drain.
        // (The stub run has no process, so the pump's liveness probe is skipped.)
        let mut deferrals = 0;
        for _ in 0..2 {
            let mut runtime = RemoteLeaseRuntime::new(&mut worker);
            // The stub completes an activated prompt at once; only deferral matters.
            let _ = runtime.recover_idle_leased_prompt_queue(&lease.id);
            if let Some((_, event)) = runtime
                .drain_leased_runtime_projection(&lease.id, run.id(), false)
                .unwrap()
            {
                let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                    notices,
                    ..
                } = event;
                deferrals += notices
                    .iter()
                    .filter(|notice| notice.contains("Deferred queued prompt"))
                    .count();
            }
        }
        assert_eq!(deferrals, 0, "the pump must not re-defer the queued prompt");
        assert!(worker
            .prompt_owner_peek_next_queued_prompt(
                &lease.backing_session_id,
                &lease.backing_agent_id
            )
            .unwrap()
            .is_none());
        // A new prompt for the changed inputs is refused while such a run is
        // busy, so it never queues behind a run the worker cannot relaunch.
        // (The stub ended the first run when its prompt completed.)
        let run = worker.providers_mut().launch_run_detached(request).unwrap();
        worker
            .provider_run_projection
            .mark_leased_provider_run(run.id());
        worker
            .prompt_owner_sync_external_active_prompt(
                &lease.backing_session_id,
                &lease.backing_agent_id,
                Some(crate::session::PromptQueueItem::new(
                    "active-prompt",
                    &lease.backing_attachment_id,
                    &lease.backing_agent_id,
                    "active prompt",
                    crate::session::PromptStatus::Running,
                )),
            )
            .unwrap();
        let admitted = worker.leased_agents.get(&lease.id).cloned().unwrap();
        let busy = RemoteLeaseRuntime::new(&mut worker).prepare_leased_provider_run_matches_mcps(
            &admitted,
            &[],
            &crate::extension::RemoteExtensionManifest::default(),
        );
        assert!(
            matches!(
                busy,
                Err(DaemonError::LocalTransport {
                    operation: "remote Project environment reload",
                    ..
                })
            ),
            "{:?}",
            busy.err()
        );
        assert_eq!(
            worker.providers.get_run(run.id()).unwrap().state(),
            crate::provider::ProviderRunState::Running
        );
        return;
    }
    drop(worker);
    let (run_id, outcome) = submit().await.unwrap();
    assert_eq!(run_id, run.id());
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
}

// MP-08 / MP-10 / MP-11: Target worker re-resolves before its Ready shortcut.
#[test]
fn mp08_mp10_mp11_leased_reuse_observes_worker_value_changes() {
    assert_leased_project_launch_and_refresh(false);
}

// MP-08 / MP-10 / MP-11: import uses mounted paths, not the lease session alias.
#[test]
fn mp08_mp10_mp11_imported_workspace_launch_bindings_and_refresh() {
    assert_leased_project_launch_and_refresh(true);
}

fn assert_leased_project_launch_and_refresh(imported_workspace: bool) {
    use crate::app::remote_lease::provider_run::LeasedProviderRunMatch;
    use crate::project_environment::*;
    let workspace = crate::test_support::TestWorktree::new("leased-project-refresh");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let execution = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &execution.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".into()),
            None,
            None,
            None,
            None,
            Some(workspace.path().to_string_lossy().into_owned()),
            None,
        )
        .unwrap();
    let initial = app.sessions.get_session(&lease.backing_session_id).unwrap();
    let manifest_workspace = if imported_workspace {
        workspace.path().to_string_lossy().into_owned()
    } else {
        initial.workspace_id().to_string()
    };
    let mut workspace_ids = vec![initial.workspace_id().into()];
    if manifest_workspace != initial.workspace_id() {
        workspace_ids.push(manifest_workspace.clone());
    }
    let project = app
        .sessions
        .read()
        .prepare_leased_project(initial.id(), "envlayer5-leased-project", workspace_ids)
        .unwrap();
    let session = app
        .sessions
        .write()
        .bind_leased_project(initial.id(), project)
        .unwrap();
    let evidence = ProjectEnvironmentEvidence::default();
    let state = StoredProjectEnvironment {
        apply_owned_bindings: Default::default(),
        source: None,
        evidence: evidence.clone(),
        reported_missing: Default::default(),
        reviewed_manifest: None,
        last_review: None,
        manifest: ProjectEnvironmentManifest {
            schema_version: 1,
            project_id: session.project_id().into(),
            evidence_digest: evidence.digest(),
            entries: vec![ProjectEnvironmentEntry {
                name: "APP_LABEL".into(),
                workspace_id: manifest_workspace,
                kind: ProjectEnvironmentEntryKind::Variable,
                classification: ProjectEnvironmentClassification::NonSecret,
                excluded: false,
                uses: vec![ProjectEnvironmentUse {
                    path: "app.ts".into(),
                    line: 1,
                }],
                locator: ProjectEnvironmentLocator::EnvFile {
                    path: ".env".into(),
                    key: "APP_LABEL".into(),
                },
                status: ProjectEnvironmentEntryStatus::Found,
            }],
            private_files: vec![],
            toolchain_hints: vec![],
            package_hints: vec![],
            service_hints: vec![],
        },
    };
    ProjectEnvironmentStore::new(&app.config.private_runtime_state_root())
        .save(&state)
        .unwrap();
    std::fs::write(workspace.path().join(".env"), "APP_LABEL=first\n").unwrap();
    let manifest = crate::extension::RemoteExtensionManifest::default();
    let select = |app: &mut DaemonApp| {
        RemoteLeaseRuntime::new(app)
            .prepare_leased_provider_run_matches_mcps(&lease, &[], &manifest)
            .unwrap()
    };
    let LeasedProviderRunMatch::LaunchRequired(request) = select(&mut app) else {
        panic!("initial launch")
    };
    let request = app
        .prepare_app_provider_launch_request(request, "MP-08 / MP-10 / MP-11 regression")
        .unwrap();
    assert!(
        request
            .provider_credential_env
            .iter()
            .any(|(name, value)| name == "APP_LABEL" && value == "first"),
        "imported Project variable must reach the initial provider launch"
    );
    let first = app.launch_provider_detached(request).unwrap();
    assert!(matches!(select(&mut app), LeasedProviderRunMatch::Ready(id) if id == first.id()));
    std::fs::write(workspace.path().join(".env"), "APP_LABEL=second\n").unwrap();
    let LeasedProviderRunMatch::LaunchRequired(request) = select(&mut app) else {
        panic!("changed values must relaunch")
    };
    assert_eq!(
        app.providers.get_run(first.id()).unwrap().state(),
        ProviderRunState::Ended
    );
    let request = app
        .prepare_app_provider_launch_request(request, "MP-08 / MP-10 / MP-11 regression")
        .unwrap();
    assert!(request
        .provider_credential_env
        .iter()
        .any(|(name, value)| name == "APP_LABEL" && value == "second"));
    let second = app.launch_provider_detached(request).unwrap();
    assert_ne!(
        first.project_environment_revision(),
        second.project_environment_revision()
    );
}

#[tokio::test]
async fn fresh_leased_prompt_receives_home_resume_state() {
    crate::test_support::isolated_env_test!();
    let (app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let saved = crate::provider::ProviderResumeState::from_codex_thread_id("archived-home-thread");
    let (run, _) = router
        .relay_submit_leased_prompt(
            &lease.id,
            profile,
            Some(saved.clone()),
            "resume after archive",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        app.lock()
            .await
            .providers()
            .get_run(&run)
            .unwrap()
            .resume_state(),
        &saved
    );
}

#[test]
fn fresh_leased_agent_resume_state_clears_stale_backing_thread_and_preserves_live_worker() {
    crate::test_support::isolated_env_test!();
    let (mut app, lease) = leased_agent_fixture(false);
    let worker_state = crate::provider::ProviderResumeState::from_codex_thread_id("worker-thread");
    app.agents_mut()
        .set_agent_provider_resume_state(&lease.backing_agent_id, worker_state.clone())
        .unwrap();
    RemoteLeaseRuntime::new(&mut app)
        .seed_leased_agent_resume_state(&lease.id, Default::default())
        .unwrap();
    assert!(app
        .agents()
        .get_agent(&lease.backing_agent_id)
        .unwrap()
        .provider_resume_state()
        .is_empty());
    let manifest = crate::extension::RemoteExtensionManifest::default();
    let LeasedProviderRunMatch::LaunchRequired(request) = RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_provider_run_matches_mcps(&lease, &[], &manifest)
        .unwrap()
    else {
        panic!("fresh launch")
    };
    assert!(request.resume_state.is_none());
    let run = app
        .providers_mut()
        .launch_run_detached(request.with_resume_state(worker_state.clone()))
        .unwrap();
    RemoteLeaseRuntime::new(&mut app)
        .seed_leased_agent_resume_state(
            &lease.id,
            crate::provider::ProviderResumeState::from_codex_thread_id("stale-home-thread"),
        )
        .unwrap();
    assert_eq!(
        app.providers().get_run(run.id()).unwrap().resume_state(),
        &worker_state
    );
}
