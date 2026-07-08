use super::*;

#[test]
fn attached_observer_refresh_filters_normalize_provider_ids() {
    let target = observer_target("agent-1");

    assert!(attached_external_observer_target_matches_refresh_filters(
        &target,
        Some(" Codex "),
        None,
    ));
    assert!(attached_external_observer_target_matches_refresh_filters(
        &target,
        Some("CODEX"),
        Some("session-agent-1"),
    ));
    assert!(!attached_external_observer_target_matches_refresh_filters(
        &target,
        Some("claude"),
        None,
    ));
    assert!(!attached_external_observer_target_matches_refresh_filters(
        &target,
        Some("unknown"),
        None,
    ));
    assert!(!attached_external_observer_target_matches_refresh_filters(
        &target,
        Some("codex"),
        Some("session-other"),
    ));
}

#[test]
fn external_provider_discovery_poller_is_not_demand_gated() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/runtime/external_provider_session_control.rs"),
    )
    .expect("source should be readable");
    let start = source
        .find("pub(crate) async fn run_external_provider_session_discovery_poller")
        .expect("discovery poller should exist");
    let end = source[start..]
        .find("#[derive(Debug, Clone)]\nstruct AttachedExternalObserverTarget")
        .map(|offset| start + offset)
        .expect("poller block should end before observer target");
    let poller_source = &source[start..end];

    assert!(
        !poller_source.contains("external_provider_session_discovery_has_demand"),
        "external provider discovery must not be demand gated"
    );
    assert!(
        !poller_source.contains("attached_external_provider_session_refs"),
        "external provider discovery must run without existing attached targets"
    );
    assert!(
        poller_source
            .matches("refresh_external_provider_session_index")
            .count()
            >= 2,
        "discovery poller should refresh before the loop and on interval ticks"
    );
}

#[test]
fn due_attached_external_observer_targets_prioritizes_overdue_targets() {
    let now = tokio::time::Instant::now();
    let mut schedule = BTreeMap::new();
    for agent in ["a", "b"] {
        schedule.insert(
            attached_observer_target_key(&observer_target(agent)),
            AttachedExternalObserverSchedule {
                next_due_at: now,
                active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
                last_changed_at: None,
                consecutive_errors: 0,
            },
        );
    }
    for agent in ["c", "d"] {
        schedule.insert(
            attached_observer_target_key(&observer_target(agent)),
            AttachedExternalObserverSchedule {
                next_due_at: now - Duration::from_secs(10),
                active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
                last_changed_at: None,
                consecutive_errors: 0,
            },
        );
    }

    let due = due_attached_external_observer_targets(
        ["a", "b", "c", "d"]
            .into_iter()
            .map(observer_target)
            .collect(),
        &mut schedule,
        now,
        2,
    );

    assert_eq!(
        due.iter()
            .map(|target| target.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "d"]
    );
}

#[test]
fn resume_state_without_attachment_or_running_run_is_not_observed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    app.agents()
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-test".to_string()),
            None,
            ProviderResumeState::from_codex_thread_id("thread-idle"),
        )
        .expect("agent runtime profile should update");

    assert!(
        attached_external_observer_targets(&app).is_empty(),
        "idle persisted resume state must not create observer work"
    );

    let store = app.external_provider_session_index_store();
    store.upsert(record("codex", "thread-idle", "/tmp/thread-idle"));
    store.mark_attached("codex:thread-idle", session.id(), agent.id());
    mark_attached_external_provider_sessions(&app, None, &store);
    let indexed = store
        .get("codex:thread-idle")
        .expect("record should remain indexed");
    assert!(
        !indexed.is_attached_to_arroba(),
        "idle persisted resume state must not mark external sessions attached"
    );
    assert_eq!(indexed.first_attached_session_id(), None);
    assert_eq!(indexed.first_attached_agent_id(), None);
    assert_eq!(session.attachment_ids().len(), 0);
}

#[test]
fn attached_resume_state_is_observed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    app.agents()
        .set_agent_runtime_profile(
            agent.id(),
            "codex",
            Some("gpt-test".to_string()),
            None,
            ProviderResumeState::from_codex_thread_id("thread-attached"),
        )
        .expect("agent runtime profile should update");
    attach_test_session(&app, session.id());

    let target = single_attached_target(&app);

    assert_eq!(target.session_id, session.id());
    assert_eq!(target.agent_id, agent.id());
    assert_eq!(target.provider, "codex");
    assert_eq!(target.provider_session_id, "thread-attached");
}

#[test]
fn session_bounded_refresh_catches_up_after_attach() {
    let _guard = crate::env_lock::lock();
    let codex_home = temp_root("codex-attach-catchup");
    let previous_codex_home = env::var_os("CODEX_HOME");
    env::set_var("CODEX_HOME", &codex_home);
    let session_dir = codex_home.join("sessions");
    fs::create_dir_all(&session_dir).expect("codex session dir should create");
    fs::write(
            session_dir.join("attach-catchup.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-attach-catchup\",\"cwd\":\"/tmp/attach-catchup\",\"model_provider\":\"openai\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"catch up this attached thread\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"a1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"caught up\"}]}}\n",
            ),
        )
        .expect("codex session should write");

    let runtime = tokio::runtime::Runtime::new().expect("runtime should create");
    runtime.block_on(async {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot"),
        ));
        let (session_id, agent_id) = {
            let mut app = crate::runtime::app_lock::lock_app_instrumented(
                &app,
                "external_provider_session_control",
            )
            .await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.agents()
                .set_agent_runtime_profile(
                    agent.id(),
                    "codex",
                    Some("gpt-test".to_string()),
                    None,
                    ProviderResumeState::from_codex_thread_id("thread-attach-catchup"),
                )
                .expect("agent runtime profile should update");
            attach_test_session(&app, session.id());
            (session.id().to_string(), agent.id().to_string())
        };

        refresh_attached_external_provider_histories_for_session(&app, None, &session_id).await;

        let app = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        let session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should load");
        let entries = app
            .load_session_history_entries(&session, Some(&agent_id))
            .expect("history should load");
        assert!(
            entries
                .iter()
                .any(|entry| entry.text == "catch up this attached thread")
        );
        assert!(entries.iter().any(|entry| entry.text == "caught up"));
    });

    restore_env_var("CODEX_HOME", previous_codex_home);
}

#[test]
fn running_provider_run_without_attachment_is_observed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let run = test_codex_run(session.id(), agent.id(), "run-live", "thread-live");
    app.providers_mut().insert_run_for_test(run.clone());

    let target = single_attached_target(&app);

    assert_eq!(target.provider_run_id.as_deref(), Some(run.id()));
    assert_eq!(target.provider_session_id, "thread-live");
}

#[test]
fn starting_provider_run_without_attachment_is_observed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let starting_run =
        test_starting_codex_run(session.id(), agent.id(), "run-starting", "thread-starting");
    app.providers_mut()
        .insert_run_for_test(starting_run.clone());

    let target = single_attached_target(&app);

    assert_eq!(target.provider_run_id.as_deref(), Some(starting_run.id()));
    assert_eq!(target.provider_session_id, "thread-starting");
}

#[test]
fn parked_provider_run_without_attachment_is_not_observed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let mut run = test_codex_run(session.id(), agent.id(), "run-parked", "thread-parked");
    run.mark_parked();
    app.providers_mut().insert_run_for_test(run);

    assert!(
        attached_external_observer_targets(&app).is_empty(),
        "parked detached runs must not keep observer polling hot"
    );
}
