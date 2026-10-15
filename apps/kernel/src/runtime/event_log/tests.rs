use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};

use super::{
    EventLog, EventRetentionPolicy, LoggedEvent, ReplayOutcome, DEFAULT_EVENT_ID_RESERVATION_BLOCK,
};

async fn wait_for_persistent_write_settlement(log: &EventLog<String>) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while log.pending_persistent_writes_for_tests() != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("persistent writer should settle its bounded append attempt");
}

static CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct CloneCountedEvent(&'static str);

impl Clone for CloneCountedEvent {
    fn clone(&self) -> Self {
        CLONE_COUNT.fetch_add(1, Ordering::SeqCst);
        Self(self.0)
    }
}

impl Serialize for CloneCountedEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0)
    }
}

#[tokio::test]
async fn appends_monotonic_event_and_stream_sequences() {
    let log = EventLog::new(16);

    let first = log.append("session:a", "first").await.unwrap();
    let second = log.append("session:a", "second").await.unwrap();
    let other = log.append("session:b", "other").await.unwrap();

    assert_eq!(first.event_id, 1);
    assert_eq!(first.stream_seq, 1);
    assert_eq!(second.event_id, 2);
    assert_eq!(second.stream_seq, 2);
    assert_eq!(other.event_id, 3);
    assert_eq!(other.stream_seq, 1);
}

#[tokio::test]
async fn replays_events_after_retained_cursor() {
    let log = EventLog::new(16);
    let first = log.append("session:a", "first").await.unwrap();
    let second = log.append("session:a", "second").await.unwrap();
    let third = log.append("session:a", "third").await.unwrap();

    let replay = log.replay_after("session:a", first.event_id).await;

    match replay {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].event_id, second.event_id);
            assert_eq!(events[1].event_id, third.event_id);
        }
        ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
    }
}

#[tokio::test]
async fn reports_replay_gap_when_cursor_is_older_than_window() {
    let log = EventLog::new(2);
    let first = log.append("session:a", "first").await.unwrap();
    let second = log.append("session:a", "second").await.unwrap();
    let third = log.append("session:a", "third").await.unwrap();

    let replay = log.replay_after("session:a", first.event_id).await;

    match replay {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.stream_id, "session:a");
            assert_eq!(gap.requested_from_event_id, first.event_id);
            assert_eq!(gap.first_retained_event_id, Some(second.event_id));
            assert_eq!(gap.latest_event_id, Some(third.event_id));
        }
        ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
    }
}

#[tokio::test]
async fn retention_does_not_clone_snapshot_without_persistent_compaction() {
    CLONE_COUNT.store(0, Ordering::SeqCst);
    let log = EventLog::new(1);
    log.append("session:a", CloneCountedEvent("first"))
        .await
        .expect("first event should append");
    CLONE_COUNT.store(0, Ordering::SeqCst);

    log.append("session:a", CloneCountedEvent("second"))
        .await
        .expect("second event should append");

    assert_eq!(
        CLONE_COUNT.load(Ordering::SeqCst),
        1,
        "retention should clone only the appended event, not a retained snapshot"
    );
}

static SERIALIZE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug)]
struct SerializeCountedEvent(&'static str);

impl Serialize for SerializeCountedEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SERIALIZE_COUNT.fetch_add(1, Ordering::SeqCst);
        serializer.serialize_str(self.0)
    }
}

