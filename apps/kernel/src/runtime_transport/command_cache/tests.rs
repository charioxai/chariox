use super::*;
use crate::local::{
    AppWorkerAction, ControlAppWorkerRequest, ListSessionsRequest,
    RequestCredentialEnrollmentInteractionRequest, RequestNativeProviderTurnInteractionRequest,
    RespondToInteractionRequest, UninstallAppRequest,
};

/// A replay of these runs them again (no cache entry, no request-id ledger),
/// so the kernel client never resends one once written. Its list and this one
/// must name the same requests.
#[test]
fn requests_a_replay_runs_again_are_the_ones_the_kernel_client_never_resends() {
    let control = LocalDaemonRequest::ControlAppWorker(ControlAppWorkerRequest {
        installation_id: "todo".to_string(),
        action: AppWorkerAction::Restart,
    });
    let uninstall = LocalDaemonRequest::UninstallApp(UninstallAppRequest {
        installation_id: "todo".to_string(),
        expected_generation: "3".to_string(),
        delete_data: false,
    });
    assert!(!request_is_cacheable(&control));
    assert!(!request_is_cacheable(&uninstall));
    for request in [
        serde_json::json!({"OpenUserAppView":{"installation_id":"todo","host":"client_native"}}),
        serde_json::json!({"CallUserAppView":{"view_id":"user-app-fixture","method":"increment","input":{}}}),
    ] {
        let request: LocalDaemonRequest = serde_json::from_value(request).unwrap();
        assert!(!request_is_cacheable(&request));
    }
    let client = include_str!("../../../../../packages/kernel-client/src/ipc.ts");
    let listed = client
        .split_once("KERNEL_REQUESTS_RUN_AGAIN_ON_REPLAY = new Set([")
        .and_then(|(_, rest)| rest.split_once("])"))
        .map(|(names, _)| {
            names
                .split(',')
                .map(|name| name.trim().trim_matches('"'))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .expect("the kernel client lists the requests it never resends");
    assert_eq!(
        listed,
        [
            "ControlAppWorker",
            "UninstallApp",
            "OpenUserAppView",
            "CallUserAppView"
        ]
    );
}

#[test]
fn command_cache_stores_shared_serialized_byte_arrays() {
    let byte_count = 64 * 1024;
    let value = serde_json::to_value(vec![7_u8; byte_count]).expect("bytes should serialize");
    let result = persistent_result_for_test(
        "bytes",
        CommandResultCache::fingerprint_from_bytes_for_test(b"bytes"),
        1,
        Some(value.clone()),
    )
    .result;
    let serialized = serde_json::to_string(&value).unwrap();
    assert_eq!(result.response.as_ref().unwrap().get(), serialized);
    assert_eq!(*result.response_value(), Some(value));
    assert!(cached_command_result_memory_bytes("bytes", &result) < serialized.len() as u64 + 1024);
    let cloned = result.clone();
    assert!(Arc::ptr_eq(
        result.response.as_ref().unwrap(),
        cloned.response.as_ref().unwrap()
    ));
    let persisted = serde_json::to_string(&result).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&persisted).unwrap()["response"],
        serde_json::from_str::<Value>(&serialized).unwrap()
    );
    let restored: CachedCommandResult = serde_json::from_str(&persisted).unwrap();
    assert_eq!(*restored.response_value(), *result.response_value());
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
    let native_request = LocalDaemonRequest::RequestNativeProviderTurnInteraction(
        RequestNativeProviderTurnInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "interaction-1",
            Some("Approve?".to_string()),
            "Approve?".to_string(),
            Some(30),
            crate::session::NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "provider-run-native-test".into(),
            },
        ),
    );
    let response_request = LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
        session_id: "session-1".to_string(),
        interaction_id: "interaction-1".to_string(),
        choice_id: "submit_callback".to_string(),
        custom_reply: Some("secret-callback".to_string()),
        passkey: None,
        passkey_remember_minutes: None,
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
    let request = LocalDaemonRequest::RequestNativeProviderTurnInteraction(
        RequestNativeProviderTurnInteractionRequest::allow_deny(
            "session-1",
            "agent-1",
            "interaction-1",
            Some("Approve?".to_string()),
            "Approve?".to_string(),
            Some(30),
            crate::session::NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "provider-run-native-test".into(),
            },
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
    assert_eq!(*replayed.response_value(), Some(response));
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
    assert_eq!(
        *result.response_value(),
        Some(serde_json::json!({"ok": true}))
    );

    let _ = fs::remove_file(path);
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
        at_most_once: false,
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
        at_most_once: false,
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
        at_most_once: false,
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
            response: serialized_response(&response),
            error: None,
            completed_at_ms,
            fingerprint,
        },
    }
}

