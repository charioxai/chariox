use super::*;

#[test]
fn records_terminal_input_and_fans_out_output() {
    let mut terminal = TerminalStreamService::new();

    terminal.record_input("session-1", "provider-run-1", "attachment-1", b"ls\n");
    let output = terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("part-1".to_string()),
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"listing\n",
    );
    let notice = terminal.record_notice(
        "session-1",
        Some("provider-run-1"),
        Some("agent-1"),
        vec!["attachment-2".to_string()],
        "provider switch failed; resumed previous run",
    );

    assert_eq!(terminal.input_records().len(), 1);
    assert_eq!(terminal.output_records().len(), 1);
    assert_eq!(terminal.notice_records().len(), 1);
    assert_eq!(output.kind, TerminalOutputKind::ProviderOutput);
    assert_eq!(output.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(output.merge_key.as_deref(), Some("part-1"));
    assert_eq!(output.recipient_attachment_ids.len(), 2);
    assert_eq!(output.pending_recipient_attachment_ids.len(), 2);
    assert_eq!(notice.provider_run_id.as_deref(), Some("provider-run-1"));
    assert_eq!(notice.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(notice.recipient_attachment_ids.len(), 1);
    assert_eq!(notice.pending_recipient_attachment_ids.len(), 1);
}

#[test]
fn output_polling_is_per_recipient() {
    let mut terminal = TerminalStreamService::new();
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::PromptEcho,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"hello\n",
    );

    let first = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].record_id, Some(0));
    assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);
    assert_eq!(
        first[0].pending_recipient_attachment_ids,
        vec!["attachment-1"]
    );
    assert_eq!(terminal.output_records().len(), 1);
    assert_eq!(
        terminal.output_records()[0].pending_recipient_attachment_ids,
        vec!["attachment-2".to_string()]
    );
    assert_eq!(terminal.output_records()[0].record_id, Some(0));

    let second = terminal.drain_output_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].record_id, first[0].record_id);
    assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);
    assert_eq!(
        second[0].pending_recipient_attachment_ids,
        vec!["attachment-2"]
    );
    assert!(terminal.output_records().is_empty());
}

#[test]
fn external_observed_output_records_carry_metadata() {
    let mut terminal = TerminalStreamService::new();
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

    let drained = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(
        drained[0].source_attachment_id.as_deref(),
        Some("external:codex")
    );
    let metadata = drained[0]
        .external_observation_metadata
        .as_ref()
        .expect("external observed metadata should survive drain");
    assert_eq!(
        metadata.source,
        SessionHistoryEntrySource::ExternalProviderObserved
    );
    assert_eq!(metadata.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        metadata.external_provider_session_id.as_deref(),
        Some("thread-1")
    );
    assert_eq!(
        metadata.external_provider_turn_id.as_deref(),
        Some("active_prompt_settled")
    );
    assert_eq!(metadata.observed_at_ms, Some(1_234));
    assert_eq!(
        metadata.external_observation,
        Some(SessionHistoryExternalObservation::active_prompt_settled())
    );
}

#[test]
fn external_observed_output_replaces_pending_record_with_same_merge_key() {
    let mut terminal = TerminalStreamService::new();
    let (merge_key, metadata, source_attachment_id) = external_observed_metadata("assistant-1");

    terminal.fan_out_external_observed_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some(merge_key.clone()),
        vec!["attachment-1".to_string()],
        b"first version",
        metadata.clone(),
        source_attachment_id.clone(),
    );
    terminal.fan_out_external_observed_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some(merge_key),
        vec!["attachment-1".to_string()],
        b"updated version",
        metadata,
        source_attachment_id,
    );

    assert_eq!(terminal.output_records().len(), 1);
    let drained = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].bytes, b"updated version");
    assert_eq!(drained[0].record_id, Some(0));
}