#[tokio::test]
async fn retention_drops_events_without_reserializing_them() {
    let log = EventLog::new(1);
    log.append("session:a", SerializeCountedEvent("first"))
        .await
        .expect("first event should append");
    SERIALIZE_COUNT.store(0, Ordering::SeqCst);

    log.append("session:a", SerializeCountedEvent("second"))
        .await
        .expect("second event should append");

    // A cap crossing drops thousands of events on the append path; their
    // sizes are known from when they were retained.
    assert_eq!(SERIALIZE_COUNT.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn persistent_event_ids_resume_above_previous_high_water() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-log-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let path = root.join("kernel.json");
    let first_log = EventLog::new_with_persistent_event_ids(16, &path)
        .expect("first persistent event log should initialize");
    let first = first_log
        .append("session:a", "first")
        .await
        .expect("first event should append");
    assert_eq!(first.event_id, 1);

    let restarted_log = EventLog::new_with_persistent_event_ids(16, &path)
        .expect("restarted persistent event log should initialize");
    let restarted = restarted_log
        .append("session:a", "restarted")
        .await
        .expect("restarted event should append");

    assert!(
        restarted.event_id > DEFAULT_EVENT_ID_RESERVATION_BLOCK,
        "restarted kernel must not emit event ids below the previous browser cursor"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_replays_after_restart() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let first_log =
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("first persistent event store should initialize");
    let first = first_log
        .append("session:a", "first".to_string())
        .await
        .expect("first event should append");
    let second = first_log
        .append("session:a", "second".to_string())
        .await
        .expect("second event should append");
    first_log
        .flush_persistence_for_tests()
        .await
        .expect("persistent writer should flush before restart");

    let restarted_log =
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("restarted persistent event store should initialize");
    let replay = restarted_log
        .replay_after("session:a", first.event_id)
        .await;

    match replay {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, second.event_id);
            assert_eq!(events[0].event, "second");
        }
        ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_recovers_after_transient_append_failure() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-recovery-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let log = EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
        .expect("persistent event store should initialize");

    // Startup already owns the descriptor. Close it before injecting a path
    // open failure; replacing the pathname alone cannot invalidate that fd.
    log.close_persistent_file_for_tests().await;
    std::fs::remove_file(&events_path).expect("startup event file should be replaceable");
    std::fs::create_dir_all(&events_path).expect("event store path should become unwritable");
    log.append("session:a", "failed".to_string())
        .await
        .expect_err("append failure must be visible before an event can be delivered");

    std::fs::remove_dir(&events_path).expect("event store path should become writable again");
    let recovered = log
        .append("session:a", "recovered".to_string())
        .await
        .expect("restored storage should recover without reconstructing the event log");
    let later = log
        .append("session:a", "later".to_string())
        .await
        .expect("later event should preserve durable order");
    log.flush_persistence_for_tests()
        .await
        .expect("recovered events should be durable");

    let restarted =
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("persistent event store should restart after recovery");
    match restarted.replay_after("session:a", 0).await {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.first_retained_event_id, Some(recovered.event_id));
            assert_eq!(gap.latest_event_id, Some(later.event_id));
        }
        ReplayOutcome::Replayed(events) => panic!("failed event id should leave a gap: {events:?}"),
    }
    match restarted
        .replay_after("session:a", recovered.event_id)
        .await
    {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, later.event_id);
            assert_eq!(events[0].event, "later");
        }
        ReplayOutcome::Gap(gap) => panic!("recovered events should replay in order: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_failure_does_not_stall_replay_for_other_streams() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-concurrent-failure-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let log = std::sync::Arc::new(
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("persistent event store should initialize"),
    );
    let stable = log
        .append("session:stable", "stable".to_string())
        .await
        .expect("stable event should append");
    std::fs::remove_file(&events_path).expect("event store file should be replaceable");
    std::fs::create_dir(&events_path).expect("event store path should become unwritable");
    log.close_persistent_file_for_tests().await;

    let gate = log.pause_next_persistent_append_for_tests();
    let first_task = {
        let log = std::sync::Arc::clone(&log);
        tokio::spawn(async move { log.append("session:a", "first".to_string()).await })
    };
    gate.wait_until_entered().await;
    let second_task = {
        let log = std::sync::Arc::clone(&log);
        tokio::spawn(async move { log.append("session:b", "second".to_string()).await })
    };
    tokio::task::yield_now().await;
    assert!(
        !second_task.is_finished(),
        "persistent appends should retain documented global ordering"
    );

    let replay = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        log.replay_after("session:stable", stable.event_id),
    )
    .await
    .expect("storage retries must not hold the global streams mutex");
    assert!(matches!(replay, ReplayOutcome::Replayed(events) if events.is_empty()));

    gate.release();
    first_task
        .await
        .expect("first append task should join")
        .expect_err("first stream append should expose sustained storage failure");
    second_task
        .await
        .expect("second append task should join")
        .expect_err("second stream append should expose sustained storage failure");
    assert_eq!(log.pending_persistent_writes_for_tests(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_cancelled_failed_append_is_never_replay_visible() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-cancelled-failure-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let log = std::sync::Arc::new(
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("persistent event store should initialize"),
    );
    // Startup already owns the descriptor. Close it before injecting a path
    // open failure; replacing the pathname alone cannot invalidate that fd.
    log.close_persistent_file_for_tests().await;
    std::fs::remove_file(&events_path).expect("startup event file should be replaceable");
    std::fs::create_dir_all(&events_path).expect("event store path should become unwritable");

    let gate = log.pause_next_persistent_append_for_tests();
    let append_task = {
        let log = std::sync::Arc::clone(&log);
        tokio::spawn(async move { log.append("session:a", "must-not-replay".to_string()).await })
    };
    gate.wait_until_entered().await;
    assert!(matches!(
        log.replay_after("session:a", 0).await,
        ReplayOutcome::Replayed(events) if events.is_empty()
    ));
    append_task.abort();
    assert!(append_task
        .await
        .expect_err("append task should be cancelled")
        .is_cancelled());
    gate.release();
    wait_for_persistent_write_settlement(&log).await;

    match log.replay_after("session:a", 0).await {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.first_retained_event_id, None);
            assert_eq!(gap.latest_event_id, None);
        }
        ReplayOutcome::Replayed(events) => {
            panic!("failed cancelled append must not be replay visible: {events:?}")
        }
    }

    std::fs::remove_dir(&events_path).expect("event store path should become writable again");
    let recovered = log
        .append("session:a", "recovered".to_string())
        .await
        .expect("same event log should recover after cancelled failure");
    match log.replay_after("session:a", 0).await {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.first_retained_event_id, Some(recovered.event_id));
            assert_eq!(gap.latest_event_id, Some(recovered.event_id));
        }
        ReplayOutcome::Replayed(events) => panic!("cancelled id should leave a gap: {events:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_cancelled_successful_append_reconciles_in_memory_and_on_restart() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-cancelled-success-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let log = std::sync::Arc::new(
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("persistent event store should initialize"),
    );
    let anchor = log
        .append("session:a", "anchor".to_string())
        .await
        .expect("anchor event should append");

    let gate = log.pause_next_persistent_append_for_tests();
    let append_task = {
        let log = std::sync::Arc::clone(&log);
        tokio::spawn(async move {
            log.append("session:a", "durable-after-cancel".to_string())
                .await
        })
    };
    gate.wait_until_entered().await;
    append_task.abort();
    assert!(append_task
        .await
        .expect_err("append task should be cancelled")
        .is_cancelled());
    gate.release();
    wait_for_persistent_write_settlement(&log).await;

    match log.replay_after("session:a", anchor.event_id).await {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event, "durable-after-cancel");
        }
        ReplayOutcome::Gap(gap) => panic!("durable cancelled append should replay: {gap:?}"),
    }
    let restarted =
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("persistent event store should restart");
    match restarted.replay_after("session:a", anchor.event_id).await {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event, "durable-after-cancel");
        }
        ReplayOutcome::Gap(gap) => panic!("durable cancelled append should restart: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_truncates_a_torn_tail_before_append() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-torn-tail-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    std::fs::create_dir_all(&root).expect("event store root should create");
    std::fs::write(&counter_path, r#"{"high_water_event_id":7}"#)
        .expect("event counter should seed");
    let first = LoggedEvent {
        event_id: 7,
        stream_id: "session:a".to_string(),
        stream_seq: 1,
        recorded_at_ms: super::unix_epoch_ms(),
        event: "first".to_string(),
    };
    let mut payload = super::logged_event_jsonl_payload(&first).expect("first event should encode");
    payload.extend_from_slice(br#"{"event_id":8,"stream_id":"session:a""#);
    std::fs::write(&events_path, payload).expect("torn event store should seed");

    let log = EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
        .expect("persistent event store should tolerate a torn tail");
    let recovered = log
        .append("session:a", "second".to_string())
        .await
        .expect("append should replace the torn tail with a complete record");
    log.flush_persistence_for_tests()
        .await
        .expect("repaired event store should flush");

    let stored = std::fs::read_to_string(&events_path).expect("event store should remain readable");
    assert!(stored.ends_with('\n'));
    assert_eq!(stored.lines().count(), 2);
    for line in stored.lines() {
        serde_json::from_str::<LoggedEvent<String>>(line)
            .expect("every retained event record should be complete");
    }

    let restarted =
        EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
            .expect("repaired event store should restart");
    match restarted.replay_after("session:a", first.event_id).await {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, recovered.event_id);
            assert_eq!(events[0].event, "second");
        }
        ReplayOutcome::Gap(gap) => panic!("repaired event should replay: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_skips_malformed_lines() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-malformed-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    std::fs::create_dir_all(&root).expect("event store root should create");
    std::fs::write(
        &events_path,
        concat!(
            "not-json\n",
            "{\"event_id\":7,\"stream_id\":\"session:a\",\"stream_seq\":1,\"event\":\"first\"}\n",
            "{\"event_id\":8,\"stream_id\":\"session:a\",\"stream_seq\":2,\"event\":\"second\"}\n"
        ),
    )
    .expect("event store should seed");

    let log = EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
        .expect("persistent event store should tolerate malformed lines");
    let replay = log.replay_after("session:a", 7).await;

    match replay {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, 8);
            assert_eq!(events[0].event, "second");
        }
        ReplayOutcome::Gap(gap) => panic!("unexpected replay gap: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_store_compacts_by_total_bytes() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-bytes-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let retention = EventRetentionPolicy {
        max_stream_events: 16,
        max_total_bytes: Some(160),
        max_age_ms: None,
    };
    let log = EventLog::<String>::new_with_persistent_event_store_and_retention(
        retention,
        &counter_path,
        &events_path,
    )
    .expect("persistent event store should initialize");
    let first = log
        .append("session:a", "first".to_string())
        .await
        .expect("first event should append");
    let second = log
        .append("session:a", "second".to_string())
        .await
        .expect("second event should append");

    let replay = log.replay_after("session:a", first.event_id).await;
    match replay {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.first_retained_event_id, Some(second.event_id));
            assert_eq!(gap.latest_event_id, Some(second.event_id));
        }
        ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
    }
    for index in 0..8 {
        log.append("session:a", format!("extra-{index}"))
            .await
            .expect("extra event should append");
        log.flush_persistence_for_tests()
            .await
            .expect("persistent writer should flush before reading event store");
        let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
        // Compaction runs beside appends; one that is still running when the
        // next compaction is due defers it to a later append.
        if !stored.contains("\"first\"")
            && stored.len() as u64
                <= super::persistent_compaction_target_bytes(retention.max_total_bytes.unwrap())
        {
            let _ = std::fs::remove_dir_all(root);
            return;
        }
    }
    panic!("event store should compact after bounded file growth");
}

