use super::*;

pub(super) fn parse_opencode_session_file(path: &Path) -> Option<ExternalProviderSessionRecord> {
    if is_opencode_sqlite_db(path) {
        return None;
    }
    if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
        return parse_opencode_jsonl(path);
    }
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(record) = cached_provider_discovery_record_for_path("opencode", path, fingerprint) {
        return Some(record);
    }
    let payload = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&payload).ok()?;
    let provider_session_id = string_field(&value, &["id", "sessionID", "sessionId", "session_id"])
        .or_else(|| file_stem(path))?;
    let title = string_field(&value, &["title", "name"]);
    let first_prompt = title.clone().or_else(|| opencode_user_prompt(&value));
    let worktree_path = string_field(&value, &["cwd", "path", "workspace"]);
    let created_at_ms = string_field(&value, &["created", "createdAt", "timeCreated"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp));
    let last_modified_at_ms = string_field(&value, &["updated", "updatedAt", "timeUpdated"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp))
        .unwrap_or(fingerprint.modified_at_ms);
    let capabilities = observed_capabilities(true);
    let record = record_from_parts(
        "opencode",
        provider_session_id.clone(),
        first_prompt,
        worktree_path,
        created_at_ms,
        last_modified_at_ms,
        None,
        capabilities,
    );
    remember_provider_discovery_record(
        "opencode",
        &provider_session_id,
        path,
        fingerprint,
        record.clone(),
    );
    Some(record)
}

pub(super) fn read_opencode_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let sqlite_turns = read_opencode_sqlite_observed_turns(root, provider_session_id);
    if !sqlite_turns.is_empty() {
        return latest_observed_turns(sqlite_turns);
    }
    if let Some(path) =
        cached_provider_transcript_path_in_root("opencode", provider_session_id, root)
    {
        if let Some(turns) = opencode_observed_turns_from_path(&path, provider_session_id) {
            return turns;
        }
    }
    let mut candidates = session_json_candidates(root, 5);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| opencode_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

pub(super) fn opencode_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    if is_opencode_sqlite_db(path) {
        return Some(read_opencode_sqlite_observed_turns(
            path.parent().unwrap_or_else(|| Path::new("")),
            provider_session_id,
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
        return opencode_jsonl_observed_turns_from_path(path, provider_session_id);
    }
    let payload = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&payload).ok()?;
    let parsed_session_id = string_field(&value, &["id", "sessionID", "sessionId", "session_id"])
        .or_else(|| file_stem(path))?;
    if parsed_session_id != provider_session_id {
        return None;
    }
    remember_provider_transcript_path("opencode", provider_session_id, path);
    let messages = value
        .get("messages")
        .or_else(|| value.get("conversation"))
        .or_else(|| value.get("entries"))
        .and_then(Value::as_array)?;
    Some(latest_observed_turns(
        messages
            .iter()
            .flat_map(opencode_observed_turns_from_value)
            .collect(),
    ))
}

pub(super) fn opencode_jsonl_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(turns) =
        cached_provider_observed_turns("opencode", provider_session_id, path, fingerprint)
    {
        return Some(turns);
    }
    if !cached_provider_transcript_identity_matches(
        "opencode",
        provider_session_id,
        path,
        fingerprint,
    ) {
        let parsed_session_id = opencode_session_id_from_values(&read_jsonl_values(path))
            .or_else(|| file_stem(path))?;
        if parsed_session_id != provider_session_id {
            return None;
        }
    }
    let previous_transcript =
        cached_provider_observed_transcript_for_path("opencode", provider_session_id, path);
    remember_provider_transcript_path("opencode", provider_session_id, path);
    if let Some(turns) =
        incremental_opencode_observed_turns(path, previous_transcript.as_ref(), fingerprint)
    {
        remember_provider_observed_turns(
            "opencode",
            provider_session_id,
            path,
            fingerprint,
            turns.clone(),
        );
        return Some(turns);
    }
    let lines = read_recent_jsonl_values(path);
    let mut turns = Vec::new();
    for value in &lines {
        turns.extend(opencode_observed_turns_from_value(value));
    }
    let turns = latest_observed_turns(turns);
    remember_provider_observed_turns(
        "opencode",
        provider_session_id,
        path,
        fingerprint,
        turns.clone(),
    );
    Some(turns)
}

pub(super) fn incremental_opencode_observed_turns(
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
        turns.extend(opencode_observed_turns_from_value(value));
    }
    Some(latest_observed_turns(turns))
}