#[test]
fn external_observed_output_requeues_replacement_for_drained_recipient() {
    let mut terminal = TerminalStreamService::new();
    let (merge_key, metadata, source_attachment_id) = external_observed_metadata("assistant-1");

    terminal.fan_out_external_observed_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some(merge_key.clone()),
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"first version",
        metadata.clone(),
        source_attachment_id.clone(),
    );
    let first = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].bytes, b"first version");

    terminal.fan_out_external_observed_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some(merge_key),
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"updated version",
        metadata,
        source_attachment_id,
    );

    let first_after_update = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(first_after_update.len(), 1);
    assert_eq!(first_after_update[0].record_id, Some(0));
    assert_eq!(first_after_update[0].bytes, b"updated version");
    let second = terminal.drain_output_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].record_id, Some(0));
    assert_eq!(second[0].bytes, b"updated version");
    assert!(terminal.output_records().is_empty());
}

#[test]
fn output_polling_drains_single_recipient_batch_records() {
    let mut terminal = TerminalStreamService::new();
    let records = terminal.fan_out_outputs(vec![
        TerminalOutputAppend {
            session_id: "session-1".to_string(),
            provider_run_id: "provider-run-1".to_string(),
            agent_id: Some("agent-1".to_string()),
            prompt_origin: None,
            source_attachment_id: None,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("chunk-1".to_string()),
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
            merge_key: Some("chunk-2".to_string()),
            recipient_attachment_ids: Arc::from(vec!["attachment-1".to_string()]),
            bytes: b"two".to_vec(),
        },
    ]);
    assert_eq!(records.records.len(), 2);
    assert_eq!(terminal.health_snapshot().pending_output_records, 2);

    let drained = terminal.drain_output_records("session-1", "attachment-1");

    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].recipient_attachment_ids, vec!["attachment-1"]);
    assert_eq!(
        drained[0].pending_recipient_attachment_ids,
        vec!["attachment-1"]
    );
    assert_eq!(drained[1].bytes, b"two");
    assert!(terminal.output_records().is_empty());
    assert_eq!(terminal.health_snapshot().pending_output_records, 0);
}

#[test]
fn output_coalescing_is_scoped_by_prompt_metadata() {
    let mut terminal = TerminalStreamService::with_output_coalesce_byte_limit(1024);
    terminal.fan_out_output_with_prompt_metadata(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("assistant".to_string()),
        Some(crate::session::PromptOrigin::External),
        Some("attachment-1".to_string()),
        vec!["attachment-viewer".to_string()],
        b"first",
    );
    terminal.fan_out_output_with_prompt_metadata(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        Some("assistant".to_string()),
        Some(crate::session::PromptOrigin::External),
        Some("attachment-2".to_string()),
        vec!["attachment-viewer".to_string()],
        b"second",
    );

    let drained = terminal.drain_output_records("session-1", "attachment-viewer");

    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].bytes, b"first");
    assert_eq!(
        drained[0].source_attachment_id.as_deref(),
        Some("attachment-1")
    );
    assert_eq!(drained[1].bytes, b"second");
    assert_eq!(
        drained[1].source_attachment_id.as_deref(),
        Some("attachment-2")
    );
}

#[test]
fn final_recipient_drain_clears_coalescing_anchor() {
    let mut terminal = TerminalStreamService::new();
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"first",
    );

    let first = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert!(terminal.output_records().is_empty());

    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"second",
    );

    assert_eq!(terminal.output_records().len(), 1);
    let second = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].bytes, b"second");
    assert!(terminal.output_records().is_empty());
}

#[test]
fn batch_fanout_deduplicates_repeated_single_recipient_changed_keys() {
    let mut terminal = TerminalStreamService::new();
    let records = terminal.fan_out_outputs(
        (0..64)
            .map(|index| TerminalOutputAppend {
                session_id: "session-1".to_string(),
                provider_run_id: format!("provider-run-{index}"),
                agent_id: Some(format!("agent-{index}")),
                prompt_origin: None,
                source_attachment_id: None,
                kind: TerminalOutputKind::ProviderTool,
                merge_key: Some(format!("chunk-{index}")),
                recipient_attachment_ids: Arc::from(vec!["attachment-1".to_string()]),
                bytes: format!("chunk-{index}").into_bytes(),
            })
            .collect(),
    );

    assert_eq!(records.records.len(), 64);
    assert_eq!(records.changed_keys.len(), 1);
    assert!(records
        .changed_keys
        .contains(&("session-1".to_string(), "attachment-1".to_string())));

    let drained = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(drained.len(), 64);
    assert_eq!(drained[0].bytes, b"chunk-0");
    assert_eq!(drained[63].bytes, b"chunk-63");
    assert!(terminal.output_records().is_empty());
}

