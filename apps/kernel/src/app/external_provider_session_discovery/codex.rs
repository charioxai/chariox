use super::*;

pub(super) fn parse_codex_transcript(path: &Path) -> Option<ExternalProviderSessionRecord> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(record) = cached_provider_discovery_record_for_path("codex", path, fingerprint) {
        return Some(record);
    }
    let lines = read_jsonl_values(path);
    let mut provider_session_id = None;
    let mut worktree_path = None;
    let mut created_at_ms = None;
    let mut account_profile = None;
    let mut first_prompt = None;

    for value in lines {
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(payload) = value.get("payload") {
                provider_session_id = provider_session_id
                    .or_else(|| string_field(payload, &["id", "session_id", "sessionId"]));
                worktree_path = worktree_path.or_else(|| string_field(payload, &["cwd"]));
                account_profile =
                    account_profile.or_else(|| string_field(payload, &["model_provider"]));
                created_at_ms = created_at_ms.or_else(|| {
                    string_field(payload, &["timestamp"])
                        .and_then(|timestamp| parse_rfc3339_millis_utc(&timestamp))
                });
            }
            continue;
        }
        if first_prompt.is_none() {
            first_prompt = codex_user_prompt(&value);
        }
    }

    let provider_session_id = provider_session_id.or_else(|| file_stem(path))?;
    let capabilities = observed_capabilities(true);
    let record = record_from_parts(
        "codex",
        provider_session_id.clone(),
        first_prompt,
        worktree_path,
        created_at_ms,
        fingerprint.modified_at_ms,
        account_profile,
        capabilities,
    );
    remember_provider_discovery_record(
        "codex",
        &provider_session_id,
        path,
        fingerprint,
        record.clone(),
    );
    Some(record)
}

pub(super) fn read_codex_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    if let Some(path) = cached_provider_transcript_path_in_root("codex", provider_session_id, root)
    {
        if let Some(turns) = codex_observed_turns_from_path(&path, provider_session_id) {
            return turns;
        }
    }
    let mut candidates = jsonl_candidates(&root.join("archived_sessions"), 4);
    candidates.extend(jsonl_candidates(&root.join("sessions"), 4));
    sort_file_candidates_by_recent_modified(&mut candidates);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| codex_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

pub(super) fn codex_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    let observed_fingerprint = complete_jsonl_fingerprint(path, fingerprint)?;
    if let Some(turns) =
        cached_provider_observed_turns("codex", provider_session_id, path, observed_fingerprint)
    {
        return Some(turns);
    }
    if !cached_provider_transcript_identity_matches("codex", provider_session_id, path, fingerprint)
    {
        let parsed_session_id =
            codex_session_id_from_values(&read_jsonl_values(path)).or_else(|| file_stem(path))?;
        if parsed_session_id != provider_session_id {
            return None;
        }
    }
    let previous_transcript =
        cached_provider_observed_transcript_for_path("codex", provider_session_id, path);
    remember_provider_transcript_path_with_fingerprint(
        "codex",
        provider_session_id,
        path,
        observed_fingerprint,
    );
    if let Some(turns) =
        incremental_codex_observed_turns(path, previous_transcript.as_ref(), observed_fingerprint)
    {
        remember_provider_observed_turns(
            "codex",
            provider_session_id,
            path,
            observed_fingerprint,
            turns.clone(),
        );
        return Some(turns);
    }
    let lines = read_recent_codex_jsonl_values(path);
    let mut turns = Vec::new();
    for value in &lines {
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            continue;
        }
        if let Some(turn) = codex_observed_turn_from_value(value) {
            turns.push(turn);
        }
    }
    let turns = latest_observed_turns(deduplicate_codex_mirrored_turns(turns));
    remember_provider_observed_turns(
        "codex",
        provider_session_id,
        path,
        observed_fingerprint,
        turns.clone(),
    );
    Some(turns)
}

pub(super) fn incremental_codex_observed_turns(
    path: &Path,
    previous: Option<&CachedProviderObservedTranscript>,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let previous = previous?;
    if previous.last_observed_offset > fingerprint.len {
        return None;
    }
    let values = read_incremental_jsonl_values(path, previous.last_observed_offset)?;
    let mut turns = previous.observed_turns.clone();
    turns.extend(values.iter().filter_map(codex_observed_turn_from_value));
    Some(latest_observed_turns(deduplicate_codex_mirrored_turns(
        turns,
    )))
}

