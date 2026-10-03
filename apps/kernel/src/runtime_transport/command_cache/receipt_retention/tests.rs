use super::*;

struct Journal(PathBuf);
impl Journal {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "chariox-retention-{:016x}.jsonl",
            rand::random::<u64>()
        )))
    }
    fn cache(&self, limit: usize) -> CommandResultCache {
        CommandResultCache::new_with_persistent_path_and_retention(
            self.0.clone(),
            CommandResultRetentionPolicy {
                max_entries: limit,
                at_most_once: true,
                max_age_ms: None,
                ..CommandResultRetentionPolicy::persistent()
            },
        )
        .unwrap()
    }
    fn age(&self) {
        let age = crate::session::unix_epoch_ms() - APP_RECEIPT_RETENTION_MS - 60_000;
        let records = fs::read_to_string(&self.0)
            .unwrap()
            .lines()
            .map(|line| {
                let mut value: Value = serde_json::from_str(line).unwrap();
                value["completed_at_ms"] = age.into();
                value["result"]["completed_at_ms"] = age.into();
                serde_json::to_string(&value).unwrap() + "\n"
            })
            .collect::<String>();
        fs::write(&self.0, records).unwrap();
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        for path in [
            &self.0,
            &self.0.with_extension("expired"),
            &self.0.with_extension("jsonl.tmp"),
        ] {
            let _ = fs::remove_file(path);
        }
    }
}
fn fingerprint() -> CommandFingerprint {
    CommandResultCache::fingerprint_from_bytes_for_test(b"receipt input")
}
async fn accept(cache: &CommandResultCache, id: &str) {
    assert!(matches!(
        cache
            .reserve_at_most_once(id, &fingerprint(), serde_json::json!({"unknown":true}))
            .await
            .unwrap(),
        CommandReservation::Dispatch
    ));
}
async fn settle(cache: &CommandResultCache, id: &str) {
    cache
        .complete_at_most_once(
            id.into(),
            fingerprint(),
            serde_json::json!({"ok": id}),
            Value::Null,
        )
        .await
        .unwrap();
}
async fn replay(cache: &CommandResultCache, id: &str) {
    match cache
        .reserve_at_most_once(id, &fingerprint(), Value::Null)
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => assert_eq!(
            *wait.await.unwrap().response_value(),
            Some(serde_json::json!({"ok":id}))
        ),
        _ => panic!("receipt redispatched"),
    }
}

#[tokio::test]
async fn lru_touch_survives_restart_and_evicted_replay_never_dispatches() {
    let journal = Journal::new();
    let cache = journal.cache(3);
    for id in ["one", "two", "three"] {
        accept(&cache, id).await;
        settle(&cache, id).await;
    }
    drop(cache);
    journal.age();
    let cache = journal.cache(3);
    replay(&cache, "one").await; // Move the oldest identity to most recently used.
    drop(cache);
    let cache = journal.cache(3);
    accept(&cache, "four").await;
    settle(&cache, "four").await;
    assert_eq!(cache.results.lock().await.len(), 3);
    for id in ["one", "three", "four"] {
        replay(&cache, id).await;
    }
    drop(cache);
    let restored = journal.cache(3);
    let error = match restored
        .reserve_at_most_once("two", &fingerprint(), Value::Null)
        .await
    {
        Err(error) => error,
        _ => panic!("evicted receipt dispatched"),
    };
    assert!(at_most_once::is_receipt_expired_error(&error));
    assert!(!at_most_once::is_receipt_capacity_error(&error));
    assert_eq!(error.to_string(), "receipt expired");
    assert!(restored.has_reserved("two").await);
    assert_eq!(restored.results.lock().await.len(), 3);
}

#[tokio::test]
async fn retention_guard_protects_recent_completed_and_live_pending_receipts() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "recent").await;
    settle(&cache, "recent").await;
    let result = cache
        .reserve_at_most_once("new", &fingerprint(), Value::Null)
        .await;
    assert!(result.is_err_and(|error| at_most_once::is_receipt_capacity_error(&error)));
    replay(&cache, "recent").await;
    drop(cache);
    let restored = journal.cache(1);
    let result = restored
        .reserve_at_most_once("new", &fingerprint(), Value::Null)
        .await;
    assert!(result.is_err_and(|error| at_most_once::is_receipt_capacity_error(&error)));

    let pending_journal = Journal::new();
    let pending = pending_journal.cache(1);
    accept(&pending, "pending").await;
    pending_journal.age(); // Even a very long-running effect cannot be evicted.
    let result = pending
        .reserve_at_most_once("new", &fingerprint(), Value::Null)
        .await;
    assert!(result.is_err_and(|error| at_most_once::is_receipt_capacity_error(&error)));
}