#[test]
fn prompt_output_records_carry_prompt_identity() {
    let mut terminal = TerminalStreamService::new();
    let output = terminal.fan_out_prompt_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        "prompt-42",
        Some(crate::session::PromptOrigin::External),
        "attachment-1",
        vec!["attachment-2".to_string()],
        b"hello\n",
    );

    assert_eq!(output.kind, TerminalOutputKind::PromptEcho);
    assert_eq!(output.prompt_id.as_deref(), Some("prompt-42"));
    assert_eq!(
        output.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
    assert_eq!(output.source_attachment_id.as_deref(), Some("attachment-1"));
    let drained = terminal.drain_output_records("session-1", "attachment-2");
    assert_eq!(drained[0].prompt_id.as_deref(), Some("prompt-42"));
    assert_eq!(
        drained[0].source_attachment_id.as_deref(),
        Some("attachment-1")
    );
}

#[test]
fn output_polling_keeps_large_drains_batched() {
    let mut terminal = TerminalStreamService::with_output_drain_json_limit(256);
    for index in 0..4 {
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some(format!("chunk-{index}")),
            vec!["attachment-1".to_string()],
            b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        );
    }

    let first = terminal.drain_output_records("session-1", "attachment-1");
    assert!(!first.is_empty());
    assert!(first.len() < 4);
    assert_eq!(first[0].merge_key.as_deref(), Some("chunk-0"));
    assert!(!terminal.output_records().is_empty());

    let second = terminal.drain_output_records("session-1", "attachment-1");
    assert!(!second.is_empty());
    let expected_next_chunk = format!("chunk-{}", first.len());
    assert_eq!(
        second[0].merge_key.as_deref(),
        Some(expected_next_chunk.as_str())
    );
    assert!(terminal.output_records().len() < 4);
}

#[test]
fn output_drain_size_estimator_bounds_scoped_json() {
    let record = TerminalOutputRecord {
        record_id: None,
        timestamp_ms: 1_700,
        session_id: "session-\n1".to_string(),
        provider_run_id: "provider-run-1".to_string(),
        agent_id: Some("agent-\"1\"".to_string()),
        prompt_id: Some("prompt-1".to_string()),
        prompt_origin: None,
        source_attachment_id: Some("attachment-source".to_string()),
        kind: TerminalOutputKind::ProviderReasoning,
        merge_key: Some("merge\\key".to_string()),
        recipient_attachment_ids: vec!["attachment-1".to_string(), "attachment-2".to_string()],
        pending_recipient_attachment_ids: vec![
            "attachment-1".to_string(),
            "attachment-2".to_string(),
        ],
        bytes: vec![0, 9, 10, 99, 100, 255],
        external_observation_metadata: None,
    };

    let scoped = scoped_output_record(&record, 7, "attachment-2");
    let actual_len = serde_json::to_vec(&scoped)
        .expect("scoped terminal output should serialize")
        .len();
    let estimated_len = terminal_output_record_scoped_json_bytes(&record, "attachment-2");

    assert!(
        estimated_len >= actual_len,
        "estimated scoped JSON length {estimated_len} should bound actual length {actual_len}"
    );
}

#[test]
fn output_backlog_is_bounded_per_slow_recipient() {
    let mut terminal = TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
    for index in 0..4 {
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some(format!("chunk-{index}")),
            vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
            format!("chunk-{index}").as_bytes(),
        );
    }

    let slow_records = terminal.drain_output_records("session-1", "slow-attachment");
    assert_eq!(slow_records.len(), 2);
    assert_eq!(slow_records[0].bytes, b"chunk-2");
    assert_eq!(slow_records[1].bytes, b"chunk-3");

    let fast_records = terminal.drain_output_records("session-1", "fast-attachment");
    assert_eq!(fast_records.len(), 2);
    assert_eq!(fast_records[0].bytes, b"chunk-2");
    assert_eq!(fast_records[1].bytes, b"chunk-3");
    assert!(terminal.output_records().is_empty());
}