pub(super) fn codex_observed_turn_from_value(
    value: &Value,
) -> Option<ObservedExternalProviderTurn> {
    let observed_at_ms = string_field(value, &["timestamp"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp));
    match value.get("type").and_then(Value::as_str) {
        Some("response_item") => {
            codex_response_item_observed_turn(value.get("payload").unwrap_or(value), observed_at_ms)
        }
        Some("event_msg") => {
            codex_event_message_observed_turn(value.get("payload").unwrap_or(value), observed_at_ms)
        }
        Some("session_meta") => None,
        _ => {
            codex_response_item_observed_turn(value.get("payload").unwrap_or(value), observed_at_ms)
        }
    }
}

pub(super) fn codex_response_item_observed_turn(
    payload: &Value,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    let item_type = payload.get("type").and_then(Value::as_str);
    if item_type == Some("message") {
        let role = payload.get("role").and_then(Value::as_str);
        let role = observed_role(role)?;
        let text = payload
            .get("content")
            .and_then(text_from_content)
            .and_then(|text| {
                clean_observed_turn_text(payload.get("role").and_then(Value::as_str), text)
            })?;
        return Some(ObservedExternalProviderTurn {
            role,
            text,
            provider_turn_id: string_field(payload, &["id", "item_id", "message_id"]),
            observed_at_ms,
        });
    }
    if item_type == Some("agentMessage") {
        return Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| payload.get("content").and_then(text_from_content))?,
            provider_turn_id: string_field(payload, &["id", "item_id", "message_id"]),
            observed_at_ms,
        });
    }
    if item_type == Some("reasoning") {
        let visible_reasoning = payload
            .get("summary")
            .and_then(text_from_content)
            .or_else(|| payload.get("content").and_then(text_from_content))
            .or_else(|| {
                payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        return Some(ObservedExternalProviderTurn {
            role: visible_reasoning
                .as_ref()
                .map(|_| ObservedExternalProviderTurnRole::Reasoning)
                .unwrap_or(ObservedExternalProviderTurnRole::Status),
            text: visible_reasoning.unwrap_or_else(|| {
                "codex reasoning item observed; visible summary unavailable".to_string()
            }),
            provider_turn_id: string_field(payload, &["id", "item_id", "message_id"])
                .or_else(|| observed_at_ms.map(|ms| format!("reasoning-{ms}"))),
            observed_at_ms,
        });
    }
    if matches!(
        item_type,
        Some(
            "function_call"
                | "function_call_output"
                | "commandExecution"
                | "fileChange"
                | "mcpToolCall"
                | "dynamicToolCall"
                | "collabAgentToolCall"
                | "local_shell_call"
        )
    ) {
        return Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Tool,
            text: codex_tool_text(payload),
            provider_turn_id: codex_tool_turn_id(payload),
            observed_at_ms,
        });
    }
    item_type.map(|item_type| ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: codex_metadata_text(&format!("codex response item {item_type}"), payload),
        provider_turn_id: string_field(payload, &["id", "item_id", "message_id"])
            .or_else(|| observed_at_ms.map(|ms| format!("{item_type}-{ms}"))),
        observed_at_ms,
    })
}

