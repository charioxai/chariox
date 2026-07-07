use super::*;
use crate::local::{ExternalProviderSessionCapabilities, ExternalProviderSessionRecord};

#[test]
fn list_sorts_filters_and_paginates_external_provider_sessions() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 20));
    store.upsert(record("opencode", "session-1", 30));
    store.upsert(record("codex", "thread-2", 10));

    let first = store.list(&ListExternalProviderSessionsRequest {
        provider: None,
        cursor: None,
        limit: Some(2),
    });
    assert_eq!(
        first
            .sessions
            .iter()
            .map(|session| session.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["opencode:session-1", "codex:thread-1"]
    );
    assert!(first.has_more);
    assert_eq!(first.next_cursor.as_deref(), Some("offset:2"));

    let second = store.list(&ListExternalProviderSessionsRequest {
        provider: None,
        cursor: first.next_cursor,
        limit: Some(2),
    });
    assert_eq!(second.sessions[0].external_session_id, "codex:thread-2");
    assert!(!second.has_more);

    let codex = store.list(&ListExternalProviderSessionsRequest {
        provider: Some(" Codex ".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(codex.sessions.len(), 2);
}

#[test]
fn replace_provider_sessions_normalizes_provider_key() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-stale", 20));

    store.replace_provider_sessions(" CODEX ", vec![record("codex", "thread-fresh", 40)]);

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(
        page.sessions
            .iter()
            .map(|session| session.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex:thread-fresh"]
    );
    assert!(store.get("codex:thread-stale").is_none());
}

#[test]
fn upsert_canonicalizes_known_provider_session_records() {
    let store = ExternalProviderSessionIndexStore::default();

    store.upsert(record(" CODEX ", " thread-1 ", 20));

    assert!(store.get(" CODEX : thread-1 ").is_some());
    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some(" codex ".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
    assert_eq!(page.sessions[0].provider, "codex");
    assert_eq!(page.sessions[0].provider_session_id, "thread-1");
}

#[test]
fn attached_markers_match_canonicalized_provider_session_records() {
    let store = ExternalProviderSessionIndexStore::default();
    assert!(
        store
            .mark_attached(" CODEX : thread-1 ", "session-1", "agent-1")
            .is_none()
    );

    store.upsert(record("Codex", " thread-1 ", 20));

    let session = store
        .get("codex:thread-1")
        .expect("session should be indexed with canonical id");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.first_attached_session_id(), Some("session-1"));
    assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
    assert!(
        store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty(),
        "canonicalized attached provider session should not appear as attachable"
    );
}

#[test]
fn replace_provider_sessions_ignores_unknown_provider_key() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 20));

    store.replace_provider_sessions("unknown", vec![record("codex", "thread-2", 40)]);

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
}

#[test]
fn replace_provider_sessions_preserves_attachment_markers() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 20));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");

    store.replace_provider_sessions("codex", vec![record("codex", "thread-1", 40)]);

    let session = store
        .get("codex:thread-1")
        .expect("session should remain indexed");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.first_attached_session_id(), Some("session-1"));
    assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
    assert_eq!(session.last_modified_at_ms, 40);
}

#[test]
fn attachment_marker_applies_to_later_discovered_provider_session() {
    let store = ExternalProviderSessionIndexStore::default();
    assert!(
        store
            .mark_provider_session_attached("codex", "thread-1", "session-1", "agent-1")
            .is_none()
    );

    store.replace_provider_sessions("codex", vec![record("codex", "thread-1", 40)]);

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert!(page.sessions.is_empty());
    let session = store
        .get("codex:thread-1")
        .expect("session should be indexed");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.first_attached_session_id(), Some("session-1"));
    assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
}

#[test]
fn attachment_marker_can_be_applied_from_external_import_metadata() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 40));
    let import =
        ExternalProviderImportMetadata::observed_history("codex:thread-1", "codex", "thread-1");

    let attached = store
        .mark_import_attached(&import, "session-1", "agent-1")
        .expect("provider session should be indexed");

    assert!(attached.is_attached_to_arroba());
    assert_eq!(attached.first_attached_session_id(), Some("session-1"));
    assert_eq!(attached.first_attached_agent_id(), Some("agent-1"));
}