pub(super) fn opencode_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines
        .iter()
        .find_map(|value| string_field(value, &["sessionID", "sessionId", "id"]))
}

pub(super) fn opencode_observed_turn_from_value(
    value: &Value,
) -> Option<ObservedExternalProviderTurn> {
    let role_text = string_field(value, &["role", "type"]);
    let text = text_from_content(value.get("content").or_else(|| value.get("message"))?)
        .and_then(|text| clean_observed_turn_text(role_text.as_deref(), text))?;
    Some(ObservedExternalProviderTurn {
        role: observed_role(role_text.as_deref())?,
        text,
        provider_turn_id: string_field(value, &["id", "messageID", "messageId", "message_id"]),
        observed_at_ms: string_field(value, &["created", "createdAt", "timestamp"])
            .and_then(|timestamp| parse_timestamp_millis(&timestamp)),
    })
}

pub(super) fn opencode_observed_turns_from_value(
    value: &Value,
) -> Vec<ObservedExternalProviderTurn> {
    if value.get("parts").and_then(Value::as_array).is_some() {
        return opencode_message_observed_turns(value);
    }
    if value
        .get("info")
        .and_then(|info| info.get("parts"))
        .is_some()
    {
        return opencode_message_observed_turns(value);
    }
    opencode_observed_turn_from_value(value)
        .into_iter()
        .collect()
}

pub(super) fn opencode_message_observed_turns(value: &Value) -> Vec<ObservedExternalProviderTurn> {
    let info = value.get("info").unwrap_or(value);
    let role = string_field(info, &["role"]).or_else(|| string_field(value, &["role", "type"]));
    let observed_at_ms = string_field(value, &["created", "createdAt", "timestamp"])
        .or_else(|| string_field(info, &["created", "createdAt", "timestamp"]))
        .and_then(|timestamp| parse_timestamp_millis(&timestamp));
    let message_id = string_field(info, &["id", "messageID", "messageId", "message_id"])
        .or_else(|| string_field(value, &["id", "messageID", "messageId", "message_id"]));
    let parts = value
        .get("parts")
        .or_else(|| info.get("parts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut turns = Vec::new();
    let mut user_parts = Vec::new();
    for (index, part) in parts.enumerate() {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
        match (role.as_deref(), part_type) {
            (Some("user"), "text") => {
                if let Some(text) = part
                    .get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
                {
                    user_parts.push(text.to_string());
                }
            }
            (Some("assistant"), "reasoning") => {
                if let Some(text) = part
                    .get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
                    .and_then(|text| clean_observed_turn_text(Some("reasoning"), text.to_string()))
                {
                    turns.push(ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Reasoning,
                        text,
                        provider_turn_id: opencode_part_turn_id(
                            part,
                            message_id.as_deref(),
                            "reasoning",
                            index,
                        ),
                        observed_at_ms,
                    });
                }
            }
            (Some("assistant"), "tool") => {
                turns.push(ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Tool,
                    text: opencode_tool_text(part),
                    provider_turn_id: opencode_part_turn_id(
                        part,
                        message_id.as_deref(),
                        "tool",
                        index,
                    ),
                    observed_at_ms,
                });
            }
            (Some("assistant"), "text") => {
                if let Some(text) = part
                    .get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
                    .and_then(|text| clean_observed_turn_text(Some("assistant"), text.to_string()))
                {
                    turns.push(ObservedExternalProviderTurn {
                        role: ObservedExternalProviderTurnRole::Assistant,
                        text,
                        provider_turn_id: opencode_part_turn_id(
                            part,
                            message_id.as_deref(),
                            "assistant",
                            index,
                        ),
                        observed_at_ms,
                    });
                }
            }
            (Some("assistant"), _) => {
                turns.push(ObservedExternalProviderTurn {
                    role: ObservedExternalProviderTurnRole::Status,
                    text: opencode_metadata_text(&format!("opencode part {part_type}"), part),
                    provider_turn_id: opencode_part_turn_id(
                        part,
                        message_id.as_deref(),
                        part_type,
                        index,
                    ),
                    observed_at_ms,
                });
            }
            _ => {}
        }
    }
    let user_text = user_parts.join("\n");
    if let Some(text) = clean_observed_turn_text(Some("user"), user_text) {
        turns.insert(
            0,
            ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::User,
                text,
                provider_turn_id: message_id.clone(),
                observed_at_ms,
            },
        );
    }
    if let Some(status) = opencode_message_status_turn(info, message_id.as_deref(), observed_at_ms)
    {
        turns.push(status);
    }
    turns
}

