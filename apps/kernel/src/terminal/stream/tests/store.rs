use super::*;

#[tokio::test]
async fn terminal_stream_store_notifies_waiters_on_output() {
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.attachment_change_sequence("session-1", "attachment-1");
    let waiter = {
        let terminal = terminal.clone();
        tokio::spawn(async move {
            terminal
                .wait_for_attachment_change_after("session-1", "attachment-1", sequence)
                .await;
        })
    };

    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        None,
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"output",
    );

    tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
        .await
        .expect("terminal stream waiter should wake")
        .expect("terminal stream waiter task should complete");
    assert!(terminal.attachment_change_sequence("session-1", "attachment-1") > sequence);
}

#[tokio::test]
async fn terminal_stream_store_does_not_wake_unrelated_attachment_waiters() {
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.attachment_change_sequence("session-1", "attachment-2");

    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        None,
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"output",
    );

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            terminal.wait_for_attachment_change_after("session-1", "attachment-2", sequence),
        )
        .await
        .is_err(),
        "unrelated attachment waiter should not wake"
    );
}

#[tokio::test]
async fn terminal_stream_store_external_observed_output_does_not_wake_unrelated_attachment_waiters()
{
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.attachment_change_sequence("session-1", "attachment-2");
    let state_entry = crate::history::SessionHistoryEntry::external_provider_observed_state_signal(
        "session-1",
        Some("provider-run-1"),
        "agent-1",
        "codex",
        "thread-1",
        crate::history::EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON,
        "external:codex:thread-1:done",
        "active_prompt_settled".to_string(),
        Some(1_234),
    );
    let metadata =
        TerminalOutputExternalObservationMetadata::from_session_history_entry(&state_entry)
            .expect("external state history entry should produce terminal metadata");

    terminal.fan_out_external_observed_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderStatus,
        state_entry.merge_key.clone(),
        vec!["attachment-1".to_string()],
        crate::history::EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes(),
        metadata,
        state_entry.source_attachment_id.clone(),
    );

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            terminal.wait_for_attachment_change_after("session-1", "attachment-2", sequence),
        )
        .await
        .is_err(),
        "external observed output for one attachment should not wake unrelated attachment waiters"
    );
}

#[tokio::test]
async fn terminal_stream_store_broadcast_notice_wakes_session_waiters() {
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.session_change_sequence("session-1");
    let waiter = {
        let terminal = terminal.clone();
        tokio::spawn(async move {
            terminal
                .wait_for_session_change_after("session-1", sequence)
                .await;
        })
    };

    terminal.record_notice("session-1", None, None, Vec::new(), "broadcast notice");

    tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
        .await
        .expect("terminal session waiter should wake")
        .expect("terminal session waiter task should complete");
    assert!(terminal.session_change_sequence("session-1") > sequence);
}

#[tokio::test]
async fn terminal_stream_store_session_waiters_ignore_unrelated_sessions() {
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.session_change_sequence("session-2");

    terminal.notify_terminal_projection_change("session-1");

    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            terminal.wait_for_session_change_after("session-2", sequence),
        )
        .await
        .is_err(),
        "unrelated session waiter should not wake"
    );
}

#[tokio::test]
async fn terminal_stream_store_batch_fanout_notifies_each_changed_attachment_once() {
    let terminal = TerminalStreamStore::new();
    let changed_sequence = terminal.attachment_change_sequence("session-1", "attachment-1");
    let unrelated_sequence = terminal.attachment_change_sequence("session-1", "attachment-2");
    let waiter = {
        let terminal = terminal.clone();
        tokio::spawn(async move {
            terminal
                .wait_for_attachment_change_after("session-1", "attachment-1", changed_sequence)
                .await;
        })
    };

    let records = terminal.fan_out_outputs(vec![
        TerminalOutputAppend {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("batch-key".to_string()),
            recipient_attachment_ids: Arc::from(vec!["attachment-1".to_string()]),
            bytes: b"one".to_vec(),
        },
        TerminalOutputAppend {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("batch-key".to_string()),
            recipient_attachment_ids: Arc::from(vec!["attachment-1".to_string()]),
            bytes: b"two".to_vec(),
        },
    ]);

    assert_eq!(records.len(), 2);
    tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
        .await
        .expect("batch terminal stream waiter should wake")
        .expect("batch terminal stream waiter task should complete");
    assert_eq!(
        terminal.attachment_change_sequence("session-1", "attachment-1"),
        changed_sequence + 1,
        "batch fanout should notify the changed attachment once per batch"
    );
    assert_eq!(
        terminal.attachment_change_sequence("session-1", "attachment-2"),
        unrelated_sequence,
        "batch fanout should not wake unrelated attachment scopes"
    );
}

#[tokio::test]
async fn terminal_stream_store_batch_fanout_coalesces_repeated_multi_recipient_change_keys() {
    let terminal = TerminalStreamStore::new();
    let first_sequence = terminal.attachment_change_sequence("session-1", "attachment-1");
    let second_sequence = terminal.attachment_change_sequence("session-1", "attachment-2");
    let unrelated_sequence = terminal.attachment_change_sequence("session-1", "attachment-3");

    let records = terminal.fan_out_outputs(vec![
        TerminalOutputAppend {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderTool,
            merge_key: Some("batch-key-1".to_string()),
            recipient_attachment_ids: Arc::from(vec![
                "attachment-1".to_string(),
                "attachment-2".to_string(),
            ]),
            bytes: b"one".to_vec(),
        },
        TerminalOutputAppend {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderTool,
            merge_key: Some("batch-key-2".to_string()),
            recipient_attachment_ids: Arc::from(vec![
                "attachment-1".to_string(),
                "attachment-2".to_string(),
            ]),
            bytes: b"two".to_vec(),
        },
    ]);

    assert_eq!(records.len(), 2);
    assert_eq!(
        terminal.attachment_change_sequence("session-1", "attachment-1"),
        first_sequence + 1,
        "repeated multi-recipient batch fanout should notify first recipient once"
    );
    assert_eq!(
        terminal.attachment_change_sequence("session-1", "attachment-2"),
        second_sequence + 1,
        "repeated multi-recipient batch fanout should notify second recipient once"
    );
    assert_eq!(
        terminal.attachment_change_sequence("session-1", "attachment-3"),
        unrelated_sequence,
        "batch fanout should not wake unrelated recipients"
    );
}

#[tokio::test]
async fn terminal_stream_store_wait_returns_when_sequence_already_changed() {
    let terminal = TerminalStreamStore::new();
    let sequence = terminal.session_change_sequence("session-1");
    terminal.record_notice("session-1", None, None, Vec::new(), "notice");

    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        terminal.wait_for_session_change_after("session-1", sequence),
    )
    .await
    .expect("changed terminal sequence should not block");
}
