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
fn daemon_run_starts_discovery_but_not_attached_transcript_observation() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/app/daemon_lifecycle.rs"),
    )
    .expect("source should be readable");
    let run_start = source
        .find("pub async fn run(self) -> Result<(), DaemonError>")
        .expect("daemon run should exist");
    let run_source = &source[run_start..];

    assert!(
        run_source.contains("run_external_provider_session_discovery_poller"),
        "daemon run should start external provider session discovery"
    );
    assert!(
        run_source.contains(
            "Attached external provider transcript observation intentionally does not run"
        ),
        "daemon run should document why attached provider transcript observation is absent"
    );
    assert!(
        !run_source.contains("run_attached_provider_transcript_observer"),
        "daemon run must not start attached provider transcript observation"
    );
}

#[test]
fn attached_provider_transcript_observation_runtime_loop_is_removed() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/runtime/external_provider_session_control.rs"),
    )
    .expect("source should be readable");
    assert!(
        !source.contains("run_attached_provider_transcript_observer"),
        "attached transcript observation must not have a daemon polling loop"
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
fn session_bounded_refresh_imports_history_without_runtime_activity() {
    let _guard = crate::env_lock::lock();
    let codex_home = temp_root("codex-attach-catchup");
    let previous_codex_home = env::var_os("CODEX_HOME");
    env::set_var("CODEX_HOME", &codex_home);
    let session_dir = codex_home.join("sessions");
    fs::create_dir_all(&session_dir).expect("codex session dir should create");
    let transcript = session_dir.join("attach-catchup.jsonl");
    fs::write(
            &transcript,
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
        assert!(
            !crate::app::external_provider_session_transcript_needs_refresh(
                "codex",
                "thread-attach-catchup",
            ),
            "the observed transcript fingerprint should be current after catch-up",
        );

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .expect("codex transcript should open");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-01-01T00:00:03.000Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"a2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"caught up after attach reload\"}}]}}}}"
        )
        .expect("codex transcript update should write");
        assert!(crate::app::external_provider_session_transcript_needs_refresh(
            "codex",
            "thread-attach-catchup",
        ));

        refresh_attached_external_provider_histories_for_session(&app, None, &session_id).await;
        assert!(
            !crate::app::external_provider_session_transcript_needs_refresh(
                "codex",
                "thread-attach-catchup",
            ),
            "session-bounded catch-up should reread a changed transcript",
        );

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
        assert!(entries
            .iter()
            .any(|entry| entry.text == "catch up this attached thread"));
        assert!(entries.iter().any(|entry| entry.text == "caught up"));
        assert!(entries
            .iter()
            .any(|entry| entry.text == "caught up after attach reload"));
        let agent = app
            .agents()
            .get_agent(&agent_id)
            .expect("agent should load");
        let (active_prompt, queued_prompts) =
            app.prompt_state_owner().state_parts(&session, agent.id());
        assert!(
            active_prompt.is_none(),
            "history catch-up must not create active prompt state"
        );
        assert!(
            queued_prompts.is_empty(),
            "history catch-up must not create queued prompt state"
        );
    });

    restore_env_var("CODEX_HOME", previous_codex_home);
}