#[tokio::test]
async fn latest_settlement_protects_old_acceptance_and_unknown_age_is_not_expired() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "one").await;
    journal.age();
    settle(&cache, "one").await; // Acceptance old, final receipt still inside retention.
    drop(cache);
    let cache = journal.cache(1);
    assert!(cache
        .reserve_at_most_once("two", &fingerprint(), Value::Null)
        .await
        .is_err());
    drop(cache);
    let zero_age = fs::read_to_string(&journal.0)
        .unwrap()
        .lines()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            value["completed_at_ms"] = 0.into();
            value["result"]["completed_at_ms"] = 0.into();
            serde_json::to_string(&value).unwrap() + "\n"
        })
        .collect::<String>();
    fs::write(&journal.0, zero_age).unwrap();
    let cache = journal.cache(1);
    assert!(cache
        .reserve_at_most_once("two", &fingerprint(), Value::Null)
        .await
        .is_err());
}

#[tokio::test]
async fn crash_between_marker_and_compaction_filters_old_receipt_and_marker_alone_refuses() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "one").await;
    settle(&cache, "one").await;
    cache
        .receipt_retention
        .lock()
        .await
        .expire(&journal.0, "one")
        .unwrap();
    drop(cache); // Crash before rewriting the response journal.
    let cache = journal.cache(1);
    assert!(cache.results.lock().await.is_empty());
    assert!(cache
        .reserve_at_most_once("one", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
    accept(&cache, "two").await;
    settle(&cache, "two").await;
    drop(cache);
    fs::remove_file(&journal.0).unwrap();
    let cache = journal.cache(1);
    assert!(cache
        .reserve_at_most_once("one", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
}

#[tokio::test]
async fn compaction_preserves_pending_acceptance_as_unknown_outcome_after_restart() {
    let journal = Journal::new();
    let cache = journal.cache(2);
    accept(&cache, "pending").await;
    accept(&cache, "completed").await;
    cache
        .persistence
        .as_ref()
        .unwrap()
        .skipped_compactions
        .store(COMMAND_RESULT_COMPACTION_SKIP_LIMIT, Ordering::Release);
    settle(&cache, "completed").await;
    drop(cache);
    let cache = journal.cache(2);
    match cache
        .reserve_at_most_once("pending", &fingerprint(), Value::Null)
        .await
        .unwrap()
    {
        CommandReservation::Wait(wait) => {
            assert_eq!(
                *wait.await.unwrap().response_value(),
                Some(serde_json::json!({"unknown":true}))
            )
        }
        _ => panic!("compaction lost pending acceptance"),
    }
}

#[test]
fn corrupt_or_oversized_expiry_markers_fail_closed() {
    let journal = Journal::new();
    let path = ReceiptRetention::marker_path(&journal.0);
    fs::write(&path, "corrupt\n").unwrap();
    assert!(CommandResultCache::new_at_most_once(journal.0.clone()).is_err());
    fs::File::create(&path)
        .unwrap()
        .set_len(COMMAND_RESULT_CACHE_MAX_BYTES + 1)
        .unwrap();
    assert!(CommandResultCache::new_at_most_once(journal.0.clone()).is_err());
}

#[tokio::test]
async fn recovery_syncs_unsynced_markers_before_compaction_and_sync_failure_preserves_receipt() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "one").await;
    settle(&cache, "one").await;
    drop(cache);
    let before = fs::read(&journal.0).unwrap();
    // Process crash after write, before syncing the marker or new directory entry.
    fs::write(
        ReceiptRetention::marker_path(&journal.0),
        format!("{}\n", ReceiptRetention::identity("one")),
    )
    .unwrap();
    let policy = CommandResultRetentionPolicy {
        max_entries: 1,
        at_most_once: true,
        ..CommandResultRetentionPolicy::persistent()
    };
    let failed = CommandResultCache::new_with_retention_loader(journal.0.clone(), policy, |path| {
        ReceiptRetention::load_with_sync(path, |_file, _marker_path| {
            assert_eq!(fs::read(&journal.0).unwrap(), before);
            Err(io::Error::other("injected recovery sync failure"))
        })
    });
    assert!(failed.is_err());
    assert_eq!(
        fs::read(&journal.0).unwrap(),
        before,
        "failed recovery must not drop a durable receipt"
    );
    let recovered =
        CommandResultCache::new_with_retention_loader(journal.0.clone(), policy, |path| {
            ReceiptRetention::load_with_sync(path, |file, marker_path| {
                assert_eq!(
                    fs::read(&journal.0).unwrap(),
                    before,
                    "sync must precede compaction"
                );
                file.sync_all()?;
                fs::File::open(marker_path.parent().unwrap())?.sync_all()
            })
        })
        .unwrap();
    assert!(recovered.results.lock().await.is_empty());
    assert!(recovered
        .reserve_at_most_once("one", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
}

#[tokio::test]
async fn receipt_age_uses_the_newest_timestamp_even_if_settlement_clock_regressed() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "one").await;
    settle(&cache, "one").await;
    drop(cache);
    let old = crate::session::unix_epoch_ms() - APP_RECEIPT_RETENTION_MS - 60_000;
    let recent = crate::session::unix_epoch_ms();
    let records = fs::read_to_string(&journal.0)
        .unwrap()
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            let at = if index == 0 { recent } else { old };
            value["completed_at_ms"] = at.into();
            value["result"]["completed_at_ms"] = at.into();
            serde_json::to_string(&value).unwrap() + "\n"
        })
        .collect::<String>();
    fs::write(&journal.0, records).unwrap();
    let cache = journal.cache(1);
    assert!(cache
        .reserve_at_most_once("new", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_capacity_error(&e)));
    replay(&cache, "one").await;
}

