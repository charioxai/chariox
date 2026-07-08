use super::*;

pub(super) fn parse_claude_transcript(path: &Path) -> Option<ExternalProviderSessionRecord> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(record) = cached_provider_discovery_record_for_path("claude", path, fingerprint) {
        return Some(record);
    }
    let lines = read_jsonl_values(path);
    let mut provider_session_id = None;
    let mut worktree_path = None;
    let mut created_at_ms = None;
    let mut first_prompt = None;

    for value in lines {
        provider_session_id =
            provider_session_id.or_else(|| string_field(&value, &["sessionId", "session_id"]));
        worktree_path = worktree_path.or_else(|| string_field(&value, &["cwd"]));
        created_at_ms = created_at_ms.or_else(|| {
            string_field(&value, &["timestamp"])
                .and_then(|timestamp| parse_rfc3339_millis_utc(&timestamp))
        });
        if first_prompt.is_none() {
            first_prompt = claude_user_prompt(&value);
        }
    }

    let provider_session_id = provider_session_id.or_else(|| file_stem(path))?;
    let capabilities = observed_capabilities(true);
    let record = record_from_parts(
        "claude",
        provider_session_id.clone(),
        first_prompt,
        worktree_path,
        created_at_ms,
        fingerprint.modified_at_ms,
        None,
        capabilities,
    );
    remember_provider_discovery_record(
        "claude",
        &provider_session_id,
        path,
        fingerprint,
        record.clone(),
    );
    Some(record)
}

pub(super) fn read_claude_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    if let Some(path) = cached_provider_transcript_path_in_root("claude", provider_session_id, root)
    {
        if let Some(turns) = claude_observed_turns_from_path(&path, provider_session_id) {
            return turns;
        }
    }
    let mut candidates = jsonl_candidates(&root.join("projects"), 3);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| claude_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

pub(super) fn claude_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    let observed_fingerprint = complete_jsonl_fingerprint(path, fingerprint)?;
    if let Some(turns) =
        cached_provider_observed_turns("claude", provider_session_id, path, observed_fingerprint)
    {
        return Some(turns);
    }
    if !cached_provider_transcript_identity_matches(
        "claude",
        provider_session_id,
        path,
        fingerprint,
    ) {
        let parsed_session_id =
            claude_session_id_from_values(&read_jsonl_values(path)).or_else(|| file_stem(path))?;
        if parsed_session_id != provider_session_id {
            return None;
        }
    }
    let previous_turns =
        cached_provider_observed_turns_for_path("claude", provider_session_id, path);
    let previous_transcript =
        cached_provider_observed_transcript_for_path("claude", provider_session_id, path);
    remember_provider_transcript_path_with_fingerprint(
        "claude",
        provider_session_id,
        path,
        observed_fingerprint,
    );
    if let Some(turns) =
        incremental_claude_observed_turns(path, previous_transcript.as_ref(), observed_fingerprint)
    {
        remember_provider_observed_turns(
            "claude",
            provider_session_id,
            path,
            observed_fingerprint,
            turns.clone(),
        );
        return Some(turns);
    }
    let lines = read_recent_claude_jsonl_values(path);
    let mut turns = Vec::new();
    for value in &lines {
        turns.extend(claude_observed_turns_from_value(value));
    }
    prepend_claude_user_anchor_from_cache_or_prefix(path, previous_turns.as_deref(), &mut turns);
    let turns = latest_observed_turns(turns);
    remember_provider_observed_turns(
        "claude",
        provider_session_id,
        path,
        observed_fingerprint,
        turns.clone(),
    );
    Some(turns)
}

pub(super) fn incremental_claude_observed_turns(
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
    for value in &values {
        turns.extend(claude_observed_turns_from_value(value));
    }
    Some(latest_observed_turns(turns))
}

pub(super) fn prepend_claude_user_anchor_from_cache_or_prefix(
    path: &Path,
    previous_turns: Option<&[ObservedExternalProviderTurn]>,
    turns: &mut Vec<ObservedExternalProviderTurn>,
) {
    if turns
        .iter()
        .any(|turn| turn.role == ObservedExternalProviderTurnRole::User)
    {
        return;
    }
    let anchor = previous_turns
        .and_then(latest_observed_user_turn)
        .cloned()
        .or_else(|| claude_user_anchor_from_prefix(path));
    if let Some(anchor) = anchor {
        turns.insert(0, anchor);
    }
}

pub(super) fn latest_observed_user_turn(
    turns: &[ObservedExternalProviderTurn],
) -> Option<&ObservedExternalProviderTurn> {
    turns
        .iter()
        .rev()
        .find(|turn| turn.role == ObservedExternalProviderTurnRole::User)
}

pub(super) fn claude_user_anchor_from_prefix(path: &Path) -> Option<ObservedExternalProviderTurn> {
    read_jsonl_values(path).into_iter().rev().find_map(|value| {
        claude_observed_turns_from_value(&value)
            .into_iter()
            .find(|turn| turn.role == ObservedExternalProviderTurnRole::User)
    })
}

