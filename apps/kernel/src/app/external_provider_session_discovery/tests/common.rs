use super::*;

#[test]
fn discovered_external_providers_have_observation_policy() {
    let discovered = discovered_external_provider_ids()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let configured =
        ExternalProviderObservationPolicy::configured_provider_ids().collect::<BTreeSet<_>>();

    assert_eq!(discovered, configured);
    for provider in discovered {
        assert!(
            ExternalProviderObservationPolicy::for_provider(provider).is_configured(),
            "{provider} discovery must have explicit observation policy"
        );
    }
}

#[test]
fn external_provider_filters_normalize_provider_ids() {
    assert!(provider_matches(None, "codex"));
    assert!(provider_matches(Some(" Codex "), "codex"));
    assert!(provider_matches(Some("CLAUDE"), "claude"));
    assert!(provider_matches(Some("OpenCode"), "opencode"));
    assert!(!provider_matches(Some("unknown"), "codex"));
    assert!(!provider_matches(Some(""), "codex"));
}

#[test]
fn observed_turn_model_derives_history_kind_and_external_keys() {
    let user = ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        provider_turn_id: Some("provider-user-1".to_string()),
        observed_at_ms: Some(1_000),
    };
    assert_eq!(
        user.role.session_history_kind(),
        SessionHistoryEntryKind::UserPrompt
    );
    assert_eq!(user.provider_turn_id_or_fallback(), "provider-user-1");
    assert_eq!(
        user.external_merge_key("codex", "thread-1"),
        "external:codex:thread-1:provider-user-1"
    );

    let tool = ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Tool,
        text: "tool output".to_string(),
        provider_turn_id: None,
        observed_at_ms: Some(1_100),
    };
    assert_eq!(
        tool.role.session_history_kind(),
        SessionHistoryEntryKind::ProviderTool
    );
    let fallback_id = tool.provider_turn_id_or_fallback();
    assert_eq!(fallback_id, "observed-v1-tool-d4be485e4ccf6617");
    assert_eq!(
        tool.external_merge_key("claude", "thread-2"),
        format!("external:claude:thread-2:{fallback_id}")
    );

    assert_eq!(
        ObservedExternalProviderTurnRole::Assistant.session_history_kind(),
        SessionHistoryEntryKind::ProviderOutput
    );
    assert_eq!(
        ObservedExternalProviderTurnRole::Reasoning.session_history_kind(),
        SessionHistoryEntryKind::ProviderReasoning
    );
    assert_eq!(
        ObservedExternalProviderTurnRole::Status.session_history_kind(),
        SessionHistoryEntryKind::ProviderStatus
    );
}

#[test]
fn discovers_codex_jsonl_sessions_with_first_real_prompt_title() {
    let temp = temp_dir("codex-discovery");
    let root = temp.path();
    let session_dir = root.join("archived_sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\",\"model_provider\":\"openai\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /repo\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix the broken JournalView build. It fails on duplicate state.\"}]}}\n",
            ),
        )
        .unwrap();

    let sessions = discover_codex_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_session_id, "codex:thread-1");
    assert_eq!(
        sessions[0].title.as_deref(),
        Some("Fix the broken JournalView build.")
    );
    assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
    assert_eq!(sessions[0].account_profile.as_deref(), Some("openai"));
    assert!(sessions[0].capabilities.can_read_history);
}

#[test]
fn file_candidate_collection_does_not_cap_before_recent_sort() {
    let temp = temp_dir("provider-file-cap");
    let root = temp.path();
    for index in 0..=MAX_PROVIDER_FILES {
        fs::write(root.join(format!("session-{index}.jsonl")), "{}\n").unwrap();
    }

    let candidates = jsonl_candidates(root, 1);

    assert_eq!(candidates.len(), MAX_PROVIDER_FILES + 1);
}

#[test]
fn signature_from_known_candidates_does_not_rescan_provider_roots() {
    let temp = temp_dir("provider-signature-known-candidates");
    let root = temp.path();
    let session_dir = root.join("sessions").join("2026").join("06");
    fs::create_dir_all(&session_dir).unwrap();
    let transcript = session_dir.join("rollout.jsonl");
    fs::write(
        &transcript,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-known\"}}\n",
    )
    .unwrap();

    reset_file_candidate_scan_count();
    let candidates = codex_candidate_paths(root);
    assert_eq!(candidates, vec![transcript.clone()]);
    assert!(file_candidate_scan_count() > 0);

    reset_file_candidate_scan_count();
    let signature = external_provider_session_discovery_signature_for_candidates(&[(
        "codex".to_string(),
        transcript.clone(),
    )]);
    assert_eq!(file_candidate_scan_count(), 0);
    assert_eq!(signature.files.len(), 1);
    assert_eq!(signature.files[0].provider, "codex");
    assert_eq!(signature.files[0].path, transcript);
}