pub(super) fn opencode_tool_text(part: &Value) -> String {
    let state = part.get("state").unwrap_or(&Value::Null);
    let status = string_field(state, &["status"]).unwrap_or_else(|| "updated".to_string());
    compact_json_text(serde_json::json!({
        "id": string_field(part, &["id"]),
        "tool": string_field(part, &["tool"]).unwrap_or_else(|| "tool".to_string()),
        "status": status,
        "title": string_field(state, &["title"]),
        "text": string_field(part, &["text"]),
        "input": state.get("input").cloned().unwrap_or(Value::Null),
        "output": string_field(state, &["output"])
            .or_else(|| state.get("metadata").and_then(|metadata| string_field(metadata, &["output", "stdout"]))),
        "error": string_field(state, &["error"]),
        "raw": string_field(state, &["raw"]),
    }))
}

pub(super) fn opencode_message_status_turn(
    info: &Value,
    message_id: Option<&str>,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    if info.get("tokens").is_none()
        && info.get("model").is_none()
        && info.get("modelID").is_none()
        && info.pointer("/time/completed").is_none()
        && info.get("finish").is_none()
    {
        return None;
    }
    let completed = info.pointer("/time/completed").is_some()
        && info
            .get("finish")
            .and_then(Value::as_str)
            .is_some_and(|finish| finish != "tool-calls" && finish != "unknown");
    Some(ObservedExternalProviderTurn {
        role: ObservedExternalProviderTurnRole::Status,
        text: opencode_metadata_text(
            if completed {
                "opencode message completed"
            } else {
                "opencode message metadata"
            },
            info,
        ),
        provider_turn_id: message_id
            .map(|id| format!("message-status-{id}"))
            .or_else(|| observed_at_ms.map(|ms| format!("message-status-{ms}"))),
        observed_at_ms,
    })
}

pub(super) fn opencode_part_turn_id(
    part: &Value,
    message_id: Option<&str>,
    label: &str,
    index: usize,
) -> Option<String> {
    string_field(part, &["id", "partID", "partId", "part_id"])
        .or_else(|| message_id.map(|id| format!("{label}-{id}-{index}")))
}

pub(super) fn opencode_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
}

pub(super) fn parse_opencode_jsonl(path: &Path) -> Option<ExternalProviderSessionRecord> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(record) = cached_provider_discovery_record_for_path("opencode", path, fingerprint) {
        return Some(record);
    }
    let lines = read_jsonl_values(path);
    let mut provider_session_id = None;
    let mut worktree_path = None;
    let mut created_at_ms = None;
    let mut first_prompt = None;

    for value in lines {
        provider_session_id =
            provider_session_id.or_else(|| string_field(&value, &["sessionID", "sessionId", "id"]));
        worktree_path = worktree_path.or_else(|| string_field(&value, &["cwd", "workspace"]));
        created_at_ms = created_at_ms.or_else(|| {
            string_field(&value, &["created", "createdAt", "timestamp"])
                .and_then(|timestamp| parse_timestamp_millis(&timestamp))
        });
        if first_prompt.is_none() {
            first_prompt = opencode_user_prompt(&value);
        }
    }

    let provider_session_id = provider_session_id.or_else(|| file_stem(path))?;
    let capabilities = observed_capabilities(true);
    let record = record_from_parts(
        "opencode",
        provider_session_id.clone(),
        first_prompt,
        worktree_path,
        created_at_ms,
        fingerprint.modified_at_ms,
        None,
        capabilities,
    );
    remember_provider_discovery_record(
        "opencode",
        &provider_session_id,
        path,
        fingerprint,
        record.clone(),
    );
    Some(record)
}

