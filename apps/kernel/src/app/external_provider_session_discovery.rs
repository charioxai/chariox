use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::local::{
    ExternalProviderSessionCapabilities, ExternalProviderSessionMode, ExternalProviderSessionRecord,
};
use crate::session::unix_epoch_ms;

const MAX_PROVIDER_FILES: usize = 1_000;
const MAX_JSONL_LINES: usize = 300;
const MAX_OBSERVED_TURNS: usize = 200;
const MAX_PROMPT_PREVIEW_CHARS: usize = 240;
const MAX_TITLE_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedExternalProviderTurn {
    pub(crate) role: ObservedExternalProviderTurnRole,
    pub(crate) text: String,
    pub(crate) provider_turn_id: Option<String>,
    pub(crate) observed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalProviderSessionDiscoverySignature {
    files: Vec<ExternalProviderSessionFileSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalProviderSessionFileSignature {
    provider: String,
    path: PathBuf,
    len: u64,
    modified_at_ms: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum ObservedExternalProviderTurnRole {
    User,
    Assistant,
}

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

pub(crate) fn external_provider_session_discovery_signature(
    provider_filter: Option<&str>,
) -> ExternalProviderSessionDiscoverySignature {
    let mut files = provider_session_candidate_paths(provider_filter)
        .into_iter()
        .filter_map(|(provider, path)| {
            let metadata = fs::metadata(&path).ok()?;
            Some(ExternalProviderSessionFileSignature {
                provider,
                path,
                len: metadata.len(),
                modified_at_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.len.cmp(&right.len))
            .then_with(|| left.modified_at_ms.cmp(&right.modified_at_ms))
    });
    ExternalProviderSessionDiscoverySignature { files }
}

pub(crate) fn read_external_provider_observed_turns(
    provider: &str,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let turns = match provider {
        "codex" => codex_roots()
            .into_iter()
            .flat_map(|root| read_codex_observed_turns(&root, provider_session_id))
            .collect::<Vec<_>>(),
        "claude" => claude_roots()
            .into_iter()
            .flat_map(|root| read_claude_observed_turns(&root, provider_session_id))
            .collect::<Vec<_>>(),
        "opencode" => opencode_roots()
            .into_iter()
            .flat_map(|root| read_opencode_observed_turns(&root, provider_session_id))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    turns.into_iter().take(MAX_OBSERVED_TURNS).collect()
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
    codex_candidate_paths(root)
        .into_iter()
        .filter_map(|path| parse_codex_transcript(&path))
        .collect()
}

pub(crate) fn discover_claude_external_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    claude_candidate_paths(root)
        .into_iter()
        .filter_map(|path| parse_claude_transcript(&path))
        .collect()
}

pub(crate) fn discover_opencode_external_sessions(
    root: &Path,
) -> Vec<ExternalProviderSessionRecord> {
    let mut sessions = discover_opencode_sqlite_sessions(root);
    sessions.extend(
        opencode_candidate_paths(root)
            .into_iter()
            .filter_map(|path| parse_opencode_session_file(&path)),
    );
    sessions
}

fn provider_session_candidate_paths(provider_filter: Option<&str>) -> Vec<(String, PathBuf)> {
    let mut paths = Vec::new();
    if provider_matches(provider_filter, "codex") {
        for root in codex_roots() {
            paths.extend(
                codex_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("codex".to_string(), path)),
            );
        }
    }
    if provider_matches(provider_filter, "claude") {
        for root in claude_roots() {
            paths.extend(
                claude_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("claude".to_string(), path)),
            );
        }
    }
    if provider_matches(provider_filter, "opencode") {
        for root in opencode_roots() {
            paths.extend(
                opencode_candidate_paths(&root)
                    .into_iter()
                    .map(|path| ("opencode".to_string(), path)),
            );
        }
    }
    paths
}

fn codex_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = jsonl_candidates(&root.join("archived_sessions"), 4);
    candidates.extend(jsonl_candidates(&root.join("sessions"), 4));
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

fn claude_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = jsonl_candidates(&root.join("projects"), 3);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

fn opencode_candidate_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = session_json_candidates(root, 5);
    candidates.extend(opencode_sqlite_signature_paths(root));
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
}

fn opencode_sqlite_db_path(root: &Path) -> PathBuf {
    root.join("opencode.db")
}

fn opencode_sqlite_signature_paths(root: &Path) -> Vec<PathBuf> {
    let db = opencode_sqlite_db_path(root);
    let wal = root.join("opencode.db-wal");
    [db, wal]
        .into_iter()
        .filter(|path| path.is_file())
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

fn read_codex_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let mut candidates = jsonl_candidates(&root.join("archived_sessions"), 4);
    candidates.extend(jsonl_candidates(&root.join("sessions"), 4));
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| codex_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

fn codex_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let lines = read_jsonl_values(path);
    let mut parsed_session_id = None;
    let mut turns = Vec::new();
    for value in &lines {
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(payload) = value.get("payload") {
                parsed_session_id = parsed_session_id
                    .or_else(|| string_field(payload, &["id", "session_id", "sessionId"]));
            }
            continue;
        }
        let payload = value.get("payload").unwrap_or(value);
        let role = payload.get("role").and_then(Value::as_str);
        let Some(role) = observed_role(role) else {
            continue;
        };
        let Some(text) = payload
            .get("content")
            .and_then(text_from_content)
            .and_then(|text| {
                clean_observed_turn_text(payload.get("role").and_then(Value::as_str), text)
            })
        else {
            continue;
        };
        turns.push(ObservedExternalProviderTurn {
            role,
            text,
            provider_turn_id: string_field(payload, &["id", "item_id", "message_id"])
                .or_else(|| string_field(value, &["id", "item_id", "message_id"])),
            observed_at_ms: string_field(value, &["timestamp"])
                .or_else(|| string_field(payload, &["timestamp"]))
                .and_then(|timestamp| parse_timestamp_millis(&timestamp)),
        });
    }
    let parsed_session_id = parsed_session_id.or_else(|| file_stem(path))?;
    (parsed_session_id == provider_session_id).then_some(turns)
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

fn read_claude_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let mut candidates = jsonl_candidates(&root.join("projects"), 3);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| claude_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

fn claude_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let lines = read_jsonl_values(path);
    let mut parsed_session_id = None;
    let mut turns = Vec::new();
    for value in &lines {
        parsed_session_id =
            parsed_session_id.or_else(|| string_field(value, &["sessionId", "session_id"]));
        let role = value.get("type").and_then(Value::as_str);
        let Some(role) = observed_role(role) else {
            continue;
        };
        let message = value.get("message").unwrap_or(value);
        let Some(text) = message
            .get("content")
            .or_else(|| value.get("content"))
            .or_else(|| value.get("message"))
            .and_then(text_from_content)
            .and_then(|text| {
                clean_observed_turn_text(value.get("type").and_then(Value::as_str), text)
            })
        else {
            continue;
        };
        turns.push(ObservedExternalProviderTurn {
            role,
            text,
            provider_turn_id: string_field(value, &["uuid", "id", "message_id"])
                .or_else(|| string_field(message, &["id"])),
            observed_at_ms: string_field(value, &["timestamp"])
                .and_then(|timestamp| parse_timestamp_millis(&timestamp)),
        });
    }
    let parsed_session_id = parsed_session_id.or_else(|| file_stem(path))?;
    (parsed_session_id == provider_session_id).then_some(turns)
}

