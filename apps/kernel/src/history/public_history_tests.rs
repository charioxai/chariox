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
                Some(text.to_owned()),
                Default::default(),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
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
    // MP-08 / MP-10 / MP-11: every predecessor, including colliding-worker v3.
    for predecessor in [1, 2, 3] {
        let f = Fixture::new();
        f.append("room", "known_public_text");
        f.store
            .connection
            .lock()
            .unwrap()
            // Old projections can contain private MCP records.
            .execute(
                "UPDATE public_history_version SET version=?1",
                [predecessor],
            )
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
                    public_history_owner_user_id: Some("owner".into()),
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
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 472);
    let f = Fixture::new();
    f.append("room", "compiler");
    let mut wire = serde_json::to_value(f.search("room", "compiler", 50, None).unwrap()).unwrap();
    wire["hits"][0]["event_ref"] = serde_json::json!("evt_fixture");
    // MP-08 / MP-10 / MP-11: the index fence is an internal revision;
    // keep the protocol453 shape hash independent of its value.
    assert_eq!(
        wire["coverage"]["index_version"],
        serde_json::json!(PUBLIC_HISTORY_VERSION)
    );
    assert_eq!(
        wire["coverage"]["redaction_version"],
        serde_json::json!(PUBLIC_HISTORY_VERSION)
    );
    wire["coverage"]["index_version"] = serde_json::json!(3);
    wire["coverage"]["redaction_version"] = serde_json::json!(3);
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap()));
    assert_eq!(
        hash,
        "d4559571dd2a2a647b55a49e3bc155b372acbff14ed7228057c4bf4b20e0f717"
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
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 472);
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
            public_history_owner_user_id: Some("owner".into()),
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
    use crate::history::leased_projection::{
        LeasedProjectionCursor, ProjectedHistoryKeys, ProjectedToolState,
    };
    let f = Fixture::new();
    let prompt = f.append("room", "retained compiler prompt");
    let tool = f
        .store
        .append_operational_event(
            HistoryEventKind::ProviderTool,
            Some(HistoryEventRole::Tool),
            Some(r#"{"id":"tool-id","output":"retained compiler tool"}"#.into()),
            Default::default(),
            HistoryEventTurnContext {
                public_history_owner_user_id: Some("owner".into()),
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                provider_run_id: Some("run".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let page = f.search("room", "compiler", 1, None).unwrap();
    let assert_retained = || {
        assert_eq!(
            f.search("room", "compiler", 50, None).unwrap().hits.len(),
            2,
            "leased bookkeeping must preserve both public documents and FTS hits"
        );
        let _guard = f.store.lock_public_history().unwrap();
        for event in [&prompt, &tool] {
            let doc = f
                .store
                .read_public_history_locked("owner", "room", &event.event_id)
                .unwrap()
                .unwrap();
            assert_eq!(doc.text, event.content.as_ref().unwrap().as_str());
        }
    };
    let cursor = f
        .store
        .load_leased_projection_cursor("room:peer:run")
        .unwrap();
    assert_retained(); // schema backfills committed sequence and tool identity
    assert!(cursor == LeasedProjectionCursor::default());
    let keys = ProjectedHistoryKeys {
        event_id: prompt.event_id.clone(),
        stream_key: Some("prompt-stream".into()),
        snapshot_key: "prompt-snapshot".into(),
    };
    let mut state = f.store.load_leased_tool_state("tool-stream").unwrap();
    assert!(state.record(
        "tool-snapshot".into(),
        br#"{"output":"retained compiler tool"}"#,
        true
    ));
    let tool_state = ProjectedToolState {
        stream_key: "tool-stream".into(),
        session_id: "room".into(),
        agent_id: "peer".into(),
        provider_run_id: "run".into(),
        identity: Some("tool-id".into()),
        state,
    };
    f.store
        .commit_leased_projection_cursor("room:peer:run", &cursor, &[], &[keys], &[tool_state])
        .unwrap();
    assert_retained(); // both ordinary key compaction and tool-state compaction
    assert_eq!(
        f.search("room", "compiler", 1, page.next_cursor.as_deref())
            .unwrap()
            .hits[0]
            .event_ref,
        prompt.event_id
    );
    let later = f.append("room", "retained compiler later"); // insert's commit-order trigger
    assert_eq!(
        f.search("room", "compiler", 1, page.next_cursor.as_deref())
            .unwrap()
            .hits[0]
            .event_ref,
        prompt.event_id,
        "leased insert bookkeeping must preserve an existing bounded cursor"
    );
    assert_eq!(
        f.search("room", "compiler", 50, None).unwrap().hits.len(),
        3
    );
    let _guard = f.store.lock_public_history().unwrap();
    assert!(f
        .store
        .read_public_history_locked("owner", "room", &later.event_id)
        .unwrap()
        .is_some());
}

#[test]
fn public_history_late_lower_sequence_invalidates_pagination() {
    let f = Fixture::new();
    let first = f.append("room", "compiler first");
    f.append("room", "unrelated second");
    let third = f.append("room", "compiler third");
    let late = HistoryEvent::operational(
        f.store.reserve_sequence(),
        HistoryEventKind::UserPrompt,
        Some(HistoryEventRole::User),
        Some("compiler delayed fourth".into()),
        Default::default(),
        HistoryEventTurnContext {
            public_history_owner_user_id: Some("owner".into()),
            session_id: Some("room".into()),
            agent_id: Some("peer".into()),
            ..Default::default()
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
    let second = f
        .search("room", "compiler", 1, restart.next_cursor.as_deref())
        .unwrap();
    assert_eq!(second.hits[0].event_ref, third.event_id);
    let last = f
        .search("room", "compiler", 1, second.next_cursor.as_deref())
        .unwrap();
    assert_eq!(last.hits[0].event_ref, first.event_id);
    assert!(last.next_cursor.is_none());
}

#[test]
fn public_history_existing_database_refreshes_bookkeeping_trigger() {
    let f = Fixture::new();
    let event = f.append("room", "retained compiler");
    // Simulate the installed pre-fix trigger, then run the ordinary open path.
    f.store
        .connection
        .lock()
        .unwrap()
        .execute_batch(
            "DROP TRIGGER public_history_source_update;
         CREATE TRIGGER public_history_source_update AFTER UPDATE ON history_events BEGIN
           DELETE FROM public_history WHERE event_ref=old.event_id;
         END;",
        )
        .unwrap();
    let reopened = OperationalHistoryStore::open(f.store.path().to_path_buf()).unwrap();
    reopened
        .load_leased_projection_cursor("room:peer:run")
        .unwrap();
    let _guard = reopened.lock_public_history().unwrap();
    assert_eq!(
        reopened
            .search_public_history_locked("owner", "room", None, "compiler", 50, None)
            .unwrap()
            .hits
            .len(),
        1
    );
    assert!(reopened
        .read_public_history_locked("owner", "room", &event.event_id)
        .unwrap()
        .is_some());
}

#[test]
fn public_history_noop_source_update_preserves_documents_but_scope_and_json_changes_invalidate() {
    for column in ["session_id", "agent_id", "event_json"] {
        let f = Fixture::new();
        let event = f.append("room", "compiler retained");
        f.append("room", "compiler another");
        let page = f.search("room", "compiler", 1, None).unwrap();
        {
            let connection = f.store.connection.lock().unwrap();
            connection.execute("UPDATE history_events SET content=content,event_json=event_json,session_id=session_id WHERE event_id=?1", [&event.event_id]).unwrap();
        }
        assert!(f
            .search("room", "compiler", 1, page.next_cursor.as_deref())
            .is_ok());
        {
            let connection = f.store.connection.lock().unwrap();
            let replacement = if column == "event_json" {
                "{}"
            } else {
                "other"
            };
            connection
                .execute(
                    &format!("UPDATE history_events SET {column}=?2 WHERE event_id=?1"),
                    params![event.event_id, replacement],
                )
                .unwrap();
        }
        assert!(f
            .search("room", "compiler", 1, page.next_cursor.as_deref())
            .is_err());
        let _guard = f.store.lock_public_history().unwrap();
        assert!(f
            .store
            .read_public_history_locked("owner", "room", &event.event_id)
            .unwrap()
            .is_none());
    }
}

#[test]
fn public_history_turn_selection_is_scoped_and_distinguishes_missing_from_null_turn() {
    let f = Fixture::new();
    let append = |room: &str, peer: &str, turn: Option<&str>, text: &str| {
        f.store
            .append_operational_event(
                HistoryEventKind::UserPrompt,
                Some(HistoryEventRole::User),
                Some(text.to_owned()),
                Default::default(),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some(room.into()),
                    agent_id: Some(peer.into()),
                    turn_id: turn.map(str::to_owned),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let previous = append("room", "peer", Some("shared-turn"), "public previous");
    append("room", "peer", None, "unattributed newest");
    append("foreign", "peer", Some("shared-turn"), "foreign marker");
    append(
        "room",
        "other-peer",
        Some("shared-turn"),
        "other peer marker",
    );
    let _guard = f.store.lock_public_history().unwrap();
    let selected = f
        .store
        .public_history_turn_locked("owner", "room", "peer", Some("shared-turn"), 99, 1)
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].event_ref, previous.event_id);
    let selected = f
        .store
        .public_history_turn_locked("owner", "room", "peer", None, 1, 200)
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].event_ref, previous.event_id);
    assert!(f
        .store
        .public_history_turn_locked("owner", "room", "peer", None, 2, 200)
        .unwrap()
        .is_empty());
    assert!(f
        .store
        .public_history_turn_locked("owner", "room", "peer", Some("missing"), 0, 200)
        .unwrap()
        .is_empty());
    assert!(f
        .store
        .public_history_turn_locked("another-owner", "room", "peer", Some("shared-turn"), 0, 200)
        .unwrap()
        .is_empty());
}

// MP-08 / MP-10 / MP-11: reproduce official Codex deltas and prompt/native IDs.
#[test]
fn public_history_recalls_fragmented_peer_answer_as_one_message() {
    let f = Fixture::new();
    let append = |room: &str, agent: &str, run: &str, prompt: &str, key: &str, text: &str| {
        f.store
            .append_transcript(
                &SessionHistoryEntry::provider_output(
                    room,
                    run,
                    Some(agent),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some(key.into()),
                    text.to_owned(),
                ),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some(room.into()),
                    agent_id: Some(agent.into()),
                    provider_run_id: Some(run.into()),
                    prompt_id: Some(prompt.into()),
                    turn_id: Some("native-turn".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let first = append("room", "peer", "run", "prompt", "message", "The stale ");
    let middle = append("room", "peer", "run", "prompt", "message", "cursor");
    append(
        "room",
        "peer",
        "run",
        "prompt",
        "message",
        " must restart the query.",
    );
    append(
        "foreign",
        "peer",
        "run",
        "prompt",
        "message",
        "foreign content",
    );
    append(
        "room",
        "other-peer",
        "run",
        "prompt",
        "message",
        "other peer",
    );
    append(
        "room",
        "peer",
        "other-run",
        "prompt",
        "message",
        "other run",
    );
    append(
        "room",
        "peer",
        "run",
        "other-prompt",
        "message",
        "other turn",
    );
    append(
        "room",
        "peer",
        "run",
        "prompt",
        "other-message",
        "other message",
    );
    let page = f.search("room", "cursor", 50, None).unwrap();
    assert_eq!(page.hits.len(), 1);
    let _guard = f.store.lock_public_history().unwrap();
    for reference in [&first.event_id, &middle.event_id, &page.hits[0].event_ref] {
        let doc = f
            .store
            .read_public_history_locked("owner", "room", reference)
            .unwrap()
            .unwrap();
        assert_eq!(doc.text, "The stale cursor must restart the query.");
        assert_eq!(doc.turn_id.as_deref(), Some("prompt"));
    }
    let turn = f
        .store
        .public_history_turn_locked("owner", "room", "peer", Some("prompt"), 0, 200)
        .unwrap();
    assert_eq!(
        turn.iter()
            .filter(|doc| doc.text == "The stale cursor must restart the query.")
            .count(),
        1
    );
}

#[test]
fn public_history_previous_turn_groups_prompt_and_native_output() {
    let f = Fixture::new();
    let append = |kind, prompt: &str, native: &str, text: &str| {
        f.store
            .append_operational_event(
                kind,
                None,
                Some(text.to_owned()),
                Default::default(),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    prompt_id: Some(prompt.into()),
                    turn_id: Some(native.into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let previous = append(
        HistoryEventKind::UserPrompt,
        "previous",
        "previous",
        "previous task",
    );
    let answer = append(
        HistoryEventKind::ProviderOutput,
        "previous",
        "native-previous",
        "previous answer",
    );
    append(
        HistoryEventKind::UserPrompt,
        "latest",
        "latest",
        "latest task",
    );
    for _ in 0..225 {
        append(
            HistoryEventKind::ProviderOutput,
            "latest",
            "native-latest",
            "latest fragment",
        );
    }
    let _guard = f.store.lock_public_history().unwrap();
    for reference in [None, Some("previous"), Some("native-previous")] {
        let turn = f
            .store
            .public_history_turn_locked("owner", "room", "peer", reference, 1, 200)
            .unwrap();
        assert_eq!(
            turn.iter().map(|doc| &doc.event_ref).collect::<Vec<_>>(),
            vec![&answer.event_id, &previous.event_id]
        );
        assert!(turn
            .iter()
            .all(|doc| doc.turn_id.as_deref() == Some("previous")));
    }
}

#[test]
fn public_history_assembled_answer_is_bounded_and_uses_only_retained_public_parts() {
    // MP-08 / MP-10 / MP-11: bound Unicode text after joining, then prove
    // removal of the public message cannot resurrect its raw content.
    let f = Fixture::new();
    let append = |text: String| {
        f.store
            .append_transcript(
                &SessionHistoryEntry::provider_output(
                    "room",
                    "run",
                    Some("peer"),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some("message".into()),
                    text,
                ),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    provider_run_id: Some("run".into()),
                    prompt_id: Some("prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let first = append("界".repeat(40_000));
    let second = append("界".repeat(40_000));
    let _guard = f.store.lock_public_history().unwrap();
    let document = f
        .store
        .read_public_history_locked("owner", "room", &second.event_id)
        .unwrap()
        .unwrap();
    assert_eq!(document.text.chars().count(), 64 * 1024);
    assert!(document.truncated);
    f.store
        .connection
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM public_history WHERE event_ref=?1",
            [&first.event_id],
        )
        .unwrap();
    // Removing the message row withholds every delta; raw text never returns.
    assert!(f
        .store
        .read_public_history_locked("owner", "room", &second.event_id)
        .unwrap()
        .is_none());
    drop(_guard);
    let exact = append("界".repeat(64 * 1024));
    append("additional tail".into());
    let _guard = f.store.lock_public_history().unwrap();
    assert!(
        f.store
            .read_public_history_locked("owner", "room", &exact.event_id)
            .unwrap()
            .unwrap()
            .truncated,
        "a full buffer must report an omitted later delta"
    );
}

#[test]
fn public_history_search_matches_streamed_message_once() {
    // MP-08 / MP-10 / MP-11: FTS indexes the message, not each provider delta.
    let f = Fixture::new();
    let append = |key: &str, text: &str| {
        f.store
            .append_transcript(
                &SessionHistoryEntry::provider_output(
                    "room",
                    "run",
                    Some("peer"),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some(key.into()),
                    text.to_owned(),
                ),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    provider_run_id: Some("run".into()),
                    prompt_id: Some("prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let first = append("message", "The stale ");
    for delta in [
        "cur",
        "sor",
        " ",
        "must restart",
        " the query. Restart",
        " now.",
    ] {
        append("message", delta);
    }
    append("other-message", "unrelated restart");
    for query in ["stale cursor", "cursor", "the query now"] {
        let page = f.search("room", query, 50, None).unwrap();
        assert_eq!(page.hits.len(), 1, "{query}");
        assert_eq!(page.hits[0].event_ref, first.event_id);
    }
    let page = f.search("room", "restart", 50, None).unwrap();
    assert_eq!(page.hits.len(), 2);
    assert!(page.coverage.complete);
    assert_eq!(page.coverage.excluded_events, 0);
    let _guard = f.store.lock_public_history().unwrap();
    assert_eq!(
        f.store
            .read_public_history_locked("owner", "room", &first.event_id)
            .unwrap()
            .unwrap()
            .text,
        "The stale cursor must restart the query. Restart now."
    );
}

#[test]
fn public_history_stream_growth_invalidates_offset_cursor() {
    // MP-08 / MP-10 / MP-11: an existing row gaining a search term can
    // otherwise shift OFFSET pages below an unchanged sequence ceiling.
    let f = Fixture::new();
    f.append("room", "compiler oldest");
    let append = |text: &str| {
        f.store
            .append_transcript(
                &SessionHistoryEntry::provider_output(
                    "room",
                    "run",
                    Some("peer"),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some("message".into()),
                    text.to_owned(),
                ),
                HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    provider_run_id: Some("run".into()),
                    prompt_id: Some("prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let growing = append("streaming ");
    f.append("room", "compiler newest");
    let first = f.search("room", "compiler", 1, None).unwrap();
    append("compiler answer");
    assert!(f
        .search("room", "compiler", 1, first.next_cursor.as_deref())
        .is_err());
    assert!(f
        .search("room", "compiler", 50, None)
        .unwrap()
        .hits
        .iter()
        .any(|hit| hit.event_ref == growing.event_id));
}

#[test]
fn public_history_invalidation_survives_busy_wal_reader() {
    // MP-08 / MP-10 / MP-11: an active reader can delay WAL truncation, but the
    // committed secure delete must not fail the protected mutation.
    let f = Fixture::new();
    let event = f.append("room", "busy_reader_marker");
    let reader = Connection::open(f.root.join("history.sqlite")).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM history_events", [], |r| r.get(0))
        .unwrap();
    let _guard = f.store.lock_public_history().unwrap();
    f.store
        .invalidate_public_history_locked(Some("room"))
        .unwrap();
    assert!(f
        .store
        .read_public_history_locked("owner", "room", &event.event_id)
        .unwrap()
        .is_none());
    drop(_guard);
    reader.execute_batch("COMMIT").unwrap();
}

#[test]
fn public_history_batch_projects_and_updates_stream_once() {
    // MP-08 / MP-10 / MP-11: one streamed key in a writer batch does one projection/FTS update.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new();
    let first = f
        .store
        .append_transcript(
            &SessionHistoryEntry::provider_output(
                "room",
                "run",
                Some("peer"),
                crate::terminal::TerminalOutputKind::ProviderOutput,
                Some("message".into()),
                "prefix ".to_owned(),
            ),
            HistoryEventTurnContext {
                public_history_owner_user_id: Some("owner".into()),
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                provider_run_id: Some("run".into()),
                prompt_id: Some("prompt".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let project = f
        .store
        .public_history_projector
        .0
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    f.store.set_public_history_projector(Arc::new(move |event| {
        counted.fetch_add(1, Ordering::Relaxed);
        project(event)
    }));
    let before: u64 = {
        let c = f.store.connection.lock().unwrap();
        c.execute_batch(
            "CREATE TABLE am9_update_count(n); INSERT INTO am9_update_count VALUES(0);
            CREATE TRIGGER am9_count_update AFTER UPDATE OF text ON public_history BEGIN
                UPDATE am9_update_count SET n=n+1; END;",
        )
        .unwrap();
        public_revision(&c, "room").unwrap()
    };
    let events: Vec<_> = (0..64)
        .map(|i| {
            let mut event = first.clone();
            event.event_id = format!("batch-{i}");
            event.sequence = first.sequence + 1 + i;
            event.content = Some("delta ".into());
            event
        })
        .collect();
    f.store.append_many(&events).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1, "batch must project once");
    let c = f.store.connection.lock().unwrap();
    assert_eq!(
        c.query_row("SELECT n FROM am9_update_count", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(public_revision(&c, "room").unwrap() - before, 1);
    drop(c);
    let guard = f.store.lock_public_history().unwrap();
    let doc = f
        .store
        .read_public_history_locked("owner", "room", &events[63].event_id)
        .unwrap()
        .unwrap();
    assert_eq!(doc.text, format!("prefix {}", "delta ".repeat(64)));
    drop(guard);
}

#[test]
fn public_history_nonmatching_stream_growth_still_restarts_cursor() {
    // MP-08 / MP-10 / MP-11: document the conservative Room-wide revision fence.
    let f = Fixture::new();
    f.append("room", "compiler oldest");
    let first = f
        .store
        .append_operational_event(
            HistoryEventKind::ProviderOutput,
            None,
            Some("unrelated streamed review ".into()),
            BTreeMap::from([("merge_key".into(), serde_json::json!("message"))]),
            HistoryEventTurnContext {
                public_history_owner_user_id: Some("owner".into()),
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                provider_run_id: Some("run".into()),
                prompt_id: Some("prompt".into()),
                ..Default::default()
            },
        )
        .unwrap();
    f.append("room", "compiler newest");
    let page = f.search("room", "compiler", 1, None).unwrap();
    assert!(page.next_cursor.is_some());
    let mut delta = first.clone();
    delta.event_id = "unrelated-growth".into();
    delta.sequence = f.store.reserve_sequence();
    delta.content = Some("more public prose".into());
    f.store.append(&delta).unwrap();
    assert!(f
        .search("room", "compiler", 1, page.next_cursor.as_deref())
        .is_err());
    assert_eq!(
        f.search("room", "compiler", 50, None).unwrap().hits.len(),
        2
    );
}