pub(super) fn discover_opencode_sqlite_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    let db_path = opencode_sqlite_db_path(root);
    let Some(connection) = open_opencode_sqlite(&db_path) else {
        return Vec::new();
    };
    let mut statement = match connection.prepare(
        "select s.id, s.title, s.directory, s.time_created, s.time_updated, \
            (select p.data \
               from message m \
               join part p on p.message_id = m.id \
              where m.session_id = s.id \
                and json_extract(m.data, '$.role') = 'user' \
                and json_extract(p.data, '$.type') = 'text' \
              order by m.time_created asc, m.id asc, p.time_created asc, p.id asc \
              limit 1) as first_user_part \
           from session s \
          order by s.time_updated desc, s.id asc \
          limit ?1",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let rows = match statement.query_map([MAX_PROVIDER_FILES as i64], |row| {
        let provider_session_id: String = row.get(0)?;
        let title: Option<String> = row.get(1)?;
        let directory: Option<String> = row.get(2)?;
        let created_at_ms: Option<i64> = row.get(3)?;
        let updated_at_ms: Option<i64> = row.get(4)?;
        let first_user_part: Option<String> = row.get(5)?;
        Ok((
            provider_session_id,
            title,
            directory,
            created_at_ms.and_then(signed_millis_to_u64),
            updated_at_ms.and_then(signed_millis_to_u64),
            first_user_part,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(Result::ok)
        .map(
            |(
                provider_session_id,
                title,
                directory,
                created_at_ms,
                updated_at_ms,
                first_user_part,
            )| {
                let first_prompt = first_user_part
                    .as_deref()
                    .and_then(opencode_text_from_sqlite_part_data)
                    .or(title);
                record_from_parts(
                    "opencode",
                    provider_session_id.clone(),
                    first_prompt,
                    directory,
                    created_at_ms,
                    updated_at_ms.unwrap_or_else(|| file_modified_ms(&db_path)),
                    None,
                    observed_capabilities(true),
                )
            },
        )
        .collect()
}

pub(super) fn read_opencode_sqlite_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let db_path = opencode_sqlite_db_path(root);
    let Some(connection) = open_opencode_sqlite(&db_path) else {
        return Vec::new();
    };
    let fingerprint =
        opencode_sqlite_session_fingerprint_with_connection(&connection, provider_session_id);
    if let Some(cached) = fingerprint.and_then(|fingerprint| {
        cached_provider_observed_turns("opencode", provider_session_id, &db_path, fingerprint)
    }) {
        return cached;
    }
    let mut statement = match connection.prepare(
        "select message_id, role, message_data, part_id, part_type, part_data, time_created, time_updated \
           from ( \
                select m.id as message_id, json_extract(m.data, '$.role') as role, \
                       m.data as message_data, \
                       p.id as part_id, json_extract(p.data, '$.type') as part_type, \
                       p.data as part_data, p.time_created as time_created, p.time_updated as time_updated \
                  from message m \
                  join part p on p.message_id = m.id \
                 where m.session_id = ?1 \
                 order by p.time_created desc, p.id desc \
                 limit ?2 \
           ) \
          order by time_created asc, part_id asc",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let rows = match statement.query_map(
        rusqlite::params![provider_session_id, MAX_OBSERVED_TURNS as i64],
        |row| {
            let message_id: String = row.get(0)?;
            let role: Option<String> = row.get(1)?;
            let message_data: String = row.get(2)?;
            let part_id: String = row.get(3)?;
            let part_type: Option<String> = row.get(4)?;
            let part_data: String = row.get(5)?;
            let created_at_ms: Option<i64> = row.get(6)?;
            let updated_at_ms: Option<i64> = row.get(7)?;
            Ok((
                message_id,
                role,
                message_data,
                part_id,
                part_type,
                part_data,
                created_at_ms.and_then(signed_millis_to_u64),
                updated_at_ms.and_then(signed_millis_to_u64),
            ))
        },
    ) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    let mut turns = Vec::new();
    let mut status_message_ids = BTreeSet::new();
    let mut status_turns = Vec::new();
    for (
        message_id,
        role,
        message_data,
        part_id,
        part_type,
        part_data,
        created_at_ms,
        updated_at_ms,
    ) in rows.filter_map(Result::ok)
    {
        let observed_at_ms = updated_at_ms.or(created_at_ms);
        if let Some(turn) = opencode_sqlite_part_observed_turn(
            role.as_deref(),
            &message_id,
            &part_id,
            part_type.as_deref(),
            &part_data,
            observed_at_ms,
        ) {
            turns.push(turn);
        }
        if status_message_ids.insert(message_id.clone()) {
            if let Ok(message_info) = serde_json::from_str::<Value>(&message_data) {
                if let Some(status) =
                    opencode_message_status_turn(&message_info, Some(&message_id), observed_at_ms)
                {
                    status_turns.push(status);
                }
            }
        }
    }
    turns.extend(status_turns);
    if let Some(fingerprint) = fingerprint {
        remember_provider_observed_turns(
            "opencode",
            provider_session_id,
            &db_path,
            fingerprint,
            turns.clone(),
        );
    }
    turns
}

pub(super) fn opencode_sqlite_session_fingerprint(
    path: &Path,
    provider_session_id: &str,
) -> Option<ProviderTranscriptFileFingerprint> {
    let connection = open_opencode_sqlite(path)?;
    opencode_sqlite_session_fingerprint_with_connection(&connection, provider_session_id)
}

fn opencode_sqlite_session_fingerprint_with_connection(
    connection: &Connection,
    provider_session_id: &str,
) -> Option<ProviderTranscriptFileFingerprint> {
    connection
        .query_row(
            "select s.time_updated, \
                    (select count(*) from message m where m.session_id = s.id), \
                    (select max(m.time_updated) from message m where m.session_id = s.id), \
                    (select count(*) from part p where p.session_id = s.id), \
                    (select max(p.time_updated) from part p where p.session_id = s.id) \
               from session s \
              where s.id = ?1",
            [provider_session_id],
            |row| {
                let session_updated_at_ms =
                    signed_millis_to_u64(row.get::<_, i64>(0)?).unwrap_or_default();
                let message_count = u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default();
                let message_updated_at_ms =
                    row.get::<_, Option<i64>>(2)?.and_then(signed_millis_to_u64);
                let part_count = u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default();
                let part_updated_at_ms =
                    row.get::<_, Option<i64>>(4)?.and_then(signed_millis_to_u64);
                Ok(ProviderTranscriptFileFingerprint {
                    len: message_count.rotate_left(32) ^ part_count,
                    modified_at_ms: [
                        Some(session_updated_at_ms),
                        message_updated_at_ms,
                        part_updated_at_ms,
                    ]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or_default(),
                })
            },
        )
        .ok()
}

pub(super) fn latest_observed_turns(
    mut turns: Vec<ObservedExternalProviderTurn>,
) -> Vec<ObservedExternalProviderTurn> {
    if turns.len() <= MAX_OBSERVED_TURNS {
        return turns;
    }
    let start = turns.len() - MAX_OBSERVED_TURNS;
    let latest_user_before_tail = turns[..start]
        .iter()
        .rposition(|turn| turn.role == ObservedExternalProviderTurnRole::User)
        .map(|index| turns[index].clone());
    turns.drain(0..start);
    if let Some(latest_user) = latest_user_before_tail {
        turns.insert(0, latest_user);
    }
    turns
}

pub(super) fn open_opencode_sqlite(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

pub(super) fn is_opencode_sqlite_db(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("opencode.db")
}

pub(super) fn opencode_text_from_sqlite_part_data(data: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(data).ok()?;
    text_from_content(&value)
}

pub(super) fn opencode_sqlite_part_observed_turn(
    role: Option<&str>,
    message_id: &str,
    part_id: &str,
    part_type: Option<&str>,
    part_data: &str,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    let part = serde_json::from_str::<Value>(part_data).ok()?;
    let provider_turn_id = if part_id.trim().is_empty() {
        message_id.to_string()
    } else {
        part_id.to_string()
    };
    match (
        role,
        part_type.or_else(|| part.get("type").and_then(Value::as_str)),
    ) {
        (Some("assistant"), Some("reasoning")) => {
            let text = part
                .get("text")
                .or_else(|| part.get("content"))
                .and_then(Value::as_str)
                .and_then(|text| clean_observed_turn_text(Some("reasoning"), text.to_string()))?;
            Some(ObservedExternalProviderTurn {
                role: ObservedExternalProviderTurnRole::Reasoning,
                text,
                provider_turn_id: Some(provider_turn_id),
                observed_at_ms,
            })
        }
        (Some("assistant"), Some("tool")) => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Tool,
            text: opencode_tool_text(&part),
            provider_turn_id: Some(provider_turn_id),
            observed_at_ms,
        }),
        (_, Some("text")) => {
            let role = observed_role(role)?;
            let text = text_from_content(&part)
                .and_then(|text| clean_observed_turn_text(Some(role.as_str()), text))?;
            Some(ObservedExternalProviderTurn {
                role,
                text,
                provider_turn_id: Some(provider_turn_id),
                observed_at_ms,
            })
        }
        (Some("assistant"), Some(part_type)) => Some(ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Status,
            text: opencode_metadata_text(&format!("opencode part {part_type}"), &part),
            provider_turn_id: Some(provider_turn_id),
            observed_at_ms,
        }),
        _ => None,
    }
}