#[test]
fn attachment_marker_can_be_applied_from_provider_resume_state() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 40));
    store.upsert(record("claude", "thread-2", 30));
    store.upsert(record("opencode", "thread-3", 20));
    let mut resume_state = ProviderResumeState::from_codex_thread_id("thread-1");
    resume_state.set_claude_session_id("thread-2");
    resume_state.set_opencode_session_id("thread-3");

    let attached_count = store.mark_resume_state_attached(&resume_state, "session-1", "agent-1");

    assert_eq!(attached_count, 3);
    for external_session_id in ["codex:thread-1", "claude:thread-2", "opencode:thread-3"] {
        let session = store
            .get(external_session_id)
            .expect("provider session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.first_attached_session_id(), Some("session-1"));
        assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
    }
    assert!(
        store
            .list(&ListExternalProviderSessionsRequest {
                provider: None,
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty(),
        "resume-state attached provider sessions should not be attachable"
    );
}

#[test]
fn provider_run_attachment_marks_resume_state_and_direct_provider_session() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-from-resume", 40));
    store.upsert(record("opencode", "session-from-run", 30));
    let resume_state = ProviderResumeState::from_codex_thread_id("thread-from-resume");

    store.mark_provider_run_attached(
        "opencode",
        Some("session-from-run"),
        &resume_state,
        "session-1",
        "agent-1",
    );

    for external_session_id in ["codex:thread-from-resume", "opencode:session-from-run"] {
        let session = store
            .get(external_session_id)
            .expect("provider session should remain indexed");
        assert!(session.is_attached_to_arroba());
        assert_eq!(session.first_attached_session_id(), Some("session-1"));
        assert_eq!(session.first_attached_agent_id(), Some("agent-1"));
    }
}

#[test]
fn external_session_id_for_provider_session_canonicalizes_known_providers() {
    assert_eq!(
        external_session_id_for_provider_session(" Codex ", " thread-1 ").as_deref(),
        Some("codex:thread-1")
    );
    assert_eq!(
        external_session_id_for_provider_session("unknown", "thread-1"),
        None
    );
    assert_eq!(
        external_session_id_for_provider_session("codex", "   "),
        None
    );
}

#[test]
fn list_excludes_attached_to_arroba_external_provider_sessions() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 30));
    store.upsert(record("codex", "thread-2", 20));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });

    assert_eq!(
        page.sessions
            .iter()
            .map(|session| session.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex:thread-2"]
    );
}

#[test]
fn detach_session_returns_provider_session_to_attachable_list() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 30));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");

    assert!(
        store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty()
    );

    store.detach_session("session-1");

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
    assert!(page.sessions[0].is_attachable_to_arroba());
    assert_eq!(page.sessions[0].first_attached_session_id(), None);
    assert_eq!(page.sessions[0].first_attached_agent_id(), None);
}

#[test]
fn detach_session_preserves_other_session_attachment_agents() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 30));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");
    store.mark_attached("codex:thread-1", "session-2", "agent-2");

    store.detach_session("session-1");

    let session = store
        .get("codex:thread-1")
        .expect("session should remain indexed");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.attached_session_ids, vec!["session-2"]);
    assert_eq!(session.attached_agent_ids, vec!["agent-2"]);
    assert!(
        store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty()
    );
}

#[test]
fn detach_agent_returns_provider_session_to_attachable_list() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 30));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");

    store.detach_agent("session-1", "agent-1");

    let page = store.list(&ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].external_session_id, "codex:thread-1");
    assert!(page.sessions[0].is_attachable_to_arroba());
    assert_eq!(page.sessions[0].first_attached_session_id(), None);
    assert_eq!(page.sessions[0].first_attached_agent_id(), None);
}

#[test]
fn detach_attachment_removes_only_exact_provider_session_agent_ref() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-old", 30));
    store.upsert(record("codex", "thread-new", 40));
    store.mark_attached("codex:thread-old", "session-1", "agent-1");
    store.mark_attached("codex:thread-new", "session-1", "agent-1");

    assert!(store.detach_attachment("codex:thread-old", "session-1", "agent-1"));

    let old = store
        .get("codex:thread-old")
        .expect("old provider session should remain indexed");
    assert!(old.is_attachable_to_arroba());
    let new = store
        .get("codex:thread-new")
        .expect("new provider session should remain indexed");
    assert!(new.is_attached_to_arroba());
    assert_eq!(new.first_attached_session_id(), Some("session-1"));
    assert_eq!(new.first_attached_agent_id(), Some("agent-1"));
    assert_eq!(
        store.attachment_refs(),
        BTreeSet::from([ExternalProviderSessionAttachmentRef {
            external_session_id: "codex:thread-new".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
        }])
    );
}