#[test]
fn discovers_unchanged_jsonl_sessions_from_cached_records() {
    let temp = temp_dir("provider-discovery-record-cache");
    let root = temp.path();
    let codex_dir = root.join("codex").join("sessions");
    let claude_dir = root.join("claude").join("projects").join("-repo");
    let opencode_dir = root.join("opencode").join("sessions");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::create_dir_all(&claude_dir).unwrap();
    fs::create_dir_all(&opencode_dir).unwrap();
    let codex_transcript = codex_dir.join("codex-record-cache.jsonl");
    fs::write(
            &codex_transcript,
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-record-cache\",\"cwd\":\"/repo\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Cache Codex discovery summary.\"}]}}\n",
            ),
        )
        .unwrap();
    fs::write(
            claude_dir.join("claude-record-cache.jsonl"),
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u-record-cache\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Cache Claude discovery summary.\"}]},\"cwd\":\"/repo\",\"sessionId\":\"claude-record-cache\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
            ),
        )
        .unwrap();
    fs::write(
            opencode_dir.join("opencode-record-cache.jsonl"),
            concat!(
                "{\"sessionID\":\"opencode-record-cache\",\"id\":\"u-record-cache\",\"role\":\"user\",\"content\":\"Cache OpenCode discovery summary.\",\"createdAt\":\"2026-03-01T00:00:01.000Z\"}\n",
            ),
        )
        .unwrap();

    reset_jsonl_read_counts();
    assert_eq!(
        discover_codex_external_sessions(&root.join("codex")).len(),
        1
    );
    assert_eq!(
        discover_claude_external_sessions(&root.join("claude")).len(),
        1
    );
    assert_eq!(
        discover_opencode_external_sessions(&root.join("opencode")).len(),
        1
    );
    assert_eq!(jsonl_prefix_read_count(), 3);

    reset_jsonl_read_counts();
    assert_eq!(
        discover_codex_external_sessions(&root.join("codex")).len(),
        1
    );
    assert_eq!(
        discover_claude_external_sessions(&root.join("claude")).len(),
        1
    );
    assert_eq!(
        discover_opencode_external_sessions(&root.join("opencode")).len(),
        1
    );
    assert_eq!(jsonl_prefix_read_count(), 0);

    let mut file = OpenOptions::new()
        .append(true)
        .open(&codex_transcript)
        .unwrap();
    writeln!(
            file,
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"Codex discovery cache invalidated.\"}}]}}}}"
        )
        .unwrap();

    reset_jsonl_read_counts();
    assert_eq!(
        discover_codex_external_sessions(&root.join("codex")).len(),
        1
    );
    assert_eq!(jsonl_prefix_read_count(), 1);
}

#[test]
fn discovers_claude_project_transcripts() {
    let temp = temp_dir("claude-discovery");
    let root = temp.path();
    let session_dir = root.join("projects").join("-repo");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"timestamp\":\"2026-02-01T00:00:00.000Z\",\"sessionId\":\"session-1\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"text\":\"Summarize the import plan. Keep it brief.\"}]},\"cwd\":\"/repo\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
            ),
        )
        .unwrap();

    let sessions = discover_claude_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_session_id, "claude:session-1");
    assert_eq!(
        sessions[0].title.as_deref(),
        Some("Summarize the import plan.")
    );
    assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
}

#[test]
fn discovers_opencode_json_session_exports() {
    let temp = temp_dir("opencode-discovery");
    let root = temp.path();
    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
            session_dir.join("session-1.json"),
            r#"{"id":"open-1","title":"Investigate provider imports","cwd":"/repo","updatedAt":"2026-03-01T00:00:00.000Z"}"#,
        )
        .unwrap();

    let sessions = discover_opencode_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_session_id, "opencode:open-1");
    assert_eq!(
        sessions[0].title.as_deref(),
        Some("Investigate provider imports")
    );
    assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
}