pub(super) fn claude_observed_turns_from_value(value: &Value) -> Vec<ObservedExternalProviderTurn> {
    let observed_at_ms = string_field(value, &["timestamp"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp));
    let record_type = value.get("type").and_then(Value::as_str);
    match record_type {
        Some("user") => claude_user_observed_turns(value, observed_at_ms),
        Some("assistant") => claude_assistant_observed_turns(value, observed_at_ms),
        Some("mode" | "permission-mode" | "queue-operation" | "ai-title" | "last-prompt") => {
            vec![ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Status,
                text: claude_metadata_text(&format!("claude {}", record_type.unwrap()), value),
                provider_turn_id: claude_record_turn_id(
                    value,
                    record_type.unwrap(),
                    observed_at_ms,
                ),
                observed_at_ms,
            }]
        }
        _ => Vec::new(),
    }
}

pub(super) fn claude_user_observed_turns(
    value: &Value,
    observed_at_ms: Option<u64>,
) -> Vec<ObservedExternalProviderTurn> {
    let message = value.get("message").unwrap_or(value);
    let content = message.get("content").or_else(|| value.get("content"));
    let mut turns = Vec::new();
    let mut prompt_parts = Vec::new();
    match content {
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                match item.get("type").and_then(Value::as_str) {
                    Some("tool_result") => {
                        if let Some(text) = claude_tool_result_text(item) {
                            turns.push(ObservedExternalProviderTurn {
                                role: ObservedExternalProviderTurnRole::Tool,
                                text,
                                provider_turn_id: claude_content_turn_id(
                                    value,
                                    message,
                                    item,
                                    "tool-result",
                                    index,
                                    observed_at_ms,
                                ),
                                observed_at_ms,
                            });
                        }
                    }
                    Some("text") | None => {
                        if let Some(text) = item
                            .get("text")
                            .or_else(|| item.get("content"))
                            .and_then(Value::as_str)
                        {
                            prompt_parts.push(text.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::String(text)) => prompt_parts.push(text.to_string()),
        Some(Value::Object(_)) => {
            if let Some(text) = content.and_then(text_from_content) {
                prompt_parts.push(text);
            }
        }
        _ => {}
    }
    let prompt = prompt_parts.join("\n");
    if let Some(text) = clean_observed_turn_text(Some("user"), prompt) {
        turns.insert(
            0,
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::User,
                text,
                provider_turn_id: string_field(value, &["uuid", "id", "message_id"])
                    .or_else(|| string_field(message, &["id"])),
                observed_at_ms,
            },
        );
    }
    turns
}

pub(super) fn claude_assistant_observed_turns(
    value: &Value,
    observed_at_ms: Option<u64>,
) -> Vec<ObservedExternalProviderTurn> {
    let message = value.get("message").unwrap_or(value);
    let Some(content) = message.get("content").or_else(|| value.get("content")) else {
        return Vec::new();
    };
    let mut turns = match content {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let single_content_block = items.len() == 1;
                let block_type = item.get("type").and_then(Value::as_str).unwrap_or("text");
                match block_type {
                    "text" => item
                        .get("text")
                        .or_else(|| item.get("content"))
                        .and_then(Value::as_str)
                        .and_then(|text| {
                            clean_observed_turn_text(Some("assistant"), text.to_string())
                        })
                        .map(|text| ObservedExternalProviderTurn {
                            role: ObservedExternalProviderTurnRole::Assistant,
                            text,
                            provider_turn_id: if single_content_block {
                                string_field(value, &["uuid", "id", "message_id"])
                                    .or_else(|| string_field(message, &["id"]))
                            } else {
                                claude_content_turn_id(
                                    value,
                                    message,
                                    item,
                                    "assistant",
                                    index,
                                    observed_at_ms,
                                )
                            },
                            observed_at_ms,
                        }),
                    "thinking" => item
                        .get("thinking")
                        .or_else(|| item.get("text"))
                        .or_else(|| item.get("content"))
                        .and_then(Value::as_str)
                        .and_then(|text| {
                            clean_observed_turn_text(Some("reasoning"), text.to_string())
                        })
                        .map(|text| ObservedExternalProviderTurn {
                            role: ObservedExternalProviderTurnRole::Reasoning,
                            text,
                            provider_turn_id: claude_content_turn_id(
                                value,
                                message,
                                item,
                                "thinking",
                                index,
                                observed_at_ms,
                            ),
                            observed_at_ms,
                        }),
                    "tool_use" => Some(ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Tool,
                        text: claude_tool_use_text(item),
                        provider_turn_id: claude_content_turn_id(
                            value,
                            message,
                            item,
                            "tool-use",
                            index,
                            observed_at_ms,
                        ),
                        observed_at_ms,
                    }),
                    "tool_result" => {
                        claude_tool_result_text(item).map(|text| ObservedExternalProviderTurn {
                            role: ObservedExternalProviderTurnRole::Tool,
                            text,
                            provider_turn_id: claude_content_turn_id(
                                value,
                                message,
                                item,
                                "tool-result",
                                index,
                                observed_at_ms,
                            ),
                            observed_at_ms,
                        })
                    }
                    _ => Some(ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Status,
                        text: claude_metadata_text(&format!("claude content {block_type}"), item),
                        provider_turn_id: claude_content_turn_id(
                            value,
                            message,
                            item,
                            block_type,
                            index,
                            observed_at_ms,
                        ),
                        observed_at_ms,
                    }),
                }
            })
            .collect(),
        Value::String(text) => clean_observed_turn_text(Some("assistant"), text.to_string())
            .map(|text| {
                vec![ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text,
                    provider_turn_id: string_field(value, &["uuid", "id", "message_id"])
                        .or_else(|| string_field(message, &["id"])),
                    observed_at_ms,
                }]
            })
            .unwrap_or_default(),
        Value::Object(_) => text_from_content(content)
            .and_then(|text| clean_observed_turn_text(Some("assistant"), text))
            .map(|text| {
                vec![ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text,
                    provider_turn_id: string_field(value, &["uuid", "id", "message_id"])
                        .or_else(|| string_field(message, &["id"])),
                    observed_at_ms,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if let Some(completion) = claude_assistant_completion_status(value, message, observed_at_ms) {
        turns.push(completion);
    }
    turns
}

pub(super) fn claude_assistant_completion_status(
    value: &Value,
    message: &Value,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    let stop_reason = string_field(message, &["stop_reason", "stopReason"])
        .or_else(|| string_field(value, &["stop_reason", "stopReason"]))?;
    if stop_reason == "tool_use" {
        return None;
    }
    let provider_turn_id = string_field(value, &["uuid", "id", "message_id"])
        .or_else(|| string_field(message, &["id"]))
        .map(|id| format!("{id}:completed"));
    let details = compact_json_text(serde_json::json!({
        "type": "claude_message_completed",
        "stop_reason": stop_reason,
    }));
    Some(ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: format!("claude message completed\n{details}"),
        provider_turn_id,
        observed_at_ms,
    })
}

pub(super) fn claude_tool_use_text(item: &Value) -> String {
    let name = string_field(item, &["name"]).unwrap_or_else(|| "tool_use".to_string());
    compact_json_text(serde_json::json!({
        "tool": name,
        "status": "called",
        "id": string_field(item, &["id"]),
        "input": item.get("input").cloned().unwrap_or(Value::Null),
    }))
}

pub(super) fn claude_tool_result_text(item: &Value) -> Option<String> {
    let content = item
        .get("content")
        .and_then(text_from_content)
        .unwrap_or_else(|| {
            item.get("content")
                .cloned()
                .map(compact_json_text)
                .unwrap_or_else(|| "".to_string())
        });
    let content = content.trim();
    if content.is_empty() {
        return None;
    }
    Some(compact_json_text(serde_json::json!({
        "id": string_field(item, &["tool_use_id", "toolUseId"]),
        "tool": "tool_result",
        "status": if item.get("is_error").and_then(Value::as_bool) == Some(true) {
            "failed"
        } else {
            "completed"
        },
        "output": content,
    })))
}

pub(super) fn claude_record_turn_id(
    value: &Value,
    label: &str,
    observed_at_ms: Option<u64>,
) -> Option<String> {
    string_field(value, &["uuid", "id", "message_id"])
        .map(|id| format!("{label}-{id}"))
        .or_else(|| {
            string_field(value, &["leafUuid", "leaf_uuid"]).map(|id| format!("{label}-leaf-{id}"))
        })
        .or_else(|| {
            string_field(value, &["sessionId", "session_id"])
                .map(|id| format!("{label}-session-{id}"))
        })
        .or_else(|| observed_at_ms.map(|ms| format!("{label}-{ms}")))
}

pub(super) fn claude_content_turn_id(
    value: &Value,
    message: &Value,
    item: &Value,
    label: &str,
    index: usize,
    observed_at_ms: Option<u64>,
) -> Option<String> {
    string_field(item, &["id", "tool_use_id", "toolUseId"])
        .map(|id| format!("{label}-{id}"))
        .or_else(|| {
            string_field(message, &["id"])
                .or_else(|| string_field(value, &["uuid", "id", "message_id"]))
                .map(|id| format!("{label}-{id}-{index}"))
        })
        .or_else(|| observed_at_ms.map(|ms| format!("{label}-{ms}-{index}")))
}

pub(super) fn claude_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
}

pub(super) fn claude_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines
        .iter()
        .find_map(|value| string_field(value, &["sessionId", "session_id"]))
}