#[tokio::test]
async fn at_most_once_receipts_never_evict_and_refuse_new_identity_at_capacity() {
    let path = temp_cache_path("at-most-once-capacity");
    let cache = CommandResultCache::new_with_persistent_path_and_retention(
        path.clone(),
        CommandResultRetentionPolicy {
            max_entries: 1,
            at_most_once: true,
            max_age_ms: Some(0),
            ..CommandResultRetentionPolicy::persistent()
        },
    )
    .unwrap();
    let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"input");
    let response = serde_json::json!({"accepted":true});
    assert!(matches!(
        cache
            .reserve_at_most_once("one", &fingerprint, response.clone())
            .await
            .unwrap(),
        CommandReservation::Dispatch
    ));
    cache
        .complete(
            "one".into(),
            fingerprint.clone(),
            &KernelOutgoingFrame::Response {
                request_id: "one".into(),
                response: Box::new(Some(response.clone())),
                error: None,
            },
        )
        .await;
    let capacity = match cache
        .reserve_at_most_once("two", &fingerprint, response.clone())
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("full journal admitted a new receipt"),
    };
    assert!(is_receipt_capacity_error(&capacity));
    // The OS uses the same ErrorKind for ENOMEM. It is an I/O failure,
    // not receipt capacity, and cannot authorize the safety-control exception.
    let enomem = io::Error::from_raw_os_error(libc::ENOMEM);
    assert_eq!(enomem.kind(), capacity.kind());
    assert!(!is_receipt_capacity_error(&enomem));
    assert!(matches!(
        cache
            .reserve_at_most_once("one", &fingerprint, response.clone())
            .await
            .unwrap(),
        CommandReservation::Wait(_)
    ));
    let restored =
        CommandResultCache::new_with_persistent_path_and_retention(path.clone(), cache.retention)
            .unwrap();
    assert!(matches!(
        restored
            .reserve_at_most_once("one", &fingerprint, response)
            .await
            .unwrap(),
        CommandReservation::Wait(_)
    ));
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn at_most_once_duplicates_wait_for_durable_settlement() {
    let path = temp_cache_path("at-most-once-blocked-settlement");
    let cache = std::sync::Arc::new(CommandResultCache::new_at_most_once(path.clone()).unwrap());
    let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"input");
    let unknown = serde_json::json!({"unknown":true});
    let success = serde_json::json!({"ok":true});
    assert!(matches!(
        cache
            .reserve_at_most_once("one", &fingerprint, unknown.clone())
            .await
            .unwrap(),
        CommandReservation::Dispatch
    ));
    let mut duplicate = match cache
        .reserve_at_most_once("one", &fingerprint, unknown.clone())
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("duplicate must wait"),
    };
    let guard = cache.persistence.as_ref().unwrap().io_lock.lock().await;
    let task_cache = cache.clone();
    let task_fingerprint = fingerprint.clone();
    let task_success = success.clone();
    let mut completion = tokio::spawn(async move {
        task_cache
            .complete_at_most_once("one".into(), task_fingerprint, task_success, unknown)
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut completion)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut duplicate)
            .await
            .is_err()
    );
    drop(guard);
    completion.await.unwrap().unwrap();
    assert_eq!(
        *duplicate.await.unwrap().response_value(),
        Some(success.clone())
    );
    let restored = CommandResultCache::new_at_most_once(path.clone()).unwrap();
    let replay = match restored
        .reserve_at_most_once("one", &fingerprint, serde_json::Value::Null)
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("settled receipt must replay"),
    };
    assert_eq!(*replay.await.unwrap().response_value(), Some(success));
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn at_most_once_sync_failure_reports_unknown_to_every_caller() {
    let path = temp_cache_path("at-most-once-failed-sync");
    let cache = CommandResultCache::new_at_most_once(path.clone()).unwrap();
    let fingerprint = CommandResultCache::fingerprint_from_bytes_for_test(b"input");
    let unknown = serde_json::json!({"unknown":true});
    assert!(matches!(
        cache
            .reserve_at_most_once("one", &fingerprint, unknown.clone())
            .await
            .unwrap(),
        CommandReservation::Dispatch
    ));
    let duplicate = match cache
        .reserve_at_most_once("one", &fingerprint, unknown.clone())
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("duplicate must wait"),
    };
    cache.fail_settlement_sync.store(true, Ordering::SeqCst);
    assert!(cache
        .complete_at_most_once(
            "one".into(),
            fingerprint.clone(),
            serde_json::json!({"ok":true}),
            unknown.clone()
        )
        .await
        .is_err());
    assert_eq!(
        *duplicate.await.unwrap().response_value(),
        Some(unknown.clone())
    );
    let replay = match cache
        .reserve_at_most_once("one", &fingerprint, unknown.clone())
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => wait,
        _ => panic!("failed settlement must never redispatch"),
    };
    assert_eq!(*replay.await.unwrap().response_value(), Some(unknown));
    let _ = fs::remove_file(path);
}