fn parse_opencode_session_file(path: &Path) -> Option<ExternalProviderSessionRecord> {
    if is_opencode_sqlite_db(path) {
        return None;
    }
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

fn read_opencode_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let sqlite_turns = read_opencode_sqlite_observed_turns(root, provider_session_id);
    if !sqlite_turns.is_empty() {
        return sqlite_turns.into_iter().take(MAX_OBSERVED_TURNS).collect();
    }
    let mut candidates = session_json_candidates(root, 5);
    candidates.truncate(MAX_PROVIDER_FILES);
    candidates
        .into_iter()
        .find_map(|path| opencode_observed_turns_from_path(&path, provider_session_id))
        .unwrap_or_default()
}

fn opencode_observed_turns_from_path(
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
    let messages = value
        .get("messages")
        .or_else(|| value.get("conversation"))
        .or_else(|| value.get("entries"))
        .and_then(Value::as_array)?;
    Some(
        messages
            .iter()
            .filter_map(opencode_observed_turn_from_value)
            .take(MAX_OBSERVED_TURNS)
            .collect(),
    )
}

fn opencode_jsonl_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let lines = read_jsonl_values(path);
    let mut parsed_session_id = None;
    let mut turns = Vec::new();
    for value in &lines {
        parsed_session_id =
            parsed_session_id.or_else(|| string_field(value, &["sessionID", "sessionId", "id"]));
        if let Some(turn) = opencode_observed_turn_from_value(value) {
            turns.push(turn);
        }
    }
    let parsed_session_id = parsed_session_id.or_else(|| file_stem(path))?;
    (parsed_session_id == provider_session_id).then_some(turns)
}

