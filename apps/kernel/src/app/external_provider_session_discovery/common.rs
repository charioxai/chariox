use super::*;

pub(super) fn parse_bounded_json_string_or_raw(value: &str) -> Value {
    if value.chars().count() > MAX_OBSERVED_METADATA_STRING_CHARS {
        return bounded_observed_string_value(value);
    }
    serde_json::from_str(value)
        .map(|value| bounded_observed_metadata_value(&value))
        .unwrap_or_else(|_| Value::String(value.to_string()))
}

pub(super) fn compact_json_text(value: Value) -> String {
    let bounded = bounded_observed_metadata_value(&value);
    let text = serde_json::to_string_pretty(&bounded).unwrap_or_else(|_| bounded.to_string());
    truncate_chars(&text, MAX_OBSERVED_METADATA_TEXT_CHARS)
}

pub(super) fn bounded_observed_metadata_value(value: &Value) -> Value {
    match value {
        Value::String(text) => bounded_observed_string_value(text),
        Value::Array(items) => {
            let mut bounded = items
                .iter()
                .take(MAX_OBSERVED_METADATA_ARRAY_ITEMS)
                .map(bounded_observed_metadata_value)
                .collect::<Vec<_>>();
            if items.len() > MAX_OBSERVED_METADATA_ARRAY_ITEMS {
                bounded.push(serde_json::json!({
                    "__chariox_truncated_items": items.len() - MAX_OBSERVED_METADATA_ARRAY_ITEMS,
                }));
            }
            Value::Array(bounded)
        }
        Value::Object(map) => {
            let mut bounded = serde_json::Map::new();
            for (key, item) in map.iter().take(MAX_OBSERVED_METADATA_OBJECT_FIELDS) {
                bounded.insert(key.clone(), bounded_observed_metadata_value(item));
            }
            if map.len() > MAX_OBSERVED_METADATA_OBJECT_FIELDS {
                bounded.insert(
                    "__chariox_truncated_fields".to_string(),
                    serde_json::json!(map.len() - MAX_OBSERVED_METADATA_OBJECT_FIELDS),
                );
            }
            Value::Object(bounded)
        }
        _ => value.clone(),
    }
}

pub(super) fn bounded_observed_string_value(value: &str) -> Value {
    if value.chars().count() <= MAX_OBSERVED_METADATA_STRING_CHARS {
        return Value::String(value.to_string());
    }
    Value::String(format!(
        "{} [chariox truncated {} chars]",
        truncate_chars(value, MAX_OBSERVED_METADATA_STRING_CHARS),
        value.chars().count() - MAX_OBSERVED_METADATA_STRING_CHARS,
    ))
}

pub(super) fn record_from_parts(
    provider: &str,
    provider_session_id: String,
    first_prompt: Option<String>,
    worktree_path: Option<String>,
    created_at_ms: Option<u64>,
    last_modified_at_ms: u64,
    account_profile: Option<String>,
    capabilities: ExternalProviderSessionCapabilities,
) -> ExternalProviderSessionRecord {
    let title = first_prompt.as_deref().and_then(first_sentence_title);
    let first_prompt_preview =
        first_prompt.map(|prompt| truncate_chars(&prompt, MAX_PROMPT_PREVIEW_CHARS));
    ExternalProviderSessionRecord {
        owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        external_session_id: format!("{provider}:{provider_session_id}"),
        provider: provider.to_string(),
        provider_session_id,
        title: title
            .clone()
            .or_else(|| Some("External session".to_string())),
        title_source: title
            .as_ref()
            .map(|_| "first_prompt".to_string())
            .or_else(|| Some("fallback".to_string())),
        first_prompt_preview,
        created_at_ms,
        last_modified_at_ms,
        worktree_path,
        account_profile: account_profile.unwrap_or_else(|| "default".to_string()),
        capabilities,
        attached_to_chariox: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}

pub(super) fn observed_capabilities(can_read_history: bool) -> ExternalProviderSessionCapabilities {
    ExternalProviderSessionCapabilities { can_read_history }
}

pub(super) fn first_sentence_title(prompt: &str) -> Option<String> {
    let mut title = String::new();
    for character in prompt.chars() {
        if matches!(character, '\n' | '\r') {
            break;
        }
        title.push(character);
        if matches!(character, '.' | '?' | '!') {
            break;
        }
        if title.chars().count() >= MAX_TITLE_CHARS {
            break;
        }
    }
    let title = truncate_chars(title.trim(), MAX_TITLE_CHARS);
    (!title.is_empty()).then_some(title)
}

pub(super) fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

pub(super) fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

pub(super) fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
}