#[test]
fn discovery_content_change_refreshes_attached_imported_history() {
    let _guard = crate::env_lock::lock();
    let root = temp_root("opencode-discovery-content-refresh");
    let previous_home = env::var_os("HOME");
    let previous_xdg_data_home = env::var_os("XDG_DATA_HOME");
    let previous_codex_home = env::var_os("CODEX_HOME");
    let previous_claude_home = env::var_os("CLAUDE_HOME");
    let previous_opencode_data_home = env::var_os("OPENCODE_DATA_HOME");
    env::set_var("HOME", root.join("home"));
    env::set_var("XDG_DATA_HOME", root.join("xdg-data"));
    env::set_var("CODEX_HOME", root.join("codex"));
    env::set_var("CLAUDE_HOME", root.join("claude"));
    env::set_var("OPENCODE_DATA_HOME", root.join("opencode"));
    let session_dir = root.join("opencode").join("sessions");
    fs::create_dir_all(&session_dir).expect("opencode session dir should create");
    let transcript = session_dir.join("open-content-refresh.json");
    fs::write(
        &transcript,
        r#"{
          "id": "open-content-refresh",
          "title": "OpenCode content refresh",
          "cwd": "/tmp/open-content-refresh",
          "updatedAt": "2026-03-01T00:00:01.000Z",
          "messages": [
            {
              "id": "opencode-user-1",
              "role": "user",
              "content": "OpenCode initial prompt",
              "createdAt": "2026-03-01T00:00:01.000Z"
            },
            {
              "id": "opencode-assistant-1",
              "role": "assistant",
              "content": "OpenCode initial reply",
              "createdAt": "2026-03-01T00:00:02.000Z"
            }
          ]
        }"#,
    )
    .expect("opencode transcript should write");

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
            persist_external_import_metadata(
                &mut app,
                session.id(),
                agent.id(),
                ExternalProviderImportMetadata::observed_history(
                    "opencode:open-content-refresh".to_string(),
                    "opencode".to_string(),
                    "open-content-refresh".to_string(),
                ),
            )
            .expect("external import metadata should persist");
            attach_test_session(&app, session.id());
            (session.id().to_string(), agent.id().to_string())
        };

        let mut cache = ExternalProviderSessionDiscoveryCache::default();
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;
        fs::write(
            &transcript,
            r#"{
              "id": "open-content-refresh",
              "title": "OpenCode content refresh",
              "cwd": "/tmp/open-content-refresh",
              "updatedAt": "2026-03-01T00:00:03.000Z",
              "messages": [
                {
                  "id": "opencode-user-1",
                  "role": "user",
                  "content": "OpenCode initial prompt",
                  "createdAt": "2026-03-01T00:00:01.000Z"
                },
                {
                  "id": "opencode-assistant-1",
                  "role": "assistant",
                  "content": "OpenCode initial reply",
                  "createdAt": "2026-03-01T00:00:02.000Z"
                },
                {
                  "info": {
                    "id": "opencode-user-2",
                    "sessionID": "open-content-refresh",
                    "role": "user"
                  },
                  "parts": [{
                    "id": "opencode-user-2-text",
                    "type": "text",
                    "text": "OpenCode appended prompt from content refresh"
                  }]
                }
              ]
            }"#,
        )
        .expect("opencode transcript update should write");
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;

        let app_guard = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        let session = app_guard
            .sessions()
            .get_session(&session_id)
            .expect("session should load");
        let entries = app_guard
            .load_session_history_entries(&session, Some(&agent_id))
            .expect("history should load");
        assert!(entries
            .iter()
            .any(|entry| entry.text == "OpenCode appended prompt from content refresh"));
        drop(app_guard);

        cache.cached_signature_checks = EXTERNAL_PROVIDER_DISCOVERY_FULL_SCAN_AFTER_CACHED_CHECKS;
        fs::write(
            &transcript,
            r#"{
              "id": "open-content-refresh",
              "title": "OpenCode content refresh",
              "cwd": "/tmp/open-content-refresh",
              "updatedAt": "2026-03-01T00:00:05.000Z",
              "messages": [
                {
                  "id": "opencode-user-1",
                  "role": "user",
                  "content": "OpenCode initial prompt",
                  "createdAt": "2026-03-01T00:00:01.000Z"
                },
                {
                  "id": "opencode-assistant-1",
                  "role": "assistant",
                  "content": "OpenCode initial reply",
                  "createdAt": "2026-03-01T00:00:02.000Z"
                },
                {
                  "info": {
                    "id": "opencode-user-2",
                    "sessionID": "open-content-refresh",
                    "role": "user"
                  },
                  "parts": [{
                    "id": "opencode-user-2-text",
                    "type": "text",
                    "text": "OpenCode appended prompt from content refresh"
                  }]
                },
                {
                  "info": {
                    "id": "opencode-assistant-2",
                    "sessionID": "open-content-refresh",
                    "role": "assistant",
                    "finish": "tool-calls"
                  },
                  "parts": [{
                    "id": "opencode-reasoning-2",
                    "type": "reasoning",
                    "text": "OpenCode appended reasoning from full scan refresh"
                  }]
                }
              ]
            }"#,
        )
        .expect("opencode transcript update should write");
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;

        let app_guard = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        let session = app_guard
            .sessions()
            .get_session(&session_id)
            .expect("session should load");
        let entries = app_guard
            .load_session_history_entries(&session, Some(&agent_id))
            .expect("history should load");
        assert!(entries
            .iter()
            .any(|entry| entry.text == "OpenCode appended reasoning from full scan refresh"));
    });

    restore_env_var("HOME", previous_home);
    restore_env_var("XDG_DATA_HOME", previous_xdg_data_home);
    restore_env_var("CODEX_HOME", previous_codex_home);
    restore_env_var("CLAUDE_HOME", previous_claude_home);
    restore_env_var("OPENCODE_DATA_HOME", previous_opencode_data_home);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn codex_discovery_content_change_refreshes_attached_imported_history() {
    let _guard = crate::env_lock::lock();
    let root = temp_root("codex-discovery-content-refresh");
    let previous_home = env::var_os("HOME");
    let previous_xdg_data_home = env::var_os("XDG_DATA_HOME");
    let previous_codex_home = env::var_os("CODEX_HOME");
    let previous_claude_home = env::var_os("CLAUDE_HOME");
    let previous_opencode_data_home = env::var_os("OPENCODE_DATA_HOME");
    env::set_var("HOME", root.join("home"));
    env::set_var("XDG_DATA_HOME", root.join("xdg-data"));
    env::set_var("CODEX_HOME", root.join("codex"));
    env::set_var("CLAUDE_HOME", root.join("claude"));
    env::set_var("OPENCODE_DATA_HOME", root.join("opencode"));
    let session_dir = root.join("codex").join("sessions");
    fs::create_dir_all(&session_dir).expect("codex session dir should create");
    let transcript = session_dir.join("codex-content-refresh.jsonl");
    fs::write(
        &transcript,
        concat!(
            "{\"timestamp\":\"2026-06-09T12:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-content-refresh\",\"cwd\":\"/tmp/codex-content-refresh\",\"model_provider\":\"openai\"}}\n",
            "{\"timestamp\":\"2026-06-09T12:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"codex-user-1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Codex initial prompt\"}]}}\n",
            "{\"timestamp\":\"2026-06-09T12:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"codex-assistant-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Codex initial reply\"}]}}\n",
        ),
    )
    .expect("codex transcript should write");

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
            persist_external_import_metadata(
                &mut app,
                session.id(),
                agent.id(),
                ExternalProviderImportMetadata::observed_history(
                    "codex:codex-content-refresh".to_string(),
                    "codex".to_string(),
                    "codex-content-refresh".to_string(),
                ),
            )
            .expect("external import metadata should persist");
            attach_test_session(&app, session.id());
            (session.id().to_string(), agent.id().to_string())
        };

        let mut cache = ExternalProviderSessionDiscoveryCache::default();
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .expect("codex transcript should open");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-09T20:37:27.547Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"id\":\"codex-user-2\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Codex appended prompt from content refresh\"}}]}}}}"
        )
        .expect("codex transcript update should write");
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-09T20:37:29.547Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"codex-reasoning-2\",\"type\":\"reasoning\",\"summary\":[{{\"type\":\"summary_text\",\"text\":\"Codex appended reasoning from second content refresh\"}}]}}}}"
        )
        .expect("codex reasoning update should write");
        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;

        let app_guard = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        let session = app_guard
            .sessions()
            .get_session(&session_id)
            .expect("session should load");
        let entries = app_guard
            .load_session_history_entries(&session, Some(&agent_id))
            .expect("history should load");
        assert!(entries
            .iter()
            .any(|entry| entry.text == "Codex appended prompt from content refresh"));
        assert!(entries
            .iter()
            .any(|entry| entry.text == "Codex appended reasoning from second content refresh"));
    });

    restore_env_var("HOME", previous_home);
    restore_env_var("XDG_DATA_HOME", previous_xdg_data_home);
    restore_env_var("CODEX_HOME", previous_codex_home);
    restore_env_var("CLAUDE_HOME", previous_claude_home);
    restore_env_var("OPENCODE_DATA_HOME", previous_opencode_data_home);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unchanged_discovery_signature_refreshes_attached_imported_history() {
    let _guard = crate::env_lock::lock();
    let root = temp_root("codex-unchanged-signature-attached-refresh");
    let previous_home = env::var_os("HOME");
    let previous_xdg_data_home = env::var_os("XDG_DATA_HOME");
    let previous_codex_home = env::var_os("CODEX_HOME");
    let previous_claude_home = env::var_os("CLAUDE_HOME");
    let previous_opencode_data_home = env::var_os("OPENCODE_DATA_HOME");
    env::set_var("HOME", root.join("home"));
    env::set_var("XDG_DATA_HOME", root.join("xdg-data"));
    env::set_var("CODEX_HOME", root.join("codex"));
    env::set_var("CLAUDE_HOME", root.join("claude"));
    env::set_var("OPENCODE_DATA_HOME", root.join("opencode"));
    let session_dir = root.join("codex").join("sessions");
    fs::create_dir_all(&session_dir).expect("codex session dir should create");
    let transcript = session_dir.join("codex-hidden-from-signature.jsonl");
    fs::write(
        &transcript,
        concat!(
            "{\"timestamp\":\"2026-06-09T12:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-hidden-from-signature\",\"cwd\":\"/tmp/codex-hidden-from-signature\",\"model_provider\":\"openai\"}}\n",
            "{\"timestamp\":\"2026-06-09T12:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"codex-user-1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Codex hidden initial prompt\"}]}}\n",
        ),
    )
    .expect("codex transcript should write");
    let decoy = root.join("unchanged-signature-decoy.jsonl");
    fs::write(&decoy, "{}\n").expect("decoy signature file should write");

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
            persist_external_import_metadata(
                &mut app,
                session.id(),
                agent.id(),
                ExternalProviderImportMetadata::observed_history(
                    "codex:codex-hidden-from-signature".to_string(),
                    "codex".to_string(),
                    "codex-hidden-from-signature".to_string(),
                ),
            )
            .expect("external import metadata should persist");
            attach_test_session(&app, session.id());
            (session.id().to_string(), agent.id().to_string())
        };

        refresh_attached_external_provider_histories_for_session(&app, None, &session_id).await;
        let candidate_paths = vec![("codex".to_string(), decoy.clone())];
        let signature =
            crate::app::external_provider_session_discovery_signature_for_candidates(
                &candidate_paths,
            );
        let mut cache = ExternalProviderSessionDiscoveryCache {
            signature: Some(signature),
            candidate_paths: Some(candidate_paths),
            cached_signature_checks: 0,
        };
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .expect("codex transcript should open");
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-09T20:37:27.547Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"id\":\"codex-user-2\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Codex appended prompt despite unchanged signature\"}}]}}}}"
        )
        .expect("codex transcript update should write");

        refresh_external_provider_session_index(&app, None, Some(&mut cache), false).await;

        let app_guard = crate::runtime::app_lock::lock_app_instrumented(
            &app,
            "external_provider_session_control",
        )
        .await;
        let session = app_guard
            .sessions()
            .get_session(&session_id)
            .expect("session should load");
        let entries = app_guard
            .load_session_history_entries(&session, Some(&agent_id))
            .expect("history should load");
        assert!(entries
            .iter()
            .any(|entry| entry.text == "Codex appended prompt despite unchanged signature"));
    });

    restore_env_var("HOME", previous_home);
    restore_env_var("XDG_DATA_HOME", previous_xdg_data_home);
    restore_env_var("CODEX_HOME", previous_codex_home);
    restore_env_var("CLAUDE_HOME", previous_claude_home);
    restore_env_var("OPENCODE_DATA_HOME", previous_opencode_data_home);
    let _ = fs::remove_dir_all(root);
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