#[test]
fn output_backlog_limit_enforcement_only_touches_changed_attachment_queues() {
    let mut terminal = TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
    for index in 0..2 {
        terminal.fan_out_output(
            "session-1",
            "provider-run-a",
            Some("agent-a"),
            TerminalOutputKind::ProviderTool,
            Some(format!("a-{index}")),
            vec!["attachment-a".to_string()],
            format!("a-{index}").as_bytes(),
        );
        terminal.fan_out_output(
            "session-1",
            "provider-run-b",
            Some("agent-b"),
            TerminalOutputKind::ProviderTool,
            Some(format!("b-{index}")),
            vec!["attachment-b".to_string()],
            format!("b-{index}").as_bytes(),
        );
    }

    terminal.fan_out_output(
        "session-1",
        "provider-run-a",
        Some("agent-a"),
        TerminalOutputKind::ProviderTool,
        Some("a-2".to_string()),
        vec!["attachment-a".to_string()],
        b"a-2",
    );

    let b_records = terminal.drain_output_records("session-1", "attachment-b");
    assert_eq!(b_records.len(), 2);
    assert_eq!(b_records[0].bytes, b"b-0");
    assert_eq!(b_records[1].bytes, b"b-1");

    let a_records = terminal.drain_output_records("session-1", "attachment-a");
    assert_eq!(a_records.len(), 2);
    assert_eq!(a_records[0].bytes, b"a-1");
    assert_eq!(a_records[1].bytes, b"a-2");
    assert!(terminal.output_records().is_empty());
}

#[test]
fn health_reports_output_backlog_pressure() {
    let mut terminal = TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
    for index in 0..4 {
        terminal.fan_out_output(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            TerminalOutputKind::ProviderOutput,
            Some(format!("chunk-{index}")),
            vec!["slow-attachment".to_string()],
            format!("chunk-{index}").as_bytes(),
        );
    }
    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string()],
        "n",
    );
    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        vec!["attachment-1".to_string()],
        "message-1",
        42,
    );

    let health = terminal.health_snapshot();
    assert_eq!(health.pending_output_records, 2);
    assert_eq!(health.pending_notice_records, 1);
    assert_eq!(health.pending_completion_records, 1);
    assert_eq!(health.pending_output_record_limit_per_attachment, 2);
    assert_eq!(health.trimmed_pending_output_recipients, 2);
}

#[test]
fn adjacent_provider_output_records_coalesce() {
    let mut terminal = TerminalStreamService::new();
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"hello ",
    );
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"world",
    );

    assert_eq!(terminal.output_records().len(), 1);
    assert_eq!(terminal.output_records()[0].bytes, b"hello world");

    let first = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].bytes, b"hello world");
}

#[test]
fn output_coalescing_preserves_recipient_progress() {
    let mut terminal = TerminalStreamService::new();
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
        b"first",
    );
    let fast_first = terminal.drain_output_records("session-1", "fast-attachment");
    assert_eq!(fast_first.len(), 1);
    assert_eq!(fast_first[0].bytes, b"first");

    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["slow-attachment".to_string(), "fast-attachment".to_string()],
        b"second",
    );

    let fast_second = terminal.drain_output_records("session-1", "fast-attachment");
    assert_eq!(fast_second.len(), 1);
    assert_eq!(fast_second[0].bytes, b"second");

    let slow_records = terminal.drain_output_records("session-1", "slow-attachment");
    assert_eq!(slow_records.len(), 2);
    assert_eq!(slow_records[0].bytes, b"first");
    assert_eq!(slow_records[1].bytes, b"second");
}

#[test]
fn output_coalescing_respects_byte_limit() {
    let mut terminal = TerminalStreamService::with_output_coalesce_byte_limit(5);
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"1234",
    );
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"56",
    );

    assert_eq!(terminal.output_records().len(), 2);
    assert_eq!(terminal.output_records()[0].bytes, b"1234");
    assert_eq!(terminal.output_records()[1].bytes, b"56");
}