#[test]
fn detach_agent_preserves_other_agent_attachments() {
    let store = ExternalProviderSessionIndexStore::default();
    store.upsert(record("codex", "thread-1", 30));
    store.mark_attached("codex:thread-1", "session-1", "agent-1");
    store.mark_attached("codex:thread-1", "session-1", "agent-2");
    store.mark_attached("codex:thread-1", "session-2", "agent-3");

    store.detach_agent("session-1", "agent-1");

    let session = store
        .get("codex:thread-1")
        .expect("session should remain indexed");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.attached_session_ids, vec!["session-1", "session-2"]);
    assert_eq!(session.attached_agent_ids, vec!["agent-2", "agent-3"]);
    assert!(
        store
            .list(&ListExternalProviderSessionsRequest {
                provider: Some("codex".to_string()),
                cursor: None,
                limit: None,
            })
            .sessions
            .is_empty()
    );

    store.detach_agent("session-1", "agent-2");
    let session = store
        .get("codex:thread-1")
        .expect("session should remain indexed");
    assert!(session.is_attached_to_arroba());
    assert_eq!(session.attached_session_ids, vec!["session-2"]);
    assert_eq!(session.attached_agent_ids, vec!["agent-3"]);
}

#[test]
fn transcript_cursor_store_detaches_session_cursors() {
    let store = AttachedProviderTranscriptCursorStore::default();
    store.set(
        AttachedProviderTranscriptCursorKey::new("session-1", "agent-1", "codex", "thread-1"),
        ExternalProviderObservedCursor {
            last_observed_turn_id: Some("turn-1".to_string()),
            ..ExternalProviderObservedCursor::default()
        },
    );
    let preserved_key =
        AttachedProviderTranscriptCursorKey::new("session-2", "agent-2", "codex", "thread-2");
    store.set(
        preserved_key.clone(),
        ExternalProviderObservedCursor {
            last_observed_turn_id: Some("turn-2".to_string()),
            ..ExternalProviderObservedCursor::default()
        },
    );

    assert_eq!(store.detach_session("session-1"), 1);

    assert_eq!(
        store.get(&AttachedProviderTranscriptCursorKey::new(
            "session-1",
            "agent-1",
            "codex",
            "thread-1"
        )),
        ExternalProviderObservedCursor::default()
    );
    assert_eq!(
        store.get(&preserved_key).last_observed_turn_id.as_deref(),
        Some("turn-2")
    );
}

#[test]
fn transcript_cursor_store_detaches_agent_cursors() {
    let store = AttachedProviderTranscriptCursorStore::default();
    store.set(
        AttachedProviderTranscriptCursorKey::new("session-1", "agent-1", "codex", "thread-1"),
        ExternalProviderObservedCursor {
            last_observed_turn_id: Some("turn-1".to_string()),
            ..ExternalProviderObservedCursor::default()
        },
    );
    let preserved_same_session =
        AttachedProviderTranscriptCursorKey::new("session-1", "agent-2", "codex", "thread-2");
    store.set(
        preserved_same_session.clone(),
        ExternalProviderObservedCursor {
            last_observed_turn_id: Some("turn-2".to_string()),
            ..ExternalProviderObservedCursor::default()
        },
    );

    assert_eq!(store.detach_agent("session-1", "agent-1"), 1);

    assert_eq!(
        store.get(&AttachedProviderTranscriptCursorKey::new(
            "session-1",
            "agent-1",
            "codex",
            "thread-1"
        )),
        ExternalProviderObservedCursor::default()
    );
    assert_eq!(
        store
            .get(&preserved_same_session)
            .last_observed_turn_id
            .as_deref(),
        Some("turn-2")
    );
}

#[test]
fn transcript_cursor_key_canonicalizes_provider_session_identity() {
    let store = AttachedProviderTranscriptCursorStore::default();
    store.set(
        AttachedProviderTranscriptCursorKey::new("session-1", "agent-1", " Codex ", " thread-1 "),
        ExternalProviderObservedCursor {
            last_observed_turn_id: Some("turn-1".to_string()),
            ..ExternalProviderObservedCursor::default()
        },
    );

    assert_eq!(
        store
            .get(&AttachedProviderTranscriptCursorKey::new(
                "session-1",
                "agent-1",
                "codex",
                "thread-1"
            ))
            .last_observed_turn_id
            .as_deref(),
        Some("turn-1")
    );
}

fn record(
    provider: &str,
    provider_session_id: &str,
    last_modified_at_ms: u64,
) -> ExternalProviderSessionRecord {
    ExternalProviderSessionRecord {
        external_session_id: format!("{provider}:{provider_session_id}"),
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        title: Some(provider_session_id.to_string()),
        title_source: Some("test".to_string()),
        first_prompt_preview: None,
        created_at_ms: None,
        last_modified_at_ms,
        worktree_path: None,
        account_profile: None,
        capabilities: ExternalProviderSessionCapabilities {
            ..ExternalProviderSessionCapabilities::default()
        },
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}
