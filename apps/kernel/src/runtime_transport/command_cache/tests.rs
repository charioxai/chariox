use super::*;
use crate::local::{
    ListSessionsRequest, RequestCredentialEnrollmentInteractionRequest,
    RequestNativeProviderInteractionRequest, RespondToInteractionRequest, SliceStateSaveMode,
    SliceStateSaveRequest, SliceStateSaveScope,
};

#[test]
fn command_cache_estimates_json_byte_arrays_by_heap_footprint() {
    let byte_count = 64 * 1024;
    let value = serde_json::to_value(vec![7_u8; byte_count]).expect("bytes should serialize");
    let estimated = value_heap_bytes(&value);

    assert!(
        estimated >= (byte_count * std::mem::size_of::<Value>()) as u64,
        "JSON byte arrays must be charged for each heap-resident Value: {estimated}"
    );
}

#[test]
fn interaction_requests_use_volatile_command_deduplication() {
    let helper_request = LocalDaemonRequest::RequestCredentialEnrollmentInteraction(
        RequestCredentialEnrollmentInteractionRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            enrollment_id: "enrollment-1".to_string(),
            profile_id: "profile-1".to_string(),
            target_version: 1,
            provider_authorization_url: "https://claude.com/oauth/authorize?state=opaque"
                .to_string(),
            timeout_sec: Some(30),
        },
    );
    let native_request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "interaction-1",
            Some("Approve?".to_string()),
            "Approve?".to_string(),
            Some(30),
        ),
    );
    let response_request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: "session-1".to_string(),
        interaction_id: "interaction-1".to_string(),
        choice_id: "submit_callback".to_string(),
        custom_reply: Some("secret-callback".to_string()),
    });

    for request in [&helper_request, &native_request, &response_request] {
        assert!(request_is_cacheable(request));
        assert!(!should_persist_completed_result(
            &CommandResultCache::fingerprint_for_test(request),
        ));
    }
    assert!(request_is_cacheable(&LocalDaemonRequest::ListSessions(
        ListSessionsRequest,
    )));
}

#[tokio::test]
async fn pending_interaction_replay_waits_for_one_volatile_result() {
    let path = temp_cache_path("pending-interaction-replay");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "interaction-1",
            Some("Approve?".to_string()),
            "Approve?".to_string(),
            Some(30),
        ),
    );
    let fingerprint = CommandResultCache::fingerprint_for_test(&request);

    assert!(matches!(
        cache.reserve("interaction-command", &fingerprint).await,
        CommandReservation::Dispatch
    ));
    let replay = match cache.reserve("interaction-command", &fingerprint).await {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("transport replay should wait for the original interaction"),
    };
    let response = serde_json::json!({
        "NativeProviderInteractionResolved": {
            "resolution": {
                "status": "resolved",
                "choice_id": "allow_once",
                "reply": "allow"
            }
        }
    });
    let frame = KernelOutgoingFrame::Response {
        request_id: "interaction-attempt-1".to_string(),
        response: Box::new(Some(response.clone())),
        error: None,
    };

    cache
        .complete(
            "interaction-command".to_string(),
            fingerprint.clone(),
            &frame,
        )
        .await;
    let replayed = replay.await.expect("interaction replay should resolve");
    assert_eq!(*replayed.response, Some(response));
    assert!(fs::read_to_string(&path).unwrap_or_default().is_empty());

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    assert!(matches!(
        restored.reserve("interaction-command", &fingerprint).await,
        CommandReservation::Dispatch
    ));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_recovers_completed_results() {
    let path = temp_cache_path("recover-completed");
    let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let fingerprint = CommandResultCache::fingerprint_for_test(&request);
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    cache
        .insert_completed_for_test(
            "command-1".to_string(),
            fingerprint.clone(),
            Some(serde_json::json!({"ok": true})),
        )
        .await;

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    let wait = match restored.reserve("command-1", &fingerprint).await {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("completed command should be replayable after reload"),
    };
    let result = wait.await.expect("cached result should resolve");
    assert_eq!(*result.response, Some(serde_json::json!({"ok": true})));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn slice_state_save_acknowledgement_replays_without_a_second_dispatch() {
    let path = temp_cache_path("slice-save-ack-replay");
    let command_id = "slice-save-command";
    let request = LocalDaemonRequest::SaveSliceState(SliceStateSaveRequest {
        slice_ref: "slice-1".to_string(),
        mode: Some(SliceStateSaveMode::Shutdown),
        scope: Some(SliceStateSaveScope::ThisSlice),
    });
    let fingerprint = CommandResultCache::fingerprint_for_test(&request);
    let response = serde_json::json!({
        "SliceStateSaved": {
            "slice": { "id": "slice-1", "saved_state_ref": "state-generation-a" },
            "state": { "id": "state-generation-a", "status": "ready" }
        }
    });
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");

    assert!(matches!(
        cache.reserve(command_id, &fingerprint).await,
        CommandReservation::Dispatch
    ));
    cache
        .complete(
            command_id.to_string(),
            fingerprint.clone(),
            &KernelOutgoingFrame::Response {
                request_id: "first-transport-attempt".to_string(),
                response: Box::new(Some(response.clone())),
                error: None,
            },
        )
        .await;

    let replayed = match cache.reserve(command_id, &fingerprint).await {
        CommandReservation::Wait(wait) => wait.await.expect("completed save should replay"),
        _ => panic!("lost acknowledgement must not dispatch a second save"),
    };
    assert_eq!(*replayed.response, Some(response.clone()));
    drop(cache);

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("completed save result should survive kernel restart");
    let replayed_after_restart = match restored.reserve(command_id, &fingerprint).await {
        CommandReservation::Wait(wait) => wait.await.expect("restored save should replay"),
        _ => panic!("restart after acknowledgement loss must not create another generation"),
    };
    assert_eq!(*replayed_after_restart.response, Some(response));

    let conflicting_request = LocalDaemonRequest::SaveSliceState(SliceStateSaveRequest {
        slice_ref: "slice-1".to_string(),
        mode: Some(SliceStateSaveMode::RestartAgents),
        scope: Some(SliceStateSaveScope::FutureSlices),
    });
    assert!(matches!(
        restored
            .reserve(
                command_id,
                &CommandResultCache::fingerprint_for_test(&conflicting_request),
            )
            .await,
        CommandReservation::Conflict
    ));

    let _ = fs::remove_file(path);
    println!(
        "CHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_save_ack_loss_probe.v1",
            "sameProcessReplay": true,
            "restartReplay": true,
            "savedStateRefPreserved": true,
            "conflictingReuseRejected": true,
            "cleanupComplete": true
        })
    );
}