#[test]
fn cloned_health_store_tracks_terminal_stream_mutations() {
    let mut terminal = TerminalStreamService::with_pending_output_record_limit_per_attachment(2);
    let health_store = terminal.health_store();

    assert_eq!(health_store.snapshot().pending_output_records, 0);

    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"chunk-1",
    );
    assert_eq!(health_store.snapshot().pending_output_records, 1);

    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string()],
        "notice",
    );
    assert_eq!(health_store.snapshot().pending_notice_records, 1);

    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        vec!["attachment-1".to_string()],
        "message-1",
        42,
    );
    assert_eq!(health_store.snapshot().pending_completion_records, 1);

    let output = terminal.drain_output_records("session-1", "attachment-1");
    assert_eq!(output.len(), 1);
    assert_eq!(health_store.snapshot().pending_output_records, 0);

    let notices = terminal.drain_notice_records("session-1", "attachment-1");
    assert_eq!(notices.len(), 1);
    assert_eq!(health_store.snapshot().pending_notice_records, 0);

    let completions = terminal.drain_completion_records("session-1", "attachment-1");
    assert_eq!(completions.len(), 1);
    assert_eq!(health_store.snapshot().pending_completion_records, 0);
}

#[test]
fn notice_polling_is_per_recipient() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "queued prompt",
    );

    let first = terminal.drain_notice_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);
    assert_eq!(terminal.notice_records().len(), 1);

    let second = terminal.drain_notice_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);
    assert!(terminal.notice_records().is_empty());
}

#[test]
fn notice_pending_index_tracks_recipient_drain() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "indexed notice",
    );

    assert_eq!(
        terminal
            .pending_notice_by_attachment
            .get(&("session-1".to_string(), "attachment-1".to_string()))
            .map(VecDeque::len),
        Some(1)
    );
    assert_eq!(
        terminal
            .pending_notice_by_attachment
            .get(&("session-1".to_string(), "attachment-2".to_string()))
            .map(VecDeque::len),
        Some(1)
    );

    let first = terminal.drain_notice_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert!(!terminal
        .pending_notice_by_attachment
        .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
    assert_eq!(
        terminal
            .pending_notice_by_attachment
            .get(&("session-1".to_string(), "attachment-2".to_string()))
            .map(VecDeque::len),
        Some(1)
    );

    let second = terminal.drain_notice_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert!(terminal.pending_notice_by_attachment.is_empty());
    assert!(terminal.notice_records().is_empty());
}

#[test]
fn terminal_event_drains_consume_single_recipient_attachment_queues() {
    let mut terminal = TerminalStreamService::new();
    for index in 0..3 {
        terminal.record_notice(
            "session-1",
            None,
            Some("agent-1"),
            vec!["attachment-1".to_string()],
            format!("notice-{index}"),
        );
        terminal.record_assistant_message_completion(
            "session-1",
            "provider-run-1",
            Some("agent-1"),
            vec!["attachment-1".to_string()],
            &format!("message-{index}"),
            index,
        );
    }
    assert_eq!(
        terminal
            .pending_notice_by_attachment
            .get(&("session-1".to_string(), "attachment-1".to_string()))
            .map(VecDeque::len),
        Some(3)
    );
    assert_eq!(
        terminal
            .pending_completion_by_attachment
            .get(&("session-1".to_string(), "attachment-1".to_string()))
            .map(VecDeque::len),
        Some(3)
    );

    let notices = terminal.drain_notice_records("session-1", "attachment-1");
    let completions = terminal.drain_completion_records("session-1", "attachment-1");

    assert_eq!(notices.len(), 3);
    assert_eq!(notices[0].message, "notice-0");
    assert_eq!(completions.len(), 3);
    assert_eq!(completions[2].message_id, "message-2");
    assert!(!terminal
        .pending_notice_by_attachment
        .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
    assert!(!terminal
        .pending_completion_by_attachment
        .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
    assert!(terminal.notice_records().is_empty());
    assert!(terminal.completion_records.is_empty());
}

#[test]
fn broadcast_notice_pending_index_tracks_session_drain() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_notice("session-1", None, None, Vec::new(), "broadcast notice");
    terminal.record_notice("session-2", None, None, Vec::new(), "other session notice");

    assert_eq!(
        terminal
            .pending_broadcast_notice_by_session
            .get("session-1")
            .map(VecDeque::len),
        Some(1)
    );
    assert_eq!(
        terminal
            .pending_broadcast_notice_by_session
            .get("session-2")
            .map(VecDeque::len),
        Some(1)
    );

    let first = terminal.drain_notice_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].message, "broadcast notice");
    assert!(!terminal
        .pending_broadcast_notice_by_session
        .contains_key("session-1"));
    assert!(terminal
        .pending_broadcast_notice_by_session
        .contains_key("session-2"));
    assert_eq!(terminal.notice_records().len(), 1);
}

