use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::local::{
    ExternalProviderSessionCapabilities, ExternalProviderSessionMode, ExternalProviderSessionRecord,
};
use crate::session::unix_epoch_ms;

const MAX_PROVIDER_FILES: usize = 1_000;
const MAX_JSONL_LINES: usize = 300;
const MAX_PROMPT_PREVIEW_CHARS: usize = 240;
const MAX_TITLE_CHARS: usize = 80;

pub(crate) fn discover_external_provider_sessions(
    provider_filter: Option<&str>,
) -> Vec<ExternalProviderSessionRecord> {
    let mut sessions = Vec::new();
    if provider_matches(provider_filter, "codex") {
        for root in codex_roots() {
            sessions.extend(discover_codex_external_sessions(&root));
        }
    }
    if provider_matches(provider_filter, "claude") {
        for root in claude_roots() {
            sessions.extend(discover_claude_external_sessions(&root));
        }
    }
    if provider_matches(provider_filter, "opencode") {
        for root in opencode_roots() {
            sessions.extend(discover_opencode_external_sessions(&root));
        }
    }
    deduplicate_external_sessions(sessions)
}

fn provider_matches(filter: Option<&str>, provider: &str) -> bool {
    filter.map_or(true, |filter| filter == provider)
}

fn codex_roots() -> Vec<PathBuf> {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".codex")))
        .into_iter()
        .collect()
}

fn claude_roots() -> Vec<PathBuf> {
    env::var_os("CLAUDE_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".claude")))
        .into_iter()
        .collect()
}

fn opencode_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("OPENCODE_DATA_HOME").map(PathBuf::from) {
        roots.push(root);
    }
    if let Some(root) = env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        roots.push(root.join("opencode"));
    }
    if let Some(home) = home_dir() {
        roots.push(home.join(".local").join("share").join("opencode"));
        roots.push(home.join(".config").join("opencode"));
    }
    deduplicate_paths(roots)
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub(crate) fn discover_codex_external_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    let mut candidates = jsonl_candidates(&root.join("archived_sessions"), 4);
    candidates.extend(jsonl_candidates(&root.join("sessions"), 4));
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .filter_map(|path| parse_codex_transcript(&path))
        .collect()
}

pub(crate) fn discover_claude_external_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    let mut candidates = jsonl_candidates(&root.join("projects"), 3);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .filter_map(|path| parse_claude_transcript(&path))
        .collect()
}

pub(crate) fn discover_opencode_external_sessions(
    root: &Path,
) -> Vec<ExternalProviderSessionRecord> {
    let mut candidates = session_json_candidates(root, 5);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .filter_map(|path| parse_opencode_session_file(&path))
        .collect()
}

fn jsonl_candidates(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    file_candidates(root, max_depth, &["jsonl"])
}

fn session_json_candidates(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    file_candidates(root, max_depth, &["json", "jsonl"])
        .into_iter()
        .filter(|path| {
            let lower = path.display().to_string().to_ascii_lowercase();
            lower.contains("session")
                || lower.contains("conversation")
                || lower.contains("message")
                || lower.ends_with(".jsonl")
        })
        .collect()
}

fn file_candidates(root: &Path, max_depth: usize, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_file_candidates(root, max_depth, extensions, &mut files);
    files.sort_by(|left, right| {
        file_modified_ms(right)
            .cmp(&file_modified_ms(left))
            .then_with(|| left.cmp(right))
    });
    files
}

fn collect_file_candidates(
    root: &Path,
    depth_remaining: usize,
    extensions: &[&str],
    files: &mut Vec<PathBuf>,
) {
    if depth_remaining == 0 || files.len() >= MAX_PROVIDER_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_PROVIDER_FILES {
            return;
        }
        let path = entry.path();
        if path
            .components()
            .any(|component| component.as_os_str() == "node_modules")
        {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_file_candidates(&path, depth_remaining - 1, extensions, files);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extensions.iter().any(|allowed| *allowed == extension))
        {
            files.push(path);
        }
    }
}

fn parse_codex_transcript(path: &Path) -> Option<ExternalProviderSessionRecord> {
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
    Some(record_from_parts(
        "codex",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        account_profile,
        observed_capabilities(true),
    ))
}

fn parse_claude_transcript(path: &Path) -> Option<ExternalProviderSessionRecord> {
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
    Some(record_from_parts(
        "claude",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        None,
        observed_capabilities(true),
    ))
}

fn parse_opencode_session_file(path: &Path) -> Option<ExternalProviderSessionRecord> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
        return parse_opencode_jsonl(path);
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
        .unwrap_or_else(|| file_modified_ms(path));
    Some(record_from_parts(
        "opencode",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        last_modified_at_ms,
        None,
        observed_capabilities(true),
    ))
}