#[tokio::test]
async fn persistent_command_cache_rejects_conflicting_reuse_after_reload() {
    let path = temp_cache_path("reject-conflict");
    let first = CommandResultCache::fingerprint_from_bytes_for_test(b"first");
    let second = CommandResultCache::fingerprint_from_bytes_for_test(b"second");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    cache
        .insert_completed_for_test(
            "command-1".to_string(),
            first,
            Some(serde_json::json!({"ok": true})),
        )
        .await;

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    assert!(matches!(
        restored.reserve("command-1", &second).await,
        CommandReservation::Conflict
    ));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_compacts_to_retention_limit() {
    let path = temp_cache_path("compact-retention");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    for index in 0..(COMMAND_RESULT_CACHE_LIMIT + 8) {
        let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(
            format!("request-{index}").as_bytes(),
        );
        cache
            .insert_completed_for_test(
                format!("command-{index}"),
                fingerprint,
                Some(serde_json::json!({ "index": index })),
            )
            .await;
    }
    assert_eq!(cache.completed_count().await, COMMAND_RESULT_CACHE_LIMIT);

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    assert_eq!(restored.completed_count().await, COMMAND_RESULT_CACHE_LIMIT);

    let first_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"request-0");
    assert!(matches!(
        restored.reserve("command-0", &first_fingerprint).await,
        CommandReservation::Dispatch
    ));
    let retained_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"request-8");
    assert!(matches!(
        restored.reserve("command-8", &retained_fingerprint).await,
        CommandReservation::Wait(_)
    ));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_defers_disk_compaction_after_memory_eviction() {
    let path = temp_cache_path("defer-eviction-compaction");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    let completed = COMMAND_RESULT_CACHE_LIMIT + 8;
    for index in 0..completed {
        cache
            .insert_completed_for_test(
                format!("command-{index}"),
                CommandResultCache::fingerprint_from_bytes_for_test(
                    format!("request-{index}").as_bytes(),
                ),
                Some(serde_json::json!({ "index": index })),
            )
            .await;
    }

    let persisted = fs::read_to_string(&path).expect("persistent cache should exist");
    assert_eq!(
        persisted.lines().count(),
        completed,
        "memory eviction must not rewrite the full disk snapshot on every completion"
    );

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_compacts_by_age_on_load() {
    let path = temp_cache_path("compact-age");
    let now_ms = crate::session::unix_epoch_ms();
    let old = persistent_result_for_test(
        "command-old",
        CommandResultCache::fingerprint_from_bytes_for_test(b"old"),
        now_ms.saturating_sub(10_000),
        Some(serde_json::json!({ "value": "old" })),
    );
    let fresh = persistent_result_for_test(
        "command-fresh",
        CommandResultCache::fingerprint_from_bytes_for_test(b"fresh"),
        now_ms,
        Some(serde_json::json!({ "value": "fresh" })),
    );
    rewrite_persistent_results(&path, &[old, fresh]).expect("cache fixture should write");
    let retention = CommandResultRetentionPolicy {
        max_entries: COMMAND_RESULT_CACHE_LIMIT,
        max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
        max_total_bytes: None,
        max_age_ms: Some(1_000),
    };

    let cache = CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
        .expect("persistent cache should reload");

    assert_eq!(cache.completed_count().await, 1);
    let old_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"old");
    assert!(matches!(
        cache.reserve("command-old", &old_fingerprint).await,
        CommandReservation::Dispatch
    ));
    let fresh_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"fresh");
    assert!(matches!(
        cache.reserve("command-fresh", &fresh_fingerprint).await,
        CommandReservation::Wait(_)
    ));
    let stored = fs::read_to_string(&path).expect("compacted cache should exist");
    assert!(!stored.contains("command-old"));
    assert!(stored.contains("command-fresh"));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_compacts_by_total_bytes() {
    let path = temp_cache_path("compact-bytes");
    let first_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"first");
    let second_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"second");
    let third_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"third");
    let first_response = Some(serde_json::json!({ "payload": "x".repeat(120) }));
    let second_response = Some(serde_json::json!({ "payload": "y".repeat(120) }));
    let third_response = Some(serde_json::json!({ "payload": "z".repeat(120) }));
    let second_entry = persistent_result_for_test(
        "command-second",
        second_fingerprint.clone(),
        crate::session::unix_epoch_ms(),
        second_response.clone(),
    );
    let retention = CommandResultRetentionPolicy {
        max_entries: COMMAND_RESULT_CACHE_LIMIT,
        max_memory_bytes: COMMAND_RESULT_CACHE_MAX_MEMORY_BYTES,
        max_total_bytes: Some(
            persistent_result_jsonl_bytes(&second_entry).expect("entry should serialize"),
        ),
        max_age_ms: None,
    };
    let cache = CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
        .expect("persistent cache should initialize");
    cache
        .insert_completed_for_test(
            "command-first".to_string(),
            first_fingerprint.clone(),
            first_response,
        )
        .await;
    cache
        .insert_completed_for_test(
            "command-second".to_string(),
            second_fingerprint.clone(),
            second_response,
        )
        .await;

    let stored = fs::read_to_string(&path).expect("cache should exist");
    assert!(
        stored.contains("command-first"),
        "disk compaction may be deferred until file growth is material"
    );
    assert!(matches!(
        cache.reserve("command-first", &first_fingerprint).await,
        CommandReservation::Wait(_)
    ));
    assert!(matches!(
        cache.reserve("command-second", &second_fingerprint).await,
        CommandReservation::Wait(_)
    ));

    cache
        .insert_completed_for_test(
            "command-third".to_string(),
            third_fingerprint.clone(),
            third_response,
        )
        .await;

    let stored = fs::read_to_string(&path).expect("cache should exist");
    assert!(
        stored.len() as u64
            <= retention.max_total_bytes.unwrap()
                * COMMAND_RESULT_COMPACTION_FILE_GROWTH_MULTIPLIER,
        "cache should compact once file growth crosses the byte budget multiplier: {stored}"
    );
    assert!(!stored.contains("command-first"));
    assert!(!stored.contains("command-second"));
    assert!(stored.contains("command-third"));

    // Disk retention must not evict valid replay entries from the live memory cache.
    assert!(matches!(
        cache.reserve("command-first", &first_fingerprint).await,
        CommandReservation::Wait(_)
    ));
    assert!(matches!(
        cache.reserve("command-second", &second_fingerprint).await,
        CommandReservation::Wait(_)
    ));
    assert!(matches!(
        cache.reserve("command-third", &third_fingerprint).await,
        CommandReservation::Wait(_)
    ));

    // A restart restores only the disk-retained tail.
    let restored =
        CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
            .expect("persistent cache should reload");
    assert!(matches!(
        restored.reserve("command-first", &first_fingerprint).await,
        CommandReservation::Dispatch
    ));
    assert!(matches!(
        restored
            .reserve("command-second", &second_fingerprint)
            .await,
        CommandReservation::Dispatch
    ));
    assert!(matches!(
        restored.reserve("command-third", &third_fingerprint).await,
        CommandReservation::Wait(_)
    ));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_drops_oversized_records_on_load() {
    let path = temp_cache_path("drop-oversized-records");
    let oversized_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"huge");
    let small_fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"small");
    let oversized = persistent_result_for_test(
        "command-huge",
        oversized_fingerprint.clone(),
        crate::session::unix_epoch_ms(),
        Some(serde_json::json!({
            "payload": "x".repeat(
                COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES as usize
            )
        })),
    );
    let small = persistent_result_for_test(
        "command-small",
        small_fingerprint.clone(),
        crate::session::unix_epoch_ms(),
        Some(serde_json::json!({"ok": true})),
    );
    rewrite_persistent_results(&path, &[oversized, small]).expect("cache fixture should write");

    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");

    assert!(matches!(
        cache.reserve("command-huge", &oversized_fingerprint).await,
        CommandReservation::Dispatch
    ));
    cache.forget_pending("command-huge").await;
    assert!(matches!(
        cache.reserve("command-small", &small_fingerprint).await,
        CommandReservation::Wait(_)
    ));
    let stored = fs::read_to_string(&path).expect("compacted cache should exist");
    assert!(!stored.contains("command-huge"));
    assert!(stored.contains("command-small"));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_does_not_persist_oversized_results() {
    let path = temp_cache_path("skip-oversized-persist");
    let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"huge-result");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");
    cache
        .insert_completed_for_test(
            "command-huge".to_string(),
            fingerprint.clone(),
            Some(serde_json::json!({
                "payload": "x".repeat(
                    COMMAND_RESULT_CACHE_MAX_PERSISTED_RECORD_BYTES as usize
                )
            })),
        )
        .await;

    assert!(matches!(
        cache.reserve("command-huge", &fingerprint).await,
        CommandReservation::Wait(_)
    ));
    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    assert!(matches!(
        restored.reserve("command-huge", &fingerprint).await,
        CommandReservation::Dispatch
    ));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn command_cache_byte_bounds_oversized_non_persisted_results_in_memory() {
    let path = temp_cache_path("byte-bound-oversized-memory-results");
    let retention = CommandResultRetentionPolicy {
        max_entries: 512,
        max_memory_bytes: 700_000,
        max_total_bytes: None,
        max_age_ms: None,
    };
    let cache = CommandResultCache::new_with_persistent_path_and_retention(path.clone(), retention)
        .expect("cache should initialize");
    let first = CommandResultCache::fingerprint_from_bytes_for_test(b"large-first");
    let second = CommandResultCache::fingerprint_from_bytes_for_test(b"large-second");

    cache
        .insert_completed_for_test(
            "command-large-first".to_string(),
            first.clone(),
            Some(serde_json::json!({ "payload": "x".repeat(400_000) })),
        )
        .await;
    cache
        .insert_completed_for_test(
            "command-large-second".to_string(),
            second.clone(),
            Some(serde_json::json!({ "payload": "y".repeat(400_000) })),
        )
        .await;

    assert!(matches!(
        cache.reserve("command-large-first", &first).await,
        CommandReservation::Dispatch
    ));
    cache.forget_pending("command-large-first").await;
    assert!(matches!(
        cache.reserve("command-large-second", &second).await,
        CommandReservation::Wait(_)
    ));
    assert!(fs::read_to_string(&path).unwrap_or_default().is_empty());

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_skips_noisy_read_commands_on_disk() {
    let noisy_command_types = [
        "external_provider_session.list",
        "provider.catalog.get",
        "prompt_input_history.get",
        "session.state.get",
        "session.history.blob",
        "session.history.outline",
        "slice.list",
        "terminal.command_catalog.get",
        "waiting_room.inventory.get",
        "waiting_room.public_snapshot.get",
    ];
    let path = temp_cache_path("skip-read-commands");
    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should initialize");

    for command_type in noisy_command_types {
        let command_id = format!("command-{}", command_type.replace('.', "-"));
        let fingerprint = CommandResultCache::fingerprint_for_command_type_test(command_type);
        cache
            .insert_completed_for_test(
                command_id.clone(),
                fingerprint.clone(),
                Some(serde_json::json!({ "command_type": command_type })),
            )
            .await;

        assert!(matches!(
            cache.reserve(&command_id, &fingerprint).await,
            CommandReservation::Wait(_)
        ));
    }
    assert!(
        fs::read_to_string(&path).unwrap_or_default().is_empty(),
        "high-frequency read command results should not be serialized to disk"
    );

    let restored = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");
    for command_type in noisy_command_types {
        let command_id = format!("command-{}", command_type.replace('.', "-"));
        let fingerprint = CommandResultCache::fingerprint_for_command_type_test(command_type);
        assert!(matches!(
            restored.reserve(&command_id, &fingerprint).await,
            CommandReservation::Dispatch
        ));
    }

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_removes_read_only_history_records_on_load() {
    let path = temp_cache_path("drop-persisted-history-reads");
    let history_fingerprint =
        CommandResultCache::fingerprint_for_command_type_test("session.history.outline");
    let mutation_fingerprint =
        CommandResultCache::fingerprint_for_command_type_test("prompt.submit");
    let history = persistent_result_for_test(
        "command-history",
        history_fingerprint.clone(),
        crate::session::unix_epoch_ms(),
        Some(serde_json::json!({ "history": "large paged response" })),
    );
    let mutation = persistent_result_for_test(
        "command-mutation",
        mutation_fingerprint.clone(),
        crate::session::unix_epoch_ms(),
        Some(serde_json::json!({ "submitted": true })),
    );
    rewrite_persistent_results(&path, &[history, mutation]).expect("cache fixture should write");

    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("persistent cache should reload");

    assert!(matches!(
        cache.reserve("command-history", &history_fingerprint).await,
        CommandReservation::Dispatch
    ));
    cache.forget_pending("command-history").await;
    assert!(matches!(
        cache
            .reserve("command-mutation", &mutation_fingerprint)
            .await,
        CommandReservation::Wait(_)
    ));
    let stored = fs::read_to_string(&path).expect("compacted cache should exist");
    assert!(!stored.contains("command-history"));
    assert!(stored.contains("command-mutation"));

    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn persistent_command_cache_ignores_malformed_lines() {
    let path = temp_cache_path("ignore-malformed");
    fs::write(
            &path,
            [
                "{not json}",
                r#"{"command_id":"command-1","result":{"response":{"ok":true},"error":null,"fingerprint":{"command_type":"test","source":"test","session_id":null,"attachment_id":null,"request_hash":42}}}"#,
            ]
            .join("\n"),
        )
        .expect("cache fixture should write");

    let cache = CommandResultCache::new_with_persistent_path(path.clone())
        .expect("malformed lines should not prevent cache load");

    assert_eq!(cache.completed_count().await, 1);

    let _ = fs::remove_file(path);
}

#[test]
fn command_fingerprint_hash_is_stable() {
    let first = CommandResultCache::fingerprint_from_bytes_for_test(b"same request");
    let second = CommandResultCache::fingerprint_from_bytes_for_test(b"same request");
    let different = CommandResultCache::fingerprint_from_bytes_for_test(b"different request");

    assert_eq!(
        CommandResultCache::request_hash_for_test(&first),
        CommandResultCache::request_hash_for_test(&second)
    );
    assert_ne!(
        CommandResultCache::request_hash_for_test(&first),
        CommandResultCache::request_hash_for_test(&different)
    );
}

fn temp_cache_path(label: &str) -> PathBuf {
    let unique = crate::session::unix_epoch_ms();
    std::env::temp_dir().join(format!(
        "chariox-command-cache-{label}-{}-{unique}.jsonl",
        std::process::id()
    ))
}

fn persistent_result_for_test(
    command_id: &str,
    fingerprint: CommandFingerprint,
    completed_at_ms: u64,
    response: Option<Value>,
) -> PersistentCommandResult {
    PersistentCommandResult {
        command_id: command_id.to_string(),
        completed_at_ms,
        result: CachedCommandResult {
            response: Box::new(response),
            error: None,
            completed_at_ms,
            fingerprint,
        },
    }
}