#[tokio::test]
async fn expiry_compaction_fences_legacy_kernels_and_preserves_the_fence_on_rewrite() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "one").await;
    settle(&cache, "one").await;
    drop(cache);
    journal.age();
    let cache = journal.cache(1);
    accept(&cache, "two").await;
    settle(&cache, "two").await;
    assert!(fs::read_to_string(&journal.0)
        .unwrap()
        .starts_with(RECEIPT_EXPIRY_FENCE));
    // Exercise the original strict at-most-once reader, without v416's fence support.
    assert!(
        read_persistent_results_with_receipt_fence(&journal.0, cache.retention, false).is_err()
    );
    cache
        .persistence
        .as_ref()
        .unwrap()
        .skipped_compactions
        .store(COMMAND_RESULT_COMPACTION_SKIP_LIMIT, Ordering::Release);
    replay(&cache, "two").await;
    drop(cache);
    let cache = journal.cache(1);
    assert!(fs::read_to_string(&journal.0)
        .unwrap()
        .starts_with(RECEIPT_EXPIRY_FENCE));
    replay(&cache, "two").await;
    assert!(cache
        .reserve_at_most_once("one", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
    assert!(
        read_persistent_results_with_receipt_fence(&journal.0, cache.retention, false).is_err()
    );
}

#[tokio::test]
async fn failed_marker_append_can_retry_before_restart_without_corrupting_the_journal() {
    let journal = Journal::new();
    let cache = journal.cache(1);
    accept(&cache, "old").await;
    settle(&cache, "old").await;
    drop(cache);
    journal.age();
    let cache = journal.cache(1);
    let mut retention = cache.receipt_retention.lock().await;
    let error = retention.expire_with_write(&journal.0, "old", |file, bytes| {
        file.write_all(&bytes[..64])?;
        Err(io::Error::other("injected newline append failure"))
    });
    assert!(error.is_err());
    assert_eq!(
        fs::metadata(ReceiptRetention::marker_path(&journal.0))
            .unwrap()
            .len(),
        0
    );
    drop(retention);
    accept(&cache, "new").await;
    settle(&cache, "new").await;
    drop(cache);
    let cache = journal.cache(1);
    replay(&cache, "new").await;
    assert!(cache
        .reserve_at_most_once("old", &fingerprint(), Value::Null)
        .await
        .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
}

#[tokio::test]
async fn torn_final_marker_is_recovered_before_a_second_eviction_and_restart() {
    for torn_bytes in [1, 32, 64] {
        let journal = Journal::new();
        let cache = journal.cache(1);
        accept(&cache, "old").await;
        settle(&cache, "old").await;
        drop(cache);
        journal.age();
        let intact = format!("{}\n", ReceiptRetention::identity("historic"));
        let marker = ReceiptRetention::identity("old");
        fs::write(
            ReceiptRetention::marker_path(&journal.0),
            format!("{intact}{}", &marker[..torn_bytes]),
        )
        .unwrap();
        let cache = journal.cache(1);
        replay(&cache, "old").await; // Torn eviction never removed its response.
        assert_eq!(
            fs::read_to_string(ReceiptRetention::marker_path(&journal.0)).unwrap(),
            intact
        );
        accept(&cache, "new").await;
        settle(&cache, "new").await;
        drop(cache);
        let cache = journal.cache(1);
        replay(&cache, "new").await;
        assert!(cache
            .reserve_at_most_once("old", &fingerprint(), Value::Null)
            .await
            .is_err_and(|e| at_most_once::is_receipt_expired_error(&e)));
    }
}

#[test]
fn marker_storage_bound_returns_typed_capacity_without_writing_a_marker() {
    let journal = Journal::new();
    let path = ReceiptRetention::marker_path(&journal.0);
    fs::File::create(&path)
        .unwrap()
        .set_len(COMMAND_RESULT_CACHE_MAX_BYTES)
        .unwrap();
    let mut retention = ReceiptRetention::default();
    let error = retention.expire(&journal.0, "new").unwrap_err();
    assert!(at_most_once::is_receipt_capacity_error(&error));
    assert!(!retention.contains("new"));
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        COMMAND_RESULT_CACHE_MAX_BYTES
    );
}
