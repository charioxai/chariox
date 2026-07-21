use super::*;

pub(super) fn is_coalescible_output_kind(kind: &TerminalOutputKind) -> bool {
    matches!(
        kind,
        TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
    )
}

pub(super) fn scoped_output_record(
    record: &TerminalOutputRecord,
    record_id: u64,
    attachment_id: &str,
) -> TerminalOutputRecord {
    TerminalOutputRecord {
        record_id: Some(record_id),
        timestamp_ms: record.timestamp_ms,
        session_id: record.session_id.clone(),
        provider_run_id: record.provider_run_id.clone(),
        agent_id: record.agent_id.clone(),
        prompt_id: record.prompt_id.clone(),
        prompt_origin: record.prompt_origin,
        source_attachment_id: record.source_attachment_id.clone(),
        kind: record.kind.clone(),
        merge_key: record.merge_key.clone(),
        recipient_attachment_ids: vec![attachment_id.to_string()],
        pending_recipient_attachment_ids: vec![attachment_id.to_string()],
        bytes: record.bytes.clone(),
        external_observation_metadata: record.external_observation_metadata.clone(),
    }
}

pub(super) fn scoped_notice_record(
    record: &RuntimeNoticeRecord,
    attachment_id: &str,
) -> RuntimeNoticeRecord {
    let mut scoped = record.clone();
    if !scoped.recipient_attachment_ids.is_empty() {
        scoped.recipient_attachment_ids = vec![attachment_id.to_string()];
    }
    scoped.pending_recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped
}

pub(super) fn scoped_completion_record(
    record: &AssistantMessageCompletionRecord,
    attachment_id: &str,
) -> AssistantMessageCompletionRecord {
    let mut scoped = record.clone();
    scoped.recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped.pending_recipient_attachment_ids = vec![attachment_id.to_string()];
    scoped
}

pub(super) fn remove_pending_recipient_id(pending: &mut Vec<String>, attachment_id: &str) -> bool {
    match pending.as_slice() {
        [only] if only == attachment_id => {
            pending.clear();
            true
        }
        _ => {
            let Some(index) = pending.iter().position(|id| id == attachment_id) else {
                return false;
            };
            pending.remove(index);
            true
        }
    }
}

pub(super) fn terminal_output_record_scoped_json_bytes(
    record: &TerminalOutputRecord,
    attachment_id: &str,
) -> usize {
    let mut total = 2_usize;
    let mut field_count = 0_usize;
    add_json_field(
        &mut total,
        &mut field_count,
        "record_id",
        json_u64_len(u64::MAX),
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "timestamp_ms",
        json_u64_len(record.timestamp_ms),
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "session_id",
        json_string_len(&record.session_id),
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "provider_run_id",
        json_string_len(&record.provider_run_id),
    );
    if let Some(agent_id) = &record.agent_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "agent_id",
            json_string_len(agent_id),
        );
    }
    if let Some(prompt_id) = &record.prompt_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "prompt_id",
            json_string_len(prompt_id),
        );
    }
    if let Some(source_attachment_id) = &record.source_attachment_id {
        add_json_field(
            &mut total,
            &mut field_count,
            "source_attachment_id",
            json_string_len(source_attachment_id),
        );
    }
    add_json_field(
        &mut total,
        &mut field_count,
        "kind",
        json_string_len(terminal_output_kind_json(&record.kind)),
    );
    if let Some(merge_key) = &record.merge_key {
        add_json_field(
            &mut total,
            &mut field_count,
            "merge_key",
            json_string_len(merge_key),
        );
    }
    let scoped_attachment_array_len = json_string_array_len(std::slice::from_ref(&attachment_id));
    add_json_field(
        &mut total,
        &mut field_count,
        "recipient_attachment_ids",
        scoped_attachment_array_len,
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "pending_recipient_attachment_ids",
        scoped_attachment_array_len,
    );
    add_json_field(
        &mut total,
        &mut field_count,
        "bytes",
        json_byte_array_len(&record.bytes),
    );
    total
}

fn add_json_field(total: &mut usize, field_count: &mut usize, field: &str, value_len: usize) {
    if *field_count > 0 {
        *total = total.saturating_add(1);
    }
    *field_count = field_count.saturating_add(1);
    *total = total
        .saturating_add(json_string_len(field))
        .saturating_add(1)
        .saturating_add(value_len);
}

fn terminal_output_kind_json(kind: &TerminalOutputKind) -> &'static str {
    match kind {
        TerminalOutputKind::ProviderOutput => "provider_output",
        TerminalOutputKind::ProviderTerminal => "provider_terminal",
        TerminalOutputKind::PromptEcho => "prompt_echo",
        TerminalOutputKind::ProviderReasoning => "provider_reasoning",
        TerminalOutputKind::ProviderTool => "provider_tool",
        TerminalOutputKind::ProviderError => "provider_error",
        TerminalOutputKind::ProviderStatus => "provider_status",
    }
}

fn json_string_array_len(values: &[&str]) -> usize {
    let commas = values.len().saturating_sub(1);
    2_usize
        .saturating_add(commas)
        .saturating_add(values.iter().fold(0_usize, |total, value| {
            total.saturating_add(json_string_len(value))
        }))
}

fn json_string_len(value: &str) -> usize {
    value.chars().fold(2_usize, |total, character| {
        total.saturating_add(match character {
            '"' | '\\' => 2,
            '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
            character if character <= '\u{1f}' => 6,
            character => character.len_utf8(),
        })
    })
}

fn json_u64_len(value: u64) -> usize {
    value.to_string().len()
}

fn json_byte_array_len(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 2;
    }
    2_usize
        .saturating_add(bytes.len().saturating_sub(1))
        .saturating_add(bytes.iter().fold(0_usize, |total, byte| {
            total.saturating_add(match byte {
                0..=9 => 1,
                10..=99 => 2,
                _ => 3,
            })
        }))
}
