use super::*;

// MP-08 / MP-10 (panes Phase-1 drill, PANE-01/PANE-06): OpenCode stores a
// call's "running" and "completed" snapshots as separate rows, and a sentence
// can land between them. The call stays one blob at its first position, and
// its content holds only that call's snapshots.
#[tokio::test]
async fn tool_call_split_by_a_sentence_stays_one_blob() {
    let path = std::env::temp_dir().join(format!(
        "chariox-split-tool-call-outline-{}-{}.db",
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
    let tool = |sequence: u64, call: &str, status: &str| {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderTool,
                Some(call.to_string()),
                format!(
                    r#"{{"id":"{call}","tool":"read","status":"{status}","input":{{"filePath":"README.md"}}}}"#
                ),
            ),
            context.clone(),
        )
    };
    let sentence = |sequence: u64, text: &str| {
        HistoryEvent::transcript(
            sequence,
            &SessionHistoryEntry::provider_output(
                "session-1",
                "run-1",
                Some("agent-1"),
                TerminalOutputKind::ProviderOutput,
                Some(format!("text-{sequence}")),
                text.to_string(),
            ),
            context.clone(),
        )
    };
    for event in [
        HistoryEvent::transcript(
            10,
            &SessionHistoryEntry::user_prompt("session-1", "attachment-1", "agent-1", "prompt"),
            context.clone(),
        ),
        tool(11, "call-1", "running"),
        sentence(12, "Reading the file."),
        tool(13, "call-1", "completed"),
        tool(14, "call-2", "running"),
        tool(15, "call-2", "completed"),
        sentence(16, "Done."),
    ] {
        store.append(&event).expect("event should append");
    }

    let outline =
        load_agent_outline(&store, "session-1", "agent-1", 1, None).expect("outline should load");
    let turn = &outline.turns[0];
    let tool_blobs = turn
        .blobs
        .iter()
        .filter(|blob| blob.kind == SessionHistoryEntryKind::ProviderTool)
        .map(|blob| (blob.sequence_start, blob.entry_count))
        .collect::<Vec<_>>();
    assert_eq!(tool_blobs, vec![(11, 2), (14, 2)]);
    let call_1 = turn
        .blobs
        .iter()
        .find(|blob| blob.sequence_start == 11)
        .expect("call-1 blob");
    assert!(call_1
        .preview
        .as_deref()
        .is_some_and(|preview| preview.contains("completed")));

    let response = execute_scoped_session_history_blob_contents_request(
        store.clone(),
        GetSessionHistoryBlobContentsRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            blob_ids: vec![call_1.blob_id.clone()],
        },
        None,
    )
    .await
    .expect("blob should load");
    let LocalDaemonResponse::SessionHistoryBlobContents { blobs } = response else {
        panic!("unexpected response");
    };
    assert_eq!(
        blobs[0]
            .entries
            .iter()
            .map(|entry| entry.entry_index)
            .collect::<Vec<_>>(),
        vec![11, 13]
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
