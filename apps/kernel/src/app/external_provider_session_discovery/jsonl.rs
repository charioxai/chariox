use super::*;

pub(super) fn signed_millis_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

#[cfg(test)]
pub(super) fn increment_jsonl_prefix_read_count() {
    JSONL_PREFIX_READ_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
pub(super) fn increment_jsonl_recent_read_count() {
    JSONL_RECENT_READ_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
pub(super) fn increment_jsonl_incremental_read_count() {
    JSONL_INCREMENTAL_READ_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
pub(super) fn increment_file_candidate_scan_count() {
    FILE_CANDIDATE_SCAN_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
pub(super) fn reset_jsonl_read_counts() {
    JSONL_PREFIX_READ_COUNT.with(|counter| counter.set(0));
    JSONL_RECENT_READ_COUNT.with(|counter| counter.set(0));
    JSONL_INCREMENTAL_READ_COUNT.with(|counter| counter.set(0));
}

#[cfg(test)]
pub(super) fn reset_file_candidate_scan_count() {
    FILE_CANDIDATE_SCAN_COUNT.with(|counter| counter.set(0));
}

#[cfg(test)]
pub(super) fn jsonl_prefix_read_count() -> usize {
    JSONL_PREFIX_READ_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(super) fn jsonl_recent_read_count() -> usize {
    JSONL_RECENT_READ_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(super) fn jsonl_incremental_read_count() -> usize {
    JSONL_INCREMENTAL_READ_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(super) fn file_candidate_scan_count() -> usize {
    FILE_CANDIDATE_SCAN_COUNT.with(Cell::get)
}

pub(super) fn read_jsonl_values(path: &Path) -> Vec<Value> {
    #[cfg(test)]
    increment_jsonl_prefix_read_count();

    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .take(MAX_JSONL_LINES)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .collect()
}

pub(super) fn read_recent_jsonl_values(path: &Path) -> Vec<Value> {
    let lines = read_recent_jsonl_lines(path);
    let start = lines.len().saturating_sub(MAX_JSONL_LINES);
    lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line.as_str()).ok())
        .collect()
}

pub(super) fn read_recent_codex_jsonl_values(path: &Path) -> Vec<Value> {
    let lines = read_recent_jsonl_lines(path);
    let start = lines.len().saturating_sub(MAX_JSONL_LINES);
    let anchor = if start > 0 {
        lines[..start].iter().rev().find_map(|line| {
            let value = serde_json::from_str::<Value>(line.as_str()).ok()?;
            let turn = codex_observed_turn_from_value(&value)?;
            (turn.role == ObservedExternalProviderTurnRole::User).then_some(value)
        })
    } else {
        None
    };
    let mut values = Vec::with_capacity(MAX_JSONL_LINES + usize::from(anchor.is_some()));
    if let Some(anchor) = anchor {
        values.push(anchor);
    }
    values.extend(
        lines[start..]
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line.as_str()).ok()),
    );
    values
}

pub(super) fn read_recent_claude_jsonl_values(path: &Path) -> Vec<Value> {
    let lines = read_recent_jsonl_lines(path);
    let start = lines.len().saturating_sub(MAX_JSONL_LINES);
    lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line.as_str()).ok())
        .collect()
}

pub(super) fn read_incremental_jsonl_values(path: &Path, offset: u64) -> Option<Vec<Value>> {
    #[cfg(test)]
    increment_jsonl_incremental_read_count();

    let mut file = fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let file_len = metadata.len();
    if offset > file_len {
        return None;
    }
    let appended_len = file_len.saturating_sub(offset);
    if appended_len > MAX_RECENT_JSONL_TAIL_BYTES {
        return None;
    }
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return None;
    }
    let mut payload = String::new();
    if file.read_to_string(&mut payload).is_err() {
        return None;
    }
    Some(
        payload
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect(),
    )
}

pub(super) fn read_recent_jsonl_lines(path: &Path) -> Vec<String> {
    #[cfg(test)]
    increment_jsonl_recent_read_count();

    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };
    let file_len = metadata.len();
    if file_len == 0 {
        return Vec::new();
    }
    let mut remaining = file_len.min(MAX_RECENT_JSONL_TAIL_BYTES);
    let mut read_from = file_len;
    let mut chunks = Vec::new();
    let mut newline_count = 0usize;
    while remaining > 0 {
        let chunk_len = remaining.min(RECENT_JSONL_TAIL_CHUNK_BYTES);
        read_from = read_from.saturating_sub(chunk_len);
        if file.seek(SeekFrom::Start(read_from)).is_err() {
            return Vec::new();
        }
        let mut chunk = vec![0u8; chunk_len as usize];
        if file.read_exact(&mut chunk).is_err() {
            return Vec::new();
        }
        newline_count += chunk.iter().filter(|byte| **byte == b'\n').count();
        chunks.push(chunk);
        if newline_count > MAX_JSONL_LINES && read_from == 0 {
            break;
        }
        if newline_count > MAX_JSONL_LINES * 2 {
            break;
        }
        remaining -= chunk_len;
    }
    chunks.reverse();
    let mut bytes = chunks.into_iter().flatten().collect::<Vec<_>>();
    if read_from > 0 {
        if let Some(first_newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=first_newline);
        } else {
            bytes.clear();
        }
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::to_string)
        .collect()
}

pub(super) fn codex_user_prompt(value: &Value) -> Option<String> {
    let payload = value.get("payload")?;
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    text_from_content(payload.get("content")?).and_then(clean_provider_prompt)
}

pub(super) fn claude_user_prompt(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("user")
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let message = value.get("message")?;
    text_from_content(message.get("content")?).and_then(clean_provider_prompt)
}

pub(super) fn opencode_user_prompt(value: &Value) -> Option<String> {
    if string_field(value, &["role", "type"]).as_deref() != Some("user") {
        return None;
    }
    text_from_content(value.get("content").or_else(|| value.get("message"))?)
        .and_then(clean_provider_prompt)
}
