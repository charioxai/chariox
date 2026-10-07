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
        "748f93a9b607e3cae05d4e5a566df6612a51cef874e7f28297a367512e599962"
    );
}
