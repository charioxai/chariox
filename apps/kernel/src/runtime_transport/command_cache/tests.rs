use super::*;
use crate::local::ListSessionsRequest;

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
        CommandReservation::Dispatch
    ));
    cache.forget_pending("command-first").await;
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
    assert!(matches!(
        cache.reserve("command-first", &first_fingerprint).await,
        CommandReservation::Dispatch
    ));
    assert!(matches!(
        cache.reserve("command-second", &second_fingerprint).await,
        CommandReservation::Dispatch
    ));
    assert!(matches!(
        cache.reserve("command-third", &third_fingerprint).await,
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
        max_total_bytes: Some(700_000),
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
        "session.state.get",
        "slice.list",
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
        "arroba-command-cache-{label}-{}-{unique}.jsonl",
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
