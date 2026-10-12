use super::*;

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
