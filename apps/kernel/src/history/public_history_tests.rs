//! MP-08 / MP-10 / MP-11 A09/S01-S04: supplementary index regressions.
use super::*;
struct Fixture {
    root: PathBuf,
    store: OperationalHistoryStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("chariox-am9-{:016x}", rand::random::<u64>()));
        fs::create_dir(&root).unwrap();
        let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
        store.set_public_history_projector(Arc::new(|event| {
            Some(PublicHistoryDocument {
                event_ref: event.event_id.clone(),
                sequence: event.sequence,
                timestamp_ms: event.timestamp_ms,
                owner_user_id: "owner".into(),
                session_id: event.session_id.clone()?,
                agent_id: event.agent_id.clone()?,
                kind: event.kind,
                turn_id: event.turn_id.clone(),
                text: event.content.clone()?,
                truncated: false,
            })
        }));
        Self { root, store }
    }
    fn append(&self, room: &str, text: &str) -> HistoryEvent {
        self.store
            .append_operational_event(
                HistoryEventKind::UserPrompt,
                Some(HistoryEventRole::User),
                Some(text.into()),
                Default::default(),
                HistoryEventTurnContext {
                    session_id: Some(room.into()),
                    agent_id: Some("peer".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    }
    fn search(
        &self,
        room: &str,
        text: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<PublicHistorySearchResult, DaemonError> {
        let _guard = self.store.lock_public_history()?;
        self.store
            .search_public_history_locked("owner", room, None, text, limit, cursor)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn public_history_scope_precedes_ranking_and_pagination() {
    let f = Fixture::new();
    for _ in 0..75 {
        f.append("foreign", "compiler compiler compiler foreign_only");
    }
    let first = f.append("room", "compiler regression public_first");
    let second = f.append("room", "compiler regression public_second");
    let third = f.append("room", "compiler regression public_third");
    let a = f.search("room", "compiler", 1, None).unwrap();
    assert_eq!(a.hits.len(), 1);
    assert_eq!(a.coverage.indexed_events, 3);
    let b = f
        .search("room", "compiler", 1, a.next_cursor.as_deref())
        .unwrap();
    let c = f
        .search("room", "compiler", 1, b.next_cursor.as_deref())
        .unwrap();
    let mut ids = vec![
        a.hits[0].event_ref.clone(),
        b.hits[0].event_ref.clone(),
        c.hits[0].event_ref.clone(),
    ];
    ids.sort();
    let mut expected = vec![first.event_id, second.event_id, third.event_id];
    expected.sort();
    assert_eq!(ids, expected);
    assert!(c.next_cursor.is_none());
    assert!(f
        .search("foreign", "compiler", 1, a.next_cursor.as_deref())
        .is_err());
    assert!(f
        .search("room", "regression", 1, a.next_cursor.as_deref())
        .is_err());
    assert!(f
        .search("room", "foreign_only", 50, None)
        .unwrap()
        .hits
        .is_empty());
    let _guard = f.store.lock_public_history().unwrap();
    assert!(f
        .store
        .search_public_history_locked("another_owner", "room", None, "compiler", 50, None)
        .unwrap()
        .hits
        .is_empty());
    assert!(f
        .store
        .read_public_history_locked("owner", "foreign", &ids[0])
        .unwrap()
        .is_none());
    assert!(f
        .store
        .read_public_history_locked("owner", "room", "../../auth.json")
        .unwrap()
        .is_none());
}

#[test]
fn public_history_invalidation_removes_tokens_details_and_cursors() {
    let f = Fixture::new();
    let old = f.append("room", "obsolete_public_marker");
    f.append("room", "obsolete_public_marker");
    let page = f.search("room", "obsolete_public_marker", 1, None).unwrap();
    {
        let _guard = f.store.lock_public_history().unwrap();
        f.store
            .invalidate_public_history_locked(Some("room"))
            .unwrap();
    }
    assert!(f
        .search("room", "obsolete_public_marker", 50, None)
        .unwrap()
        .hits
        .is_empty());
    assert!(f
        .search(
            "room",
            "obsolete_public_marker",
            1,
            page.next_cursor.as_deref()
        )
        .is_err());
    {
        let _guard = f.store.lock_public_history().unwrap();
        assert!(f
            .store
            .read_public_history_locked("owner", "room", &old.event_id)
            .unwrap()
            .is_none());
    }
    assert!(
        !f.search("room", "obsolete", 50, None)
            .unwrap()
            .coverage
            .complete
    );
    // Rebuild is only from sanitized source, so removed tokens cannot reappear.
    f.store.begin_public_history_rebuild().unwrap();
    assert!(f
        .search("room", "obsolete_public_marker", 50, None)
        .unwrap()
        .hits
        .is_empty());
}

#[test]
fn public_history_retention_and_source_updates_invalidate_index() {
    let f = Fixture::new();
    let event = f.append("room", "deleted_marker");
    f.append("room", "deleted_marker");
    let page = f.search("room", "deleted_marker", 1, None).unwrap();
    {
        let connection = f.store.connection.lock().unwrap();
        connection
            .execute(
                "UPDATE history_events SET content='replacement' WHERE event_id=?1",
                [event.event_id.as_str()],
            )
            .unwrap();
    }
    assert_eq!(
        f.search("room", "deleted_marker", 50, None)
            .unwrap()
            .hits
            .len(),
        1
    );
    assert!(f
        .search("room", "deleted_marker", 1, page.next_cursor.as_deref())
        .is_err());
    f.store.prune_events_before(u64::MAX / 2, true).unwrap();
    let result = f.search("room", "deleted_marker", 50, None).unwrap();
    assert!(result.hits.is_empty());
    assert!(!result.coverage.complete);
    assert_eq!(result.coverage.retention_gap_events, 2);
}

#[test]
fn public_history_interrupted_rebuild_resumes_sanitized_source_after_restart() {
    let f = Fixture::new();
    for _ in 0..300 {
        f.append("room", "restartable compiler");
    }
    f.store.begin_public_history_rebuild().unwrap();
    {
        let _guard = f.store.lock_public_history().unwrap();
        f.store.advance_public_history_rebuild_locked(1).unwrap();
    }
    let reopened = OperationalHistoryStore::open(f.store.path().to_path_buf()).unwrap();
    {
        let _guard = reopened.lock_public_history().unwrap();
        let result = reopened
            .search_public_history_locked("owner", "room", None, "compiler", 50, None)
            .unwrap();
        assert!(result.coverage.rebuilding);
        assert!(!result.coverage.complete);
        assert_eq!(result.coverage.rebuild_cursor, 257);
        assert_eq!(result.coverage.indexed_events, 257);
        assert!(result.next_cursor.is_none());
    }
    {
        let _guard = reopened.lock_public_history().unwrap();
        let result = reopened
            .search_public_history_locked("owner", "room", None, "compiler", 50, None)
            .unwrap();
        assert!(!result.coverage.rebuilding);
        assert!(result.coverage.complete);
        assert_eq!(result.coverage.indexed_events, 300);
    }
}

#[test]
fn public_history_version_change_and_unknown_legacy_have_no_raw_fallback() {
    let f = Fixture::new();
    f.append("room", "known_public_text");
    f.store
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE public_history_version SET version=0", [])
        .unwrap();
    let reopened = OperationalHistoryStore::open(f.store.path().to_path_buf()).unwrap();
    // No installed projection means raw events remain unknown and unindexed.
    reopened
        .append_operational_event(
            HistoryEventKind::UserPrompt,
            Some(HistoryEventRole::User),
            Some("unknown_legacy_text".into()),
            Default::default(),
            HistoryEventTurnContext {
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let _guard = reopened.lock_public_history().unwrap();
    for query in ["known_public_text", "unknown_legacy_text"] {
        let result = reopened
            .search_public_history_locked("owner", "room", None, query, 50, None)
            .unwrap();
        assert!(result.hits.is_empty());
        assert!(!result.coverage.complete);
        assert_eq!(result.coverage.excluded_events, 2);
    }
}

#[test]
fn public_history_duplicate_event_cannot_replace_authoritative_public_text() {
    let f = Fixture::new();
    let original = f.append("room", "canonical_text");
    let mut replay = original.clone();
    replay.content = Some("forged_replay_text".into());
    f.store.append(&replay).unwrap();
    assert!(f
        .search("room", "forged_replay_text", 50, None)
        .unwrap()
        .hits
        .is_empty());
    assert_eq!(
        f.search("room", "canonical_text", 50, None)
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn public_history_protocol_453_result_shape_hash() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 453);
    let f = Fixture::new();
    f.append("room", "compiler");
    let mut wire = serde_json::to_value(f.search("room", "compiler", 50, None).unwrap()).unwrap();
    wire["hits"][0]["event_ref"] = serde_json::json!("evt_fixture");
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap()));
    assert_eq!(
        hash,
        "c99672b199f2f982ba174fcc60e96bdf749f09399e615a1b113f0a6b765dde0b"
    );
}

#[test]
fn public_history_rebuild_coverage_never_exposes_foreign_progress() {
    let f = Fixture::new();
    let local = f.append("room", "local compiler");
    for _ in 0..300 {
        f.append("foreign", "foreign compiler");
    }
    f.store.begin_public_history_rebuild().unwrap();
    let local_result = f.search("room", "compiler", 1, None).unwrap();
    assert_eq!(local_result.coverage.indexed_events, 1);
    assert_eq!(local_result.coverage.rebuild_cursor, local.sequence);
    assert!(!local_result.coverage.rebuilding);
    assert!(local_result.coverage.complete);
}

#[test]
fn public_history_recreated_index_rebuilds_from_sanitized_source() {
    let f = Fixture::new();
    for _ in 0..300 {
        f.append("room", "retained compiler");
    }
    f.search("room", "compiler", 50, None).unwrap();
    f.store
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE public_history_fts;")
        .unwrap();
    let reopened = OperationalHistoryStore::open(f.store.path().to_path_buf()).unwrap();
    let _guard = reopened.lock_public_history().unwrap();
    let partial = reopened
        .search_public_history_locked("owner", "room", None, "compiler", 50, None)
        .unwrap();
    assert!(partial.coverage.rebuilding);
    assert_eq!(partial.coverage.indexed_events, 256);
    assert!(partial.next_cursor.is_none());
    let complete = reopened
        .search_public_history_locked("owner", "room", None, "compiler", 50, None)
        .unwrap();
    assert!(complete.coverage.complete);
    assert_eq!(complete.coverage.indexed_events, 300);
}

#[test]
fn public_history_protocol_453_detail_shape_hash() {
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 453);
    let f = Fixture::new();
    let event = f.append("room", "compiler");
    let _guard = f.store.lock_public_history().unwrap();
    let mut detail = f
        .store
        .read_public_history_locked("owner", "room", &event.event_id)
        .unwrap()
        .unwrap();
    detail.event_ref = "evt_fixture".into();
    detail.timestamp_ms = 0;
    let wire = serde_json::to_value(detail).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "b52f6eb046910ca68f1b14fb3e54747cea6f1fa8b834180c658ffc4fff71348b"
    );
}

#[test]
fn public_history_complete_snapshot_keeps_paginating_while_later_rows_rebuild() {
    let f = Fixture::new();
    let first = f.append("room", "compiler first");
    f.append("room", "compiler second");
    f.append("room", "compiler third");
    for _ in 0..800 {
        f.append("foreign", "unrelated activity");
    }
    f.store.begin_public_history_rebuild().unwrap();
    let a = f.search("room", "compiler", 1, None).unwrap();
    assert!(!a.coverage.rebuilding);
    assert!(a.next_cursor.is_some());
    f.append("room", "compiler later");
    let b = f
        .search("room", "compiler", 1, a.next_cursor.as_deref())
        .unwrap();
    assert!(b.coverage.rebuilding);
    assert!(b.next_cursor.is_some());
    let c = f
        .search("room", "compiler", 1, b.next_cursor.as_deref())
        .unwrap();
    assert_eq!(c.hits[0].event_ref, first.event_id);
    assert!(c.next_cursor.is_none());
}

#[test]
fn public_history_queued_append_is_drained_before_protection_invalidation() {
    let f = Fixture::new();
    let event = HistoryEvent::operational(
        f.store.reserve_sequence(),
        HistoryEventKind::UserPrompt,
        Some(HistoryEventRole::User),
        Some("queued public finding".into()),
        Default::default(),
        HistoryEventTurnContext {
            session_id: Some("room".into()),
            agent_id: Some("peer".into()),
            ..Default::default()
        },
    );
    let _guard = f.store.lock_public_history().unwrap();
    let response = f
        .store
        .enqueue_history_records(vec![super::super::OperationalHistoryWriteRecord {
            event_json: serde_json::to_string(&event).unwrap(),
            metadata_text: super::super::searchable_metadata(&event),
            merge_key: None,
            public_document: f.store.project_public_history(&event),
            event: event.clone(),
        }])
        .unwrap();
    f.store
        .invalidate_public_history_locked(Some("room"))
        .unwrap();
    response.recv().unwrap().unwrap();
    assert!(f
        .store
        .read_public_history_locked("owner", "room", &event.event_id)
        .unwrap()
        .is_none());
    assert!(f
        .store
        .search_public_history_locked("owner", "room", None, "finding", 20, None)
        .unwrap()
        .hits
        .is_empty());
}


#[test]
fn public_history_leased_bookkeeping_preserves_search_and_read() {
    use crate::history::leased_projection::{LeasedProjectionCursor, ProjectedHistoryKeys, ProjectedToolState};
    let f = Fixture::new();
    let prompt = f.append("room", "retained compiler prompt");
    let tool = f.store.append_operational_event(
        HistoryEventKind::ProviderTool, Some(HistoryEventRole::Tool),
        Some(r#"{"id":"tool-id","output":"retained compiler tool"}"#.into()),
        Default::default(), HistoryEventTurnContext {
            session_id: Some("room".into()), agent_id: Some("peer".into()),
            provider_run_id: Some("run".into()), ..Default::default()
        },
    ).unwrap();
    let page = f.search("room", "compiler", 1, None).unwrap();
    let assert_retained = || {
        assert_eq!(f.search("room", "compiler", 50, None).unwrap().hits.len(), 2,
            "leased bookkeeping must preserve both public documents and FTS hits");
        let _guard = f.store.lock_public_history().unwrap();
        for event in [&prompt, &tool] {
            let doc = f.store.read_public_history_locked("owner", "room", &event.event_id).unwrap().unwrap();
            assert_eq!(doc.text, event.content.as_ref().unwrap().as_str());
        }
    };
    let cursor = f.store.load_leased_projection_cursor("room:peer:run").unwrap();
    assert_retained(); // schema backfills committed sequence and tool identity
    assert!(cursor == LeasedProjectionCursor::default());
    let keys = ProjectedHistoryKeys { event_id: prompt.event_id.clone(),
        stream_key: Some("prompt-stream".into()), snapshot_key: "prompt-snapshot".into() };
    let mut state = f.store.load_leased_tool_state("tool-stream").unwrap();
    assert!(state.record("tool-snapshot".into(), br#"{"output":"retained compiler tool"}"#, true));
    let tool_state = ProjectedToolState { stream_key: "tool-stream".into(),
        session_id: "room".into(), agent_id: "peer".into(), provider_run_id: "run".into(),
        identity: Some("tool-id".into()), state };
    f.store.commit_leased_projection_cursor("room:peer:run", &cursor, &[], &[keys], &[tool_state]).unwrap();
    assert_retained(); // both ordinary key compaction and tool-state compaction
    assert_eq!(f.search("room", "compiler", 1, page.next_cursor.as_deref()).unwrap().hits[0].event_ref, prompt.event_id);
    let later = f.append("room", "retained compiler later"); // insert's commit-order trigger
    assert_eq!(f.search("room", "compiler", 50, None).unwrap().hits.len(), 3);
    let _guard = f.store.lock_public_history().unwrap();
    assert!(f.store.read_public_history_locked("owner", "room", &later.event_id).unwrap().is_some());
}

#[test]
fn public_history_late_lower_sequence_invalidates_pagination() {
    let f = Fixture::new();
    let first = f.append("room", "compiler first");
    f.append("room", "unrelated second");
    let third = f.append("room", "compiler third");
    let late = HistoryEvent::operational(
        f.store.reserve_sequence(), HistoryEventKind::UserPrompt,
        Some(HistoryEventRole::User), Some("compiler delayed fourth".into()),
        Default::default(), HistoryEventTurnContext {
            session_id: Some("room".into()), agent_id: Some("peer".into()), ..Default::default()
        },
    );
    f.append("room", "unrelated fifth");
    let page = f.search("room", "compiler", 1, None).unwrap();
    assert_eq!(page.hits[0].event_ref, third.event_id);
    assert_eq!(page.coverage.through_sequence, 5);
    f.store.append(&late).unwrap();
    assert!(f.search("room", "compiler", 1, page.next_cursor.as_deref()).is_err(),
        "a late insert below the ceiling must invalidate OFFSET pagination rather than repeat a hit");
    let restart = f.search("room", "compiler", 1, None).unwrap();
    assert_eq!(restart.hits[0].event_ref, late.event_id);
    // Increasing sequence appends do not change the retained snapshot.
    f.append("room", "compiler sixth");
    let second = f.search("room", "compiler", 1, restart.next_cursor.as_deref()).unwrap();
    assert_eq!(second.hits[0].event_ref, third.event_id);
    let last = f.search("room", "compiler", 1, second.next_cursor.as_deref()).unwrap();
    assert_eq!(last.hits[0].event_ref, first.event_id);
    assert!(last.next_cursor.is_none());
}