#[test]
fn completion_polling_is_per_recipient() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "message-1",
        42,
    );

    let first = terminal.drain_completion_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].recipient_attachment_ids, vec!["attachment-1"]);

    let second = terminal.drain_completion_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].recipient_attachment_ids, vec!["attachment-2"]);

    let none_left = terminal.drain_completion_records("session-1", "attachment-2");
    assert!(none_left.is_empty());
}

#[test]
fn completion_pending_index_tracks_recipient_drain() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        Some("agent-1"),
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "message-1",
        42,
    );

    assert_eq!(
        terminal
            .pending_completion_by_attachment
            .get(&("session-1".to_string(), "attachment-1".to_string()))
            .map(VecDeque::len),
        Some(1)
    );
    assert_eq!(
        terminal
            .pending_completion_by_attachment
            .get(&("session-1".to_string(), "attachment-2".to_string()))
            .map(VecDeque::len),
        Some(1)
    );

    let first = terminal.drain_completion_records("session-1", "attachment-1");
    assert_eq!(first.len(), 1);
    assert!(!terminal
        .pending_completion_by_attachment
        .contains_key(&("session-1".to_string(), "attachment-1".to_string())));
    assert_eq!(
        terminal
            .pending_completion_by_attachment
            .get(&("session-1".to_string(), "attachment-2".to_string()))
            .map(VecDeque::len),
        Some(1)
    );

    let second = terminal.drain_completion_records("session-1", "attachment-2");
    assert_eq!(second.len(), 1);
    assert!(terminal.pending_completion_by_attachment.is_empty());
    assert!(terminal.completion_records.is_empty());
}

#[test]
fn removing_attachment_prunes_pending_terminal_records() {
    let mut terminal = TerminalStreamService::new();
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        None,
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        b"output",
    );
    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "notice",
    );
    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        None,
        vec!["attachment-1".to_string(), "attachment-2".to_string()],
        "message-1",
        1,
    );

    assert!(terminal.remove_attachment("session-1", "attachment-1"));

    assert_eq!(
        terminal.output_records()[0].pending_recipient_attachment_ids,
        vec!["attachment-2".to_string()],
    );
    assert_eq!(
        terminal.notice_records()[0].pending_recipient_attachment_ids,
        vec!["attachment-2".to_string()],
    );
    assert_eq!(
        terminal
            .completion_records
            .values()
            .next()
            .expect("completion record should remain")
            .pending_recipient_attachment_ids,
        vec!["attachment-2".to_string()],
    );

    assert!(terminal.remove_attachment("session-1", "attachment-2"));
    assert!(terminal.output_records().is_empty());
    assert!(terminal.notice_records().is_empty());
    assert!(terminal.completion_records.is_empty());
}

#[test]
fn removes_all_records_for_session() {
    let mut terminal = TerminalStreamService::new();
    terminal.record_input("session-1", "provider-run-1", "attachment-1", b"one");
    terminal.record_input("session-2", "provider-run-2", "attachment-2", b"two");
    terminal.fan_out_output(
        "session-1",
        "provider-run-1",
        None,
        TerminalOutputKind::ProviderOutput,
        None,
        vec!["attachment-1".to_string()],
        b"output",
    );
    terminal.record_notice(
        "session-1",
        None,
        None,
        vec!["attachment-1".to_string()],
        "notice",
    );
    terminal.record_assistant_message_completion(
        "session-1",
        "provider-run-1",
        None,
        vec!["attachment-1".to_string()],
        "message-1",
        1,
    );

    terminal.remove_session("session-1");

    assert_eq!(terminal.input_records().len(), 1);
    assert_eq!(terminal.input_records()[0].session_id, "session-2");
    assert!(terminal.output_records().is_empty());
    assert!(terminal.notice_records().is_empty());
    assert_eq!(terminal.health_snapshot().pending_output_records, 0);
    assert_eq!(terminal.health_snapshot().pending_notice_records, 0);
    assert_eq!(terminal.health_snapshot().pending_completion_records, 0);
}