pub(super) fn codex_event_message_observed_turn(
    payload: &Value,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    let event_type = payload.get("type").and_then(Value::as_str)?;
    match event_type {
        "user_message" => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: clean_observed_turn_text(
                Some("user"),
                payload.get("message").and_then(Value::as_str)?.to_string(),
            )?,
            provider_turn_id: string_field(payload, &["id", "item_id", "message_id"])
                .or_else(|| observed_at_ms.map(|ms| format!("user-message-{ms}"))),
            observed_at_ms,
        }),
        "agent_reasoning" => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Reasoning,
            text: payload.get("text").and_then(Value::as_str)?.to_string(),
            provider_turn_id: observed_at_ms.map(|ms| format!("agent-reasoning-{ms}")),
            observed_at_ms,
        }),
        "agent_message" => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Assistant,
            text: payload.get("message").and_then(Value::as_str)?.to_string(),
            provider_turn_id: observed_at_ms.map(|ms| format!("agent-message-{ms}")),
            observed_at_ms,
        }),
        "mcp_tool_call_end" => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Tool,
            text: codex_metadata_text("codex mcp tool call ended", payload),
            provider_turn_id: string_field(payload, &["call_id"])
                .map(|call_id| format!("mcp-tool-end-{call_id}")),
            observed_at_ms,
        }),
        "token_count" | "task_started" | "task_complete" => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: codex_metadata_text(&format!("codex {event_type}"), payload),
            provider_turn_id: string_field(payload, &["turn_id"])
                .map(|turn_id| format!("{event_type}-{turn_id}"))
                .or_else(|| observed_at_ms.map(|ms| format!("{event_type}-{ms}"))),
            observed_at_ms,
        }),
        _ => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: codex_metadata_text(&format!("codex event {event_type}"), payload),
            provider_turn_id: observed_at_ms.map(|ms| format!("{event_type}-{ms}")),
            observed_at_ms,
        }),
    }
}

pub(super) fn deduplicate_codex_mirrored_turns(
    turns: Vec<ObservedExternalProviderTurn>,
) -> Vec<ObservedExternalProviderTurn> {
    let mut deduplicated: Vec<ObservedExternalProviderTurn> = Vec::new();
    for turn in turns {
        if let Some(existing_index) = deduplicated
            .iter()
            .position(|existing| codex_mirrored_visible_message(existing, &turn))
        {
            if codex_turn_identity_is_richer(&turn, &deduplicated[existing_index]) {
                deduplicated[existing_index] = turn;
            }
            continue;
        }
        deduplicated.push(turn);
    }
    deduplicated
}

pub(super) fn codex_mirrored_visible_message(
    left: &ObservedExternalProviderTurn,
    right: &ObservedExternalProviderTurn,
) -> bool {
    matches!(
        left.role,
        ObservedExternalProviderTurnRole::User | ObservedExternalProviderTurnRole::Assistant
    ) && left.role == right.role
        && left.text == right.text
        && observed_timestamps_are_close(left.observed_at_ms, right.observed_at_ms)
}

pub(super) fn observed_timestamps_are_close(left: Option<u64>, right: Option<u64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= 2_000,
        _ => false,
    }
}

pub(super) fn codex_turn_identity_is_richer(
    candidate: &ObservedExternalProviderTurn,
    existing: &ObservedExternalProviderTurn,
) -> bool {
    candidate
        .provider_turn_id
        .as_deref()
        .is_some_and(|id| id.starts_with("msg_"))
        && !existing
            .provider_turn_id
            .as_deref()
            .is_some_and(|id| id.starts_with("msg_"))
}

pub(super) fn codex_tool_turn_id(payload: &Value) -> Option<String> {
    let item_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    string_field(payload, &["id", "item_id", "message_id"])
        .or_else(|| string_field(payload, &["call_id"]))
        .map(|id| format!("{item_type}-{id}"))
}

pub(super) fn codex_tool_text(payload: &Value) -> String {
    match payload.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            let name =
                string_field(payload, &["name"]).unwrap_or_else(|| "function_call".to_string());
            let namespace = string_field(payload, &["namespace"]);
            let arguments = payload
                .get("arguments")
                .and_then(|value| value.as_str().map(parse_bounded_json_string_or_raw))
                .unwrap_or(Value::Null);
            compact_json_text(serde_json::json!({
                "tool": name,
                "namespace": namespace,
                "status": "called",
                "arguments": arguments,
                "call_id": string_field(payload, &["call_id"]),
            }))
        }
        Some("function_call_output") => compact_json_text(serde_json::json!({
            "tool": "function_call_output",
            "status": "completed",
            "call_id": string_field(payload, &["call_id"]),
            "output": payload.get("output").map(bounded_observed_metadata_value).unwrap_or(Value::Null),
        })),
        _ => codex_metadata_text("codex tool item", payload),
    }
}

pub(super) fn codex_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
}

pub(super) fn codex_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines.iter().find_map(|value| {
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            return None;
        }
        value
            .get("payload")
            .and_then(|payload| string_field(payload, &["id", "session_id", "sessionId"]))
    })
}
