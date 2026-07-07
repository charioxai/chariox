use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};

use super::{
    DEFAULT_EVENT_ID_RESERVATION_BLOCK, EventLog, EventRetentionPolicy, LoggedEvent, ReplayOutcome,
};

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

#[tokio::test]
async fn persistent_event_ids_resume_above_previous_high_water() {
    let root = std::env::temp_dir().join(format!(
        "arroba-event-log-test-{}-{}",
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
        "arroba-event-store-test-{}-{}",
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
async fn persistent_event_store_skips_malformed_lines() {
    let root = std::env::temp_dir().join(format!(
        "arroba-event-store-malformed-test-{}-{}",
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
        "arroba-event-store-bytes-test-{}-{}",
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
            .expect("persistent writer should flush before reading event store");
        let stored = std::fs::read_to_string(&events_path).expect("event store should exist");
        if !stored.contains("\"first\"") {
            assert!(
                stored.len() as u64
                    <= super::persistent_compaction_target_bytes(
                        retention.max_total_bytes.unwrap()
                    ),
                "event store should compact with headroom after bounded file growth: {stored}"
            );
            let _ = std::fs::remove_dir_all(root);
            return;
        }
    }
    panic!("event store should compact after bounded file growth");
}

#[tokio::test]
async fn persistent_event_store_compacts_by_event_age_on_load() {
    let root = std::env::temp_dir().join(format!(
        "arroba-event-store-age-test-{}-{}",
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