#[test]
fn discovers_opencode_sqlite_sessions() {
    let temp = temp_dir("opencode-sqlite-discovery");
    let root = temp.path();
    let db_path = root.join("opencode.db");
    seed_opencode_sqlite(&db_path);

    let sessions = discover_opencode_external_sessions(root);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].external_session_id, "opencode:ses_sqlite_1");
    assert_eq!(
        sessions[0].title.as_deref(),
        Some("Investigate SQLite-backed OpenCode imports.")
    );
    assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo/sqlite"));
    assert!(sessions[0].last_modified_at_ms >= 1_782_113_000_000);
}

#[test]
fn discovery_signature_reports_only_changed_candidate_providers() {
    let left = ExternalProviderSessionDiscoverySignature {
        files: vec![
            ExternalProviderSessionFileSignature {
                provider: "claude".to_string(),
                path: PathBuf::from("claude-a.jsonl"),
                len: 1,
                modified_at_ms: 1,
            },
            ExternalProviderSessionFileSignature {
                provider: "codex".to_string(),
                path: PathBuf::from("codex-a.jsonl"),
                len: 1,
                modified_at_ms: 1,
            },
        ],
    };
    let right = ExternalProviderSessionDiscoverySignature {
        files: vec![
            ExternalProviderSessionFileSignature {
                provider: "claude".to_string(),
                path: PathBuf::from("claude-a.jsonl"),
                len: 2,
                modified_at_ms: 2,
            },
            ExternalProviderSessionFileSignature {
                provider: "codex".to_string(),
                path: PathBuf::from("codex-b.jsonl"),
                len: 1,
                modified_at_ms: 1,
            },
            ExternalProviderSessionFileSignature {
                provider: "opencode".to_string(),
                path: PathBuf::from("opencode.db"),
                len: 1,
                modified_at_ms: 1,
            },
        ],
    };

    assert_eq!(
        left.providers_with_changed_candidate_files(&right),
        BTreeSet::from(["codex".to_string(), "opencode".to_string()])
    );
}

#[test]
fn prompt_recovery_match_prefers_exact_worktree_and_tracks_recovery_anchor() {
    let sessions = vec![
        recovery_session("thread-other", "/repo/other", 30),
        recovery_session("thread-target", "/repo/target", 20),
    ];
    let matched = select_external_provider_prompt_recovery_match(
        "codex",
        "build the recovery path",
        Some("/repo/target"),
        Some("arroba-recovery:prompt-1:1"),
        sessions,
        |session_id| {
            let recovery = format!(
                "[Arroba recovery operation arroba-recovery:prompt-1:1] continue {session_id}"
            );
            vec![
                observed_turn(
                    ObservedExternalProviderTurnRole::User,
                    "build the recovery path",
                ),
                observed_turn(ObservedExternalProviderTurnRole::User, &recovery),
                observed_turn(ObservedExternalProviderTurnRole::Assistant, "resumed"),
                observed_turn(
                    ObservedExternalProviderTurnRole::Status,
                    "codex task_complete\n{}",
                ),
            ]
        },
    )
    .expect("matching provider session should be found");

    assert_eq!(matched.provider_session_id, "thread-target");
    assert!(matched.original_prompt_observed);
    assert!(matched.recovery_operation_observed);
    assert!(matched.provider_activity_after_anchor);
    assert!(matched.settled_after_anchor);
}

fn recovery_session(
    provider_session_id: &str,
    worktree_path: &str,
    last_modified_at_ms: u64,
) -> ExternalProviderSessionRecord {
    ExternalProviderSessionRecord {
        external_session_id: format!("codex:{provider_session_id}"),
        provider: "codex".to_string(),
        provider_session_id: provider_session_id.to_string(),
        title: None,
        title_source: None,
        first_prompt_preview: None,
        created_at_ms: None,
        last_modified_at_ms,
        worktree_path: Some(worktree_path.to_string()),
        account_profile: None,
        capabilities: ExternalProviderSessionCapabilities::default(),
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}

fn observed_turn(
    role: ObservedExternalProviderTurnRole,
    text: &str,
) -> ObservedExternalProviderTurn {
    ObservedExternalProviderTurn {
        role,
        text: text.to_string(),
        provider_turn_id: None,
        observed_at_ms: None,
    }
}