fn opencode_observed_turn_from_value(value: &Value) -> Option<ObservedExternalProviderTurn> {
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

fn discover_opencode_sqlite_sessions(root: &Path) -> Vec<ExternalProviderSessionRecord> {
    let db_path = opencode_sqlite_db_path(root);
    let Some(connection) = open_opencode_sqlite(&db_path) else {
        return Vec::new();
    };
    let mut statement = match connection.prepare(
        "select s.id, s.title, s.directory, s.time_created, s.time_updated, \
            (select p.data \
               from part p \
               join message m on m.id = p.message_id \
              where p.session_id = s.id \
                and json_extract(m.data, '$.role') = 'user' \
                and json_extract(p.data, '$.type') = 'text' \
              order by p.time_created asc, p.id asc \
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
                    provider_session_id,
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

fn read_opencode_sqlite_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let db_path = opencode_sqlite_db_path(root);
    let Some(connection) = open_opencode_sqlite(&db_path) else {
        return Vec::new();
    };
    let mut statement = match connection.prepare(
        "select m.id, json_extract(m.data, '$.role') as role, \
                p.id, json_extract(p.data, '$.type') as part_type, \
                p.data, p.time_created, p.time_updated \
           from message m \
           join part p on p.message_id = m.id \
          where m.session_id = ?1 \
          order by p.time_created asc, p.id asc \
          limit ?2",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let rows = match statement.query_map(
        rusqlite::params![provider_session_id, MAX_OBSERVED_TURNS as i64],
        |row| {
            let message_id: String = row.get(0)?;
            let role: Option<String> = row.get(1)?;
            let part_id: String = row.get(2)?;
            let part_type: Option<String> = row.get(3)?;
            let part_data: String = row.get(4)?;
            let created_at_ms: Option<i64> = row.get(5)?;
            let updated_at_ms: Option<i64> = row.get(6)?;
            Ok((
                message_id,
                role,
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
    rows.filter_map(Result::ok)
        .filter_map(
            |(message_id, role, part_id, part_type, part_data, created_at_ms, updated_at_ms)| {
                if part_type.as_deref() != Some("text") {
                    return None;
                }
                let role = observed_role(role.as_deref())?;
                let text = opencode_text_from_sqlite_part_data(&part_data)
                    .and_then(|text| clean_observed_turn_text(Some(role_text(role)), text))?;
                let provider_turn_id = if part_id.trim().is_empty() {
                    message_id
                } else {
                    part_id
                };
                Some(ObservedExternalProviderTurn {
                    role,
                    text,
                    provider_turn_id: Some(provider_turn_id),
                    observed_at_ms: updated_at_ms.or(created_at_ms),
                })
            },
        )
        .collect()
}

fn open_opencode_sqlite(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn is_opencode_sqlite_db(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("opencode.db")
}

fn opencode_text_from_sqlite_part_data(data: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(data).ok()?;
    text_from_content(&value)
}

fn role_text(role: ObservedExternalProviderTurnRole) -> &'static str {
    match role {
        ObservedExternalProviderTurnRole::User => "user",
        ObservedExternalProviderTurnRole::Assistant => "assistant",
    }
}

fn signed_millis_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
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

fn observed_role(role: Option<&str>) -> Option<ObservedExternalProviderTurnRole> {
    match role {
        Some("user") => Some(ObservedExternalProviderTurnRole::User),
        Some("assistant") => Some(ObservedExternalProviderTurnRole::Assistant),
        _ => None,
    }
}

fn clean_observed_turn_text(role: Option<&str>, text: String) -> Option<String> {
    match observed_role(role)? {
        ObservedExternalProviderTurnRole::User => clean_provider_prompt(text),
        ObservedExternalProviderTurnRole::Assistant => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
    }
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

    #[test]
    fn discovers_opencode_sqlite_sessions() {
        let temp = temp_dir("opencode-sqlite-discovery");
        let root = temp.path();
        let db_path = root.join("opencode.db");
        seed_opencode_sqlite(&db_path);

        let sessions = discover_opencode_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].external_session_id, "opencode:ses_sqlite_1");
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Investigate SQLite-backed OpenCode imports.")
        );
        assert_eq!(sessions[0].worktree_path.as_deref(), Some("/repo/sqlite"));
        assert!(sessions[0].last_modified_at_ms >= 1_782_113_000_000);
    }

    #[test]
    fn reads_codex_observed_user_and_assistant_turns() {
        let temp = temp_dir("codex-observed-turns");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Plan the importer tests.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"a1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Use fixture transcripts.\"}]}}\n",
            ),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-1");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Plan the importer tests.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u1"));
        assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
        assert_eq!(turns[1].text, "Use fixture transcripts.");
    }

    #[test]
    fn reads_claude_observed_user_and_assistant_turns() {
        let temp = temp_dir("claude-observed-turns");
        let root = temp.path();
        let session_dir = root.join("projects").join("-repo");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"text\":\"Summarize external imports.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"text\":\"External imports reuse provider sessions.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
            ),
        )
        .unwrap();

        let turns = read_claude_observed_turns(root, "session-1");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Summarize external imports.");
        assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
        assert_eq!(turns[1].provider_turn_id.as_deref(), Some("a1"));
    }

    #[test]
    fn reads_opencode_observed_json_turns() {
        let temp = temp_dir("opencode-observed-turns");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.json"),
            r#"{"id":"open-1","messages":[{"id":"u1","role":"user","content":"Draft the OpenCode import drill.","createdAt":"2026-03-01T00:00:01.000Z"},{"id":"a1","role":"assistant","content":"Capture the waiting-room evidence.","createdAt":"2026-03-01T00:00:02.000Z"}]}"#,
        )
        .unwrap();

        let turns = read_opencode_observed_turns(root, "open-1");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u1"));
        assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
        assert_eq!(turns[1].text, "Capture the waiting-room evidence.");
    }

    #[test]
    fn reads_opencode_observed_sqlite_turns() {
        let temp = temp_dir("opencode-sqlite-observed-turns");
        let root = temp.path();
        let db_path = root.join("opencode.db");
        seed_opencode_sqlite(&db_path);

        let turns = read_opencode_observed_turns(root, "ses_sqlite_1");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Investigate SQLite-backed OpenCode imports.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("prt_user_text"));
        assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Assistant);
        assert_eq!(turns[1].text, "Use the session, message, and part tables.");
        assert_eq!(
            turns[1].provider_turn_id.as_deref(),
            Some("prt_assistant_text")
        );
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

    fn seed_opencode_sqlite(path: &Path) {
        let connection = Connection::open(path).expect("sqlite fixture should open");
        connection
            .execute_batch(
                r#"
                create table session (
                    id text primary key,
                    project_id text not null,
                    parent_id text,
                    slug text not null,
                    directory text not null,
                    title text not null,
                    version text not null,
                    share_url text,
                    summary_additions integer,
                    summary_deletions integer,
                    summary_files integer,
                    summary_diffs text,
                    revert text,
                    permission text,
                    time_created integer not null,
                    time_updated integer not null,
                    time_compacting integer,
                    time_archived integer,
                    workspace_id text
                );
                create table message (
                    id text primary key,
                    session_id text not null,
                    time_created integer not null,
                    time_updated integer not null,
                    data text not null
                );
                create table part (
                    id text primary key,
                    message_id text not null,
                    session_id text not null,
                    time_created integer not null,
                    time_updated integer not null,
                    data text not null
                );
                insert into session (
                    id, project_id, slug, directory, title, version,
                    time_created, time_updated
                ) values (
                    'ses_sqlite_1', 'project_1', 'sqlite-imports',
                    '/repo/sqlite', 'SQLite OpenCode import', '0.0.0',
                    1782113000000, 1782113050000
                );
                insert into message (
                    id, session_id, time_created, time_updated, data
                ) values (
                    'msg_user', 'ses_sqlite_1', 1782113001000, 1782113001000,
                    '{"role":"user"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_user_text', 'msg_user', 'ses_sqlite_1',
                    1782113001001, 1782113001001,
                    '{"type":"text","text":"Investigate SQLite-backed OpenCode imports."}'
                );
                insert into message (
                    id, session_id, time_created, time_updated, data
                ) values (
                    'msg_assistant', 'ses_sqlite_1', 1782113002000, 1782113003000,
                    '{"role":"assistant"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_reasoning', 'msg_assistant', 'ses_sqlite_1',
                    1782113002001, 1782113002001,
                    '{"type":"reasoning","text":"Internal reasoning"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_assistant_text', 'msg_assistant', 'ses_sqlite_1',
                    1782113003000, 1782113003000,
                    '{"type":"text","text":"Use the session, message, and part tables."}'
                );
                "#,
            )
            .expect("sqlite fixture should seed");
    }
}