fn parse_opencode_jsonl(path: &Path) -> Option<ExternalProviderSessionRecord> {
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
    Some(record_from_parts(
        "opencode",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        None,
        observed_capabilities(true),
    ))
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
    let Ok(payload) = fs::read_to_string(path) else {
        return Vec::new();
    };
    payload
        .lines()
        .take(MAX_JSONL_LINES)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn codex_user_prompt(value: &Value) -> Option<String> {
    let payload = value.get("payload")?;
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    text_from_content(payload.get("content")?).and_then(clean_provider_prompt)
}

fn claude_user_prompt(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let message = value.get("message")?;
    text_from_content(message.get("content")?).and_then(clean_provider_prompt)
}

fn opencode_user_prompt(value: &Value) -> Option<String> {
    if string_field(value, &["role", "type"]).as_deref() != Some("user") {
        return None;
    }
    text_from_content(value.get("content").or_else(|| value.get("message"))?)
        .and_then(clean_provider_prompt)
}

fn text_from_content(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .or_else(|| item.get("content"))
                        .or_else(|| item.get("value"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(_) => value
            .get("text")
            .or_else(|| value.get("content"))
            .or_else(|| value.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

fn clean_provider_prompt(prompt: String) -> Option<String> {
    let prompt = prompt.trim();
    if prompt.is_empty()
        || prompt.starts_with("# AGENTS.md instructions")
        || prompt.starts_with("<environment_context>")
        || prompt.starts_with("Native provider execution is enabled")
    {
        return None;
    }
    let prompt = prompt
        .split("## My request for Codex:")
        .last()
        .unwrap_or(prompt)
        .split("## My request:")
        .last()
        .unwrap_or(prompt)
        .trim();
    (!prompt.is_empty()).then(|| compact_whitespace(prompt))
}

fn record_from_parts(
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
        account_profile,
        running_state: None,
        capabilities,
        mode: ExternalProviderSessionMode::Observed,
        already_imported: false,
        imported_session_ids: Vec::new(),
        imported_agent_ids: Vec::new(),
    }
}

fn observed_capabilities(can_read_history: bool) -> ExternalProviderSessionCapabilities {
    ExternalProviderSessionCapabilities {
        can_resume: true,
        can_read_history,
        can_watch_history: false,
        can_attach_live: false,
        can_proxy_permissions: false,
        can_receive_hidden_context: false,
        supports_workspace_live_sync: false,
    }
}

fn first_sentence_title(prompt: &str) -> Option<String> {
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

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
}

fn file_modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_else(unix_epoch_ms)
}

fn parse_timestamp_millis(value: &str) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .or_else(|| parse_rfc3339_millis_utc(value))
}

fn parse_rfc3339_millis_utc(value: &str) -> Option<u64> {
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

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
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

fn deduplicate_external_sessions(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn discovers_codex_jsonl_sessions_with_first_real_prompt_title() {
        let temp = temp_dir("codex-discovery");
        let root = temp.path();
        let session_dir = root.join("archived_sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\",\"model_provider\":\"openai\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /repo\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix the broken JournalView build. It fails on duplicate state.\"}]}}\n",
            ),
        )
        .unwrap();

        let sessions = discover_codex_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].external_session_id, "codex:thread-1");
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Fix the broken JournalView build.")
        );
        assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
        assert_eq!(sessions[0].account_profile.as_deref(), Some("openai"));
        assert!(sessions[0].capabilities.can_resume);
        assert!(sessions[0].capabilities.can_read_history);
    }

    #[test]
    fn discovers_claude_project_transcripts() {
        let temp = temp_dir("claude-discovery");
        let root = temp.path();
        let session_dir = root.join("projects").join("-repo");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"timestamp\":\"2026-02-01T00:00:00.000Z\",\"sessionId\":\"session-1\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"text\":\"Summarize the import plan. Keep it brief.\"}]},\"cwd\":\"/repo\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
            ),
        )
        .unwrap();

        let sessions = discover_claude_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].external_session_id, "claude:session-1");
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Summarize the import plan.")
        );
        assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
    }

    #[test]
    fn discovers_opencode_json_session_exports() {
        let temp = temp_dir("opencode-discovery");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.json"),
            r#"{"id":"open-1","title":"Investigate provider imports","cwd":"/repo","updatedAt":"2026-03-01T00:00:00.000Z"}"#,
        )
        .unwrap();

        let sessions = discover_opencode_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].external_session_id, "opencode:open-1");
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Investigate provider imports")
        );
        assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo"));
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn temp_dir(name: &str) -> TempDir {
        let path = env::temp_dir().join(format!("arroba-{name}-{}", unix_epoch_ms()));
        match fs::create_dir_all(&path) {
            Ok(()) => TempDir { path },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let fallback =
                    env::temp_dir().join(format!("arroba-{name}-{}-fallback", unix_epoch_ms()));
                fs::create_dir_all(&fallback).unwrap();
                TempDir { path: fallback }
            }
            Err(error) => panic!("failed to create temp dir: {error}"),
        }
    }
}