#[test]
fn saved_snapshot_restore_is_not_transport_cacheable() {
    let request =
        LocalDaemonRequest::RestoreAppDataSnapshot(crate::local::RestoreAppDataSnapshotRequest {
            installation_id: "installed".into(),
            expected_generation: "1".into(),
            snapshot_id: "snapshot-saved".into(),
        });
    assert!(!request_is_cacheable(&request));
}

#[test]
fn command_cache_replays_generated_responses_beyond_input_nesting_limit() {
    let mut response = Value::Null;
    for _ in 0..150 {
        response = Value::Array(vec![response]);
    }
    let result = persistent_result_for_test(
        "nested",
        CommandResultCache::fingerprint_from_bytes_for_test(b"nested"),
        1,
        Some(response.clone()),
    )
    .result;
    assert_eq!(*result.response_value(), Some(response));
}

#[test]
fn command_cache_replays_floats_without_precision_loss() {
    for value in [
        -0.0_f64,
        51.248178375505404,
        2.0030397744267762e-253,
        3.9287532173373315e299,
    ] {
        let result = persistent_result_for_test(
            "float",
            CommandResultCache::fingerprint_from_bytes_for_test(b"float"),
            1,
            Some(serde_json::json!({ "value": value })),
        )
        .result;
        let replayed = (*result.response_value()).unwrap();
        assert_eq!(
            replayed["value"].as_f64().unwrap().to_bits(),
            value.to_bits()
        );
    }
}

#[test]
fn md_capture_observations_reenter_live_protection_instead_of_replaying_pixels() {
    let request: crate::local::LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "CaptureVisibleRegion": {
            "capture_id": "capture-1",
            "surface": {"kind":"kernel_browser","tab_id":"host-tab-1","generation":1},
            "region": {"x":0,"y":0,"width":1,"height":1,"viewport_width":1280,"viewport_height":800,"frame_width":1280,"frame_height":800}
        }
    })).unwrap();
    assert!(!super::request_is_cacheable(&request));
}
