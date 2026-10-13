use super::*;

#[tokio::test]
async fn initial_outline_batches_three_agents_and_marks_budgeted_steers() {
    // MP-08 / MP-10: source regression only; live Web/TUI acceptance is separate.
    let path = std::env::temp_dir().join(format!(
        "chariox-initial-outline-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let store = OperationalHistoryStore::open(path.clone()).unwrap();
    for (index, agent) in ["a", "b", "c"].iter().enumerate() {
        let start = index as u64 * 100 + 1;
        let context = HistoryEventTurnContext {
            session_id: Some("s".into()),
            agent_id: Some((*agent).into()),
            turn_id: Some(format!("turn-{agent}")),
            prompt_id: Some(format!("prompt-{agent}")),
            provider_run_id: Some(format!("run-{agent}")),
            ..HistoryEventTurnContext::default()
        };
        store
            .append(&HistoryEvent::transcript(
                start,
                &SessionHistoryEntry::user_prompt("s", "client", agent, "original task"),
                context.clone(),
            ))
            .unwrap();
        for offset in 1..=40 {
            store
                .append(&HistoryEvent::transcript(
                    start + offset + 2,
                    &SessionHistoryEntry::provider_output(
                        "s",
                        &format!("run-{agent}"),
                        Some(agent),
                        TerminalOutputKind::ProviderOutput,
                        Some(format!("message-{offset}")),
                        format!("step {offset}"),
                    ),
                    context.clone(),
                ))
                .unwrap();
        }
        store
            .append(&HistoryEvent::operational(
                start + 1,
                HistoryEventKind::UserPrompt,
                Some(crate::history::HistoryEventRole::User),
                Some("also run date".into()),
                BTreeMap::from([(
                    "merge_key".into(),
                    serde_json::Value::String(crate::history::steering_prompt_merge_key(&format!(
                        "queued-{agent}"
                    ))),
                )]),
                context.clone(),
            ))
            .unwrap();
        store.append(&HistoryEvent::transcript(start + 43, &SessionHistoryEntry::provider_output("s", &format!("run-{agent}"), Some(agent), TerminalOutputKind::ProviderTool, Some(format!("call-{agent}")), format!(r#"{{"id":"call-{agent}","tool":"read","status":"completed","output":"source-{agent}"}}"#)), context)).unwrap();
    }
    let request = GetSessionHistoryOutlineRequest {
        session_id: "s".into(),
        agent_ids: Some(vec!["a".into(), "b".into(), "c".into()]),
        latest_prompt_count: Some(4),
        cursor: None,
        include_prefetched_content: true,
    };
    let response = execute_session_history_outline_request(store.clone(), request)
        .await
        .unwrap();
    let LocalDaemonResponse::SessionHistoryOutline { agents } = response else {
        panic!("wrong response")
    };
    assert_eq!(agents.len(), 3);
    for agent in agents {
        assert_eq!(agent.turns.len(), 1);
        let blobs = &agent.turns[0].blobs;
        let steer = blobs
            .iter()
            .find(|blob| blob.steering)
            .expect("budgeted steer marker");
        let pages = agent.prefetched_blobs.as_ref().unwrap();
        assert!(pages
            .iter()
            .any(|page| page.blob_id == steer.blob_id
                && page.entries[0].entry.text == "also run date"));
        assert!(pages
            .iter()
            .flat_map(|page| &page.entries)
            .any(|entry| entry
                .entry
                .text
                .contains(&format!("source-{}", agent.agent_id))));
    }
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

// MP-08 / MP-10 (panes R7, protocol 508): one request returns several blobs of
// an agent, in request order, and refuses oversized batches.
#[tokio::test]
async fn blob_contents_batch_returns_each_requested_blob_in_order() {
    let path = std::env::temp_dir().join(format!(
        "chariox-blob-contents-batch-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let store =
        OperationalHistoryStore::open(path.clone()).expect("operational history store should open");
    let context = HistoryEventTurnContext {
        session_id: Some("session-1".to_string()),
        agent_id: Some("agent-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        provider_run_id: Some("run-1".to_string()),
        ..HistoryEventTurnContext::default()
    };
    for (sequence, call) in [(20, "call-1"), (21, "call-2")] {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::provider_output(
                    "session-1",
                    "run-1",
                    Some("agent-1"),
                    TerminalOutputKind::ProviderTool,
                    Some(call.to_string()),
                    format!(r#"{{"id":"{call}","tool":"bash","status":"completed"}}"#),
                ),
                context.clone(),
            ))
            .expect("tool row should append");
    }

    let response = execute_scoped_session_history_blob_contents_request(
        store.clone(),
        GetSessionHistoryBlobContentsRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            blob_ids: vec!["history:21:21".to_string(), "history:20:20".to_string()],
        },
        None,
    )
    .await
    .expect("batch should load");
    let LocalDaemonResponse::SessionHistoryBlobContents { blobs } = response else {
        panic!("unexpected response");
    };
    assert_eq!(
        blobs
            .iter()
            .map(|blob| (blob.blob_id.as_str(), blob.entries.len()))
            .collect::<Vec<_>>(),
        vec![("history:21:21", 1), ("history:20:20", 1)]
    );
    assert!(blobs[0].entries[0].entry.text.contains("call-2"));

    let oversized = execute_scoped_session_history_blob_contents_request(
        store,
        GetSessionHistoryBlobContentsRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            blob_ids: vec!["history:20:20".to_string(); MAX_BLOB_CONTENTS_PER_REQUEST + 1],
        },
        None,
    )
    .await;
    assert!(oversized.is_err());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[tokio::test]
async fn initial_outline_compacts_word_delta_metadata_before_relay_delivery() {
    // MP-08 / MP-10: supplementary regression for the real WAN oversized response.
    let path = std::env::temp_dir().join(format!(
        "chariox-initial-compact-{}-{}.db",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let store = OperationalHistoryStore::open(path.clone()).unwrap();
    let context = HistoryEventTurnContext {
        session_id: Some("s".into()),
        agent_id: Some("a".into()),
        turn_id: Some("t".into()),
        prompt_id: Some("p".into()),
        provider_run_id: Some("r".into()),
        ..HistoryEventTurnContext::default()
    };
    store
        .append(&HistoryEvent::transcript(
            1,
            &SessionHistoryEntry::user_prompt("s", "c", "a", "review the source"),
            context.clone(),
        ))
        .unwrap();
    for sequence in 2..=513 {
        store
            .append(&HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::provider_output(
                    "s",
                    "r",
                    Some("a"),
                    TerminalOutputKind::ProviderReasoning,
                    Some("thought".into()),
                    "é中".to_string(),
                ),
                context.clone(),
            ))
            .unwrap();
    }
    let result = execute_session_history_outline_request(
        store.clone(),
        GetSessionHistoryOutlineRequest {
            session_id: "s".into(),
            agent_ids: Some(vec!["a".into()]),
            latest_prompt_count: Some(4),
            cursor: None,
            include_prefetched_content: true,
        },
    )
    .await
    .unwrap();
    let LocalDaemonResponse::SessionHistoryOutline { agents } = result else {
        panic!("outline")
    };
    let pages = agents[0].prefetched_blobs.as_ref().unwrap();
    assert!(
        serde_json::to_vec(pages).unwrap().len() < 12_000,
        "MP-08 / MP-10: closed thought deltas repeat metadata on the WAN wire"
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn complete_inline_delta_groups_keep_each_original_sequence() {
    // MP-08 / MP-10: grouped inline words must deduplicate the same rows as blobs.
    let events = [(10, "é"), (12, "中"), (13, "é")]
        .into_iter()
        .map(|(sequence, text)| {
            HistoryEvent::transcript(
                sequence,
                &SessionHistoryEntry::provider_output(
                    "s",
                    "r",
                    Some("a"),
                    TerminalOutputKind::ProviderOutput,
                    Some("m".into()),
                    text.to_string(),
                ),
                HistoryEventTurnContext::default(),
            )
        })
        .collect::<Vec<_>>();
    let page = outline_page_entry_from_event_group(&events.iter().collect::<Vec<_>>()).unwrap();
    assert_eq!(page.sequence_indices, vec![10, 12, 13]);
    assert_eq!(page.sequence_byte_lengths, vec![2, 3, 2]);
    assert_eq!(page.entry.text, "é中é");
}