#[tokio::test]
async fn persistent_event_store_compacts_by_event_age_on_load() {
    let root = std::env::temp_dir().join(format!(
        "chariox-event-store-age-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ));
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    std::fs::create_dir_all(&root).expect("event store root should create");
    let now_ms = super::unix_epoch_ms();
    let old = LoggedEvent {
        event_id: 7,
        stream_id: "session:a".to_string(),
        stream_seq: 1,
        recorded_at_ms: now_ms.saturating_sub(10_000),
        event: "old".to_string(),
    };
    let fresh = LoggedEvent {
        event_id: 8,
        stream_id: "session:a".to_string(),
        stream_seq: 2,
        recorded_at_ms: now_ms,
        event: "fresh".to_string(),
    };
    super::rewrite_logged_events(&events_path, &[old, fresh]).expect("event store should seed");
    let retention = EventRetentionPolicy {
        max_stream_events: 16,
        max_total_bytes: None,
        max_age_ms: Some(1_000),
    };

    let log = EventLog::<String>::new_with_persistent_event_store_and_retention(
        retention,
        &counter_path,
        &events_path,
    )
    .expect("persistent event store should initialize");

    let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
    assert!(!stored.contains("\"old\""));
    assert!(stored.contains("\"fresh\""));

    let replay = log.replay_after("session:a", 7).await;
    match replay {
        ReplayOutcome::Gap(gap) => {
            assert_eq!(gap.first_retained_event_id, Some(8));
            assert_eq!(gap.latest_event_id, Some(8));
        }
        ReplayOutcome::Replayed(events) => panic!("expected replay gap, got {events:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compaction_serializes_borrowed_events_in_global_order() {
    #[derive(Serialize)]
    struct BorrowOnlyEvent(&'static str);

    let mut streams = std::collections::BTreeMap::new();
    for (stream_id, event_ids) in [("a", vec![2, 4]), ("b", vec![1, 3])] {
        let mut stream = super::EventStream::default();
        for event_id in event_ids {
            stream.retained.push_back(Box::new(LoggedEvent {
                event_id,
                stream_id: stream_id.to_string(),
                stream_seq: event_id,
                recorded_at_ms: 1,
                event: BorrowOnlyEvent("payload"),
            }));
        }
        streams.insert(stream_id.to_string(), stream);
    }
    // This payload has no Clone implementation: compaction must only borrow it.
    let payload = super::retained_events_jsonl_payload(&streams).unwrap();
    let events = String::from_utf8(payload)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<LoggedEvent<String>>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(events.iter().all(|event| event.event == "payload"));
}

fn scratch_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "chariox-event-store-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos()
    ))
}

#[tokio::test]
async fn persistent_event_store_appends_through_one_open_file() {
    let root = scratch_root("open-file");
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    let log = EventLog::<String>::new_with_persistent_event_store(16, &counter_path, &events_path)
        .expect("persistent event store should initialize");
    log.append("session:a", "first".to_string())
        .await
        .expect("first event should append");

    // The writer keeps its descriptor: no open, stat or tail read per append.
    let moved_path = root.join("moved.jsonl");
    std::fs::rename(&events_path, &moved_path).expect("event file should move");
    log.append("session:a", "second".to_string())
        .await
        .expect("second event should append");
    log.flush_persistence_for_tests()
        .await
        .expect("persistent writer should flush");

    let moved = std::fs::read_to_string(&moved_path).expect("open event file should remain");
    assert_eq!(moved.lines().count(), 2, "{moved}");
    assert!(!events_path.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn persistent_event_store_compaction_does_not_block_appends() {
    use std::io::Read;

    let root = scratch_root("compaction-off-append");
    let counter_path = root.join("counter.json");
    let events_path = root.join("events.jsonl");
    std::fs::create_dir_all(&root).expect("event store root should create");
    // A compaction side file that cannot be opened stalls only the compactor.
    let stalled_side_files = [
        events_path.with_extension("jsonl.tmp"),
        events_path.with_extension("jsonl.compact"),
    ];
    for side_file in &stalled_side_files {
        let path = std::ffi::CString::new(side_file.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }
    let retention = EventRetentionPolicy {
        max_stream_events: 16,
        max_total_bytes: Some(400),
        max_age_ms: None,
    };
    let log = EventLog::<String>::new_with_persistent_event_store_and_retention(
        retention,
        &counter_path,
        &events_path,
    )
    .expect("persistent event store should initialize");
    for index in 0..24 {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            log.append("session:a", format!("event-{index:02}")),
        )
        .await
        .expect("append must not wait for event log compaction")
        .expect("event should append");
    }

    // Release the stalled compactor. A pipe cannot be synced and its path is
    // gone, so that compaction fails cleanly and the next append starts afresh.
    let stalled = stalled_side_files[1].clone();
    let drain = std::thread::spawn(move || {
        let mut side = std::fs::File::open(&stalled).expect("stalled side file should open");
        // The failed compactor may already have removed its side file.
        let _ = std::fs::remove_file(&stalled);
        let mut discarded = Vec::new();
        side.read_to_end(&mut discarded)
            .expect("stalled side file should drain");
    });
    std::fs::remove_file(&stalled_side_files[0]).expect("unused side file should unlink");
    tokio::task::spawn_blocking(move || drain.join().expect("drain should finish"))
        .await
        .expect("drain task should join");
    log.flush_persistence_for_tests()
        .await
        .expect("stalled compaction should settle");
    // Appends that arrive while a compaction runs defer the next one to a
    // later append, so the file converges to the retention budget.
    let mut latest = None;
    for index in 0..16 {
        latest = Some(
            log.append("session:a", format!("after-{index:02}"))
                .await
                .expect("append should start a fresh compaction"),
        );
        log.flush_persistence_for_tests()
            .await
            .expect("fresh compaction should settle");
        let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
        if !stored.contains("event-") && stored.len() as u64 <= retention.max_total_bytes.unwrap() {
            break;
        }
    }
    let latest = latest.unwrap();
    let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
    assert!(
        stored.len() as u64 <= retention.max_total_bytes.unwrap() && !stored.contains("event-"),
        "event store should compact: {stored}"
    );
    assert!(stored.ends_with("\n"));
    let restarted = EventLog::<String>::new_with_persistent_event_store_and_retention(
        retention,
        &counter_path,
        &events_path,
    )
    .expect("compacted event store should restart");
    match restarted
        .replay_after("session:a", latest.event_id - 1)
        .await
    {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, latest.event_id);
        }
        ReplayOutcome::Gap(gap) => panic!("latest event should replay: {gap:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn persistent_event_file_is_open_before_first_append() {
    let root = scratch_root("warm-event-file");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("events.jsonl");
    let log =
        EventLog::<String>::new_with_persistent_event_store(16, root.join("counter.json"), &path)
            .unwrap();
    assert!(
        path.is_file(),
        "startup must open the event file before admission begins"
    );
    // Renaming an already opened file must not force the first append to do
    // path lookup, directory creation or torn-tail recovery.
    let moved = root.join("opened.jsonl");
    std::fs::rename(&path, &moved).unwrap();
    log.append("a", "first".to_string()).await.unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read_to_string(&moved).unwrap().lines().count(), 1);
    drop(log);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compaction_keeps_the_replacement_open_and_returns_the_retired_inode() {
    use std::sync::{atomic::AtomicU64, Arc};
    let root = scratch_root("retire-event-inode");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("events.jsonl");
    let side = root.join("events.jsonl.compact");
    let mut writer = super::EventFileWriter {
        path: path.clone(),
        file: None,
        file_bytes: Arc::new(AtomicU64::new(0)),
        append_space: None,
        space_reservation_started: false,
    };
    writer.append(b"first\nsecond\n").unwrap();
    std::fs::write(&side, b"second\n").unwrap();
    let retired = writer
        .swap_compacted(&side, 13)
        .unwrap()
        .expect("the compactor must receive the old inode");
    assert_eq!(retired.metadata().unwrap().len(), 13);
    assert!(
        writer.file.is_some(),
        "the next append must reuse the replacement descriptor"
    );
    writer.append(b"third\n").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second\nthird\n");
    // On the fixed path, this owns the unlinked inode until the compactor
    // retires it; the writer never performs its last close on an append.
    drop(retired);
    drop(writer);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn growing_a_wrapped_retention_queue_does_not_relocate_existing_payloads() {
    let mut stream = super::EventStream::default();
    stream.retained.reserve(8);
    let capacity = stream.retained.capacity();
    let event = |id| LoggedEvent {
        event_id: id,
        stream_id: "a".to_string(),
        stream_seq: id,
        recorded_at_ms: 1,
        event: id,
    };
    for id in 0..capacity as u64 {
        stream.retain(event(id), 1);
    }
    for id in 0..capacity as u64 / 2 {
        stream.drop_oldest();
        stream.retain(event(capacity as u64 + id), 1);
    }
    let pointers = stream
        .retained
        .iter()
        .map(|event| (event.event_id, std::ptr::from_ref(&event.event)))
        .collect::<std::collections::BTreeMap<_, _>>();
    stream.retain(event(2 * capacity as u64), 1);
    assert!(stream.retained.capacity() > capacity);
    for event in &stream.retained {
        if let Some(previous) = pointers.get(&event.event_id) {
            assert_eq!(*previous, std::ptr::from_ref(&event.event));
        }
    }
}

#[test]
fn reserving_append_space_preserves_logical_bytes_and_append_position() {
    use std::io::{Seek, Write};
    let root = scratch_root("reserve-event-space");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("events.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(b"first\n").unwrap();
    let position = file.stream_position().unwrap();
    super::reserve_event_file_space(&file, 1024 * 1024);
    assert_eq!(file.metadata().unwrap().len(), 6);
    assert_eq!(file.stream_position().unwrap(), position);
    file.write_all(b"second\n").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first\nsecond\n");
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn byte_retention_keeps_the_newest_global_ids_across_many_streams() {
    let mut streams = std::collections::BTreeMap::new();
    for id in 1..=12_000_u64 {
        let key = format!("stream-{}", id % 300);
        streams
            .entry(key.clone())
            .or_insert_with(super::EventStream::default)
            .retain(
                LoggedEvent {
                    event_id: id,
                    stream_id: key,
                    stream_seq: id / 300 + 1,
                    recorded_at_ms: 1,
                    event: (),
                },
                1,
            );
    }
    let dropped = super::apply_retention(
        &mut streams,
        EventRetentionPolicy {
            max_stream_events: usize::MAX,
            max_total_bytes: Some(10_000),
            max_age_ms: None,
        },
        1,
    );
    assert_eq!(
        dropped.iter().map(|e| e.event_id).collect::<Vec<_>>(),
        (1..=4_500).collect::<Vec<_>>()
    );
    let mut retained = streams
        .values()
        .flat_map(|s| s.retained.iter().map(|e| e.event_id))
        .collect::<Vec<_>>();
    retained.sort_unstable();
    assert_eq!(retained, (4_501..=12_000).collect::<Vec<_>>());
    assert_eq!(super::retained_events_jsonl_bytes(&streams), 7_500);
}

#[test]
fn compaction_finishes_all_data_bursts_before_handoff() {
    use std::io::Write;
    let root = scratch_root("compaction-data-bursts");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("events.jsonl");
    let side = root.join("events.jsonl.compact");
    let mut file = std::fs::File::create(&path).unwrap();
    let mut kept = std::collections::HashSet::new();
    let mut expected = Vec::new();
    for id in 1..=1024 {
        let event = LoggedEvent {
            event_id: id,
            stream_id: format!("stream-{}", id % 16),
            stream_seq: id,
            recorded_at_ms: 1,
            event: format!("{id}:{}", "x".repeat(1024)),
        };
        let payload = super::logged_event_jsonl_payload(&event).unwrap();
        file.write_all(&payload).unwrap();
        if id % 3 != 0 {
            kept.insert(id);
            expected.extend_from_slice(&payload);
        }
    }
    file.sync_all().unwrap();
    let bytes = file.metadata().unwrap().len();
    assert!(expected.len() > 8 * super::COMPACTION_WRITEBACK_BYTES);
    super::write_compacted_side_file(&path, &side, bytes, &kept, None).unwrap();
    assert_eq!(std::fs::read(&side).unwrap(), expected);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}