pub(super) fn file_modified_ms(path: &Path) -> u64 {
    provider_transcript_file_fingerprint(path)
        .map(|fingerprint| fingerprint.modified_at_ms)
        .unwrap_or_else(unix_epoch_ms)
}

pub(super) fn provider_transcript_file_fingerprint(
    path: &Path,
) -> Option<ProviderTranscriptFileFingerprint> {
    fs::metadata(path)
        .ok()
        .map(|metadata| ProviderTranscriptFileFingerprint {
            len: metadata.len(),
            modified_at_ms: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
        })
}

pub(super) fn complete_jsonl_fingerprint(
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> Option<ProviderTranscriptFileFingerprint> {
    Some(ProviderTranscriptFileFingerprint {
        len: complete_jsonl_offset(path, fingerprint.len)?,
        modified_at_ms: fingerprint.modified_at_ms,
    })
}

pub(super) fn complete_jsonl_offset(path: &Path, file_len: u64) -> Option<u64> {
    if file_len == 0 {
        return Some(0);
    }
    let mut file = fs::File::open(path).ok()?;
    if file.seek(SeekFrom::Start(file_len - 1)).is_err() {
        return None;
    }
    let mut last = [0u8; 1];
    if file.read_exact(&mut last).is_err() {
        return None;
    }
    if last[0] == b'\n' {
        return Some(file_len);
    }
    let mut remaining = file_len;
    while remaining > 0 {
        let chunk_len = remaining.min(RECENT_JSONL_TAIL_CHUNK_BYTES);
        remaining = remaining.saturating_sub(chunk_len);
        if file.seek(SeekFrom::Start(remaining)).is_err() {
            return None;
        }
        let mut chunk = vec![0u8; chunk_len as usize];
        if file.read_exact(&mut chunk).is_err() {
            return None;
        }
        if let Some(index) = chunk.iter().rposition(|byte| *byte == b'\n') {
            return Some(remaining + index as u64 + 1);
        }
    }
    Some(0)
}

pub(super) fn parse_timestamp_millis(value: &str) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .or_else(|| parse_rfc3339_millis_utc(value))
}

pub(super) fn parse_rfc3339_millis_utc(value: &str) -> Option<u64> {
    let value = value.trim();
    let (date, time) = value.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i32>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    let time = time.trim_end_matches('Z');
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u32>().ok()?;
    let minute = time_parts.next()?.parse::<u32>().ok()?;
    let second_part = time_parts.next()?;
    let mut second_parts = second_part.split('.');
    let second = second_parts.next()?.parse::<u32>().ok()?;
    let millis = second_parts
        .next()
        .map(|fraction| {
            let digits = fraction.chars().take(3).collect::<String>();
            format!("{digits:0<3}").parse::<u32>().ok()
        })
        .unwrap_or(Some(0))?;
    let days = days_from_civil(year, month, day)?;
    Some(
        (((days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64) * 1_000)
            + millis as i64) as u64,
    )
}

pub(super) fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - (month <= 2) as i32;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) as i64)
}

pub(super) fn deduplicate_external_sessions(
    sessions: Vec<ExternalProviderSessionRecord>,
) -> Vec<ExternalProviderSessionRecord> {
    let mut by_id = BTreeMap::<String, ExternalProviderSessionRecord>::new();
    for session in sessions {
        by_id
            .entry(session.external_session_id.clone())
            .and_modify(|existing| {
                if session.last_modified_at_ms > existing.last_modified_at_ms {
                    *existing = session.clone();
                }
            })
            .or_insert(session);
    }
    by_id.into_values().collect()
}
