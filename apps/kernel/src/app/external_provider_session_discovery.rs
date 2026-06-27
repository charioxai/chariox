#[cfg(test)]
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::history::SessionHistoryEntryKind;
use crate::local::{ExternalProviderSessionCapabilities, ExternalProviderSessionRecord};
use crate::session::unix_epoch_ms;

const MAX_PROVIDER_FILES: usize = 1_000;
const MAX_JSONL_LINES: usize = 300;
const MAX_RECENT_JSONL_TAIL_BYTES: u64 = 4 * 1024 * 1024;
const RECENT_JSONL_TAIL_CHUNK_BYTES: u64 = 256 * 1024;
const MAX_OBSERVED_TURNS: usize = 200;
const MAX_OBSERVED_METADATA_STRING_CHARS: usize = 4_000;
const MAX_OBSERVED_METADATA_ARRAY_ITEMS: usize = 40;
const MAX_OBSERVED_METADATA_OBJECT_FIELDS: usize = 80;
const MAX_OBSERVED_METADATA_TEXT_CHARS: usize = 16_000;
const MAX_PROMPT_PREVIEW_CHARS: usize = 240;
const MAX_TITLE_CHARS: usize = 80;

static PROVIDER_TRANSCRIPT_PATH_INDEX: OnceLock<
    Mutex<BTreeMap<String, ExternalProviderTranscriptIndexEntry>>,
> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static JSONL_PREFIX_READ_COUNT: Cell<usize> = const { Cell::new(0) };
    static JSONL_RECENT_READ_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalProviderTranscriptIndexEntry {
    provider_session_id: String,
    path: PathBuf,
    len: u64,
    modified_at_ms: u64,
    last_observed_offset: u64,
    observed_turns: Option<Vec<ObservedExternalProviderTurn>>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ProviderTranscriptFileFingerprint {
    len: u64,
    modified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedExternalProviderTurn {
    pub(crate) role: ObservedExternalProviderTurnRole,
    pub(crate) text: String,
    pub(crate) provider_turn_id: Option<String>,
    pub(crate) observed_at_ms: Option<u64>,
}

impl ObservedExternalProviderTurn {
    pub(crate) fn stable_fallback_id(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.role.hash(&mut hasher);
        self.text.hash(&mut hasher);
        self.observed_at_ms.hash(&mut hasher);
        format!("observed-{}-{:016x}", role_text(self.role), hasher.finish())
    }

    pub(crate) fn provider_turn_id_or_fallback(&self) -> String {
        self.provider_turn_id
            .clone()
            .unwrap_or_else(|| self.stable_fallback_id())
    }

    pub(crate) fn external_merge_key(&self, provider: &str, provider_session_id: &str) -> String {
        crate::history::external_provider_observed_merge_key(
            provider,
            provider_session_id,
            &self.provider_turn_id_or_fallback(),
        )
    }
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ObservedExternalProviderTurnRole {
    User,
    Assistant,
    Reasoning,
    Tool,
    Status,
}

impl ObservedExternalProviderTurnRole {
    pub(crate) fn session_history_kind(self) -> SessionHistoryEntryKind {
        match self {
            Self::User => SessionHistoryEntryKind::UserPrompt,
            Self::Assistant => SessionHistoryEntryKind::ProviderOutput,
            Self::Reasoning => SessionHistoryEntryKind::ProviderReasoning,
            Self::Tool => SessionHistoryEntryKind::ProviderTool,
            Self::Status => SessionHistoryEntryKind::ProviderStatus,
        }
    }
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
    latest_observed_turns(turns)
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

fn provider_transcript_path_index(
) -> &'static Mutex<BTreeMap<String, ExternalProviderTranscriptIndexEntry>> {
    PROVIDER_TRANSCRIPT_PATH_INDEX.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn provider_transcript_path_index_key(provider: &str, provider_session_id: &str) -> String {
    format!("{provider}:{provider_session_id}")
}

fn cached_provider_transcript_path(provider: &str, provider_session_id: &str) -> Option<PathBuf> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let path = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .map(|entry| entry.path.clone())?;
    path.is_file().then_some(path)
}

fn remember_provider_transcript_path(provider: &str, provider_session_id: &str, path: &Path) {
    let Some(fingerprint) = provider_transcript_file_fingerprint(path) else {
        return;
    };
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    if let Ok(mut index) = provider_transcript_path_index().lock() {
        let observed_turns = index.get(&key).and_then(|existing| {
            (existing.path == path
                && existing.len == fingerprint.len
                && existing.modified_at_ms == fingerprint.modified_at_ms)
                .then(|| existing.observed_turns.clone())
                .flatten()
        });
        index.insert(
            key,
            ExternalProviderTranscriptIndexEntry {
                provider_session_id: provider_session_id.to_string(),
                path: path.to_path_buf(),
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                last_observed_offset: observed_turns
                    .as_ref()
                    .map(|_| fingerprint.len)
                    .unwrap_or(0),
                observed_turns,
            },
        );
    }
}

fn cached_provider_observed_turns(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    let entry = provider_transcript_path_index()
        .lock()
        .ok()?
        .get(&key)
        .cloned()?;
    (entry.provider_session_id == provider_session_id
        && entry.path == path
        && entry.len == fingerprint.len
        && entry.modified_at_ms == fingerprint.modified_at_ms)
        .then(|| entry.observed_turns)
        .flatten()
}

fn cached_provider_transcript_identity_matches(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
) -> bool {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    provider_transcript_path_index()
        .lock()
        .ok()
        .and_then(|index| index.get(&key).cloned())
        .is_some_and(|entry| {
            entry.provider_session_id == provider_session_id
                && entry.path == path
                && fingerprint.len >= entry.last_observed_offset
        })
}

fn remember_provider_observed_turns(
    provider: &str,
    provider_session_id: &str,
    path: &Path,
    fingerprint: ProviderTranscriptFileFingerprint,
    turns: Vec<ObservedExternalProviderTurn>,
) {
    let key = provider_transcript_path_index_key(provider, provider_session_id);
    if let Ok(mut index) = provider_transcript_path_index().lock() {
        index.insert(
            key,
            ExternalProviderTranscriptIndexEntry {
                provider_session_id: provider_session_id.to_string(),
                path: path.to_path_buf(),
                len: fingerprint.len,
                modified_at_ms: fingerprint.modified_at_ms,
                last_observed_offset: fingerprint.len,
                observed_turns: Some(turns),
            },
        );
    }
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
    sort_file_candidates_by_recent_modified(&mut candidates);
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
    sort_file_candidates_by_recent_modified(&mut candidates);
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
    sort_file_candidates_by_recent_modified(&mut files);
    files
}

fn sort_file_candidates_by_recent_modified(files: &mut [PathBuf]) {
    let modified_by_path = files
        .iter()
        .map(|path| (path.clone(), file_modified_ms(path)))
        .collect::<BTreeMap<_, _>>();
    files.sort_by(|left, right| {
        modified_by_path
            .get(right)
            .copied()
            .unwrap_or(0)
            .cmp(&modified_by_path.get(left).copied().unwrap_or(0))
            .then_with(|| left.cmp(right))
    });
}

fn collect_file_candidates(
    root: &Path,
    depth_remaining: usize,
    extensions: &[&str],
    files: &mut Vec<PathBuf>,
) {
    if depth_remaining == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
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
    remember_provider_transcript_path("codex", &provider_session_id, path);
    let capabilities = observed_capabilities(true);
    Some(record_from_parts(
        "codex",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        account_profile,
        capabilities,
    ))
}

fn read_codex_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    if let Some(path) = cached_provider_transcript_path("codex", provider_session_id) {
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

fn codex_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(turns) =
        cached_provider_observed_turns("codex", provider_session_id, path, fingerprint)
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
    remember_provider_transcript_path("codex", provider_session_id, path);
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
        fingerprint,
        turns.clone(),
    );
    Some(turns)
}

fn codex_observed_turn_from_value(value: &Value) -> Option<ObservedExternalProviderTurn> {
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

fn codex_response_item_observed_turn(
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

fn codex_event_message_observed_turn(
    payload: &Value,
    observed_at_ms: Option<u64>,
) -> Option<ObservedExternalProviderTurn> {
    let event_type = payload.get("type").and_then(Value::as_str)?;
    match event_type {
        "user_message" => None,
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

fn deduplicate_codex_mirrored_turns(
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

fn codex_mirrored_visible_message(
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

fn observed_timestamps_are_close(left: Option<u64>, right: Option<u64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= 2_000,
        _ => false,
    }
}

fn codex_turn_identity_is_richer(
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

fn codex_tool_turn_id(payload: &Value) -> Option<String> {
    let item_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    string_field(payload, &["id", "item_id", "message_id"])
        .or_else(|| string_field(payload, &["call_id"]))
        .map(|id| format!("{item_type}-{id}"))
}

fn codex_tool_text(payload: &Value) -> String {
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

fn codex_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
}

fn codex_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines.iter().find_map(|value| {
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            return None;
        }
        value
            .get("payload")
            .and_then(|payload| string_field(payload, &["id", "session_id", "sessionId"]))
    })
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
    remember_provider_transcript_path("claude", &provider_session_id, path);
    let capabilities = observed_capabilities(true);
    Some(record_from_parts(
        "claude",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        None,
        capabilities,
    ))
}

fn read_claude_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    if let Some(path) = cached_provider_transcript_path("claude", provider_session_id) {
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

fn claude_observed_turns_from_path(
    path: &Path,
    provider_session_id: &str,
) -> Option<Vec<ObservedExternalProviderTurn>> {
    let fingerprint = provider_transcript_file_fingerprint(path)?;
    if let Some(turns) =
        cached_provider_observed_turns("claude", provider_session_id, path, fingerprint)
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
    remember_provider_transcript_path("claude", provider_session_id, path);
    let lines = read_recent_claude_jsonl_values(path);
    let mut turns = Vec::new();
    for value in &lines {
        turns.extend(claude_observed_turns_from_value(value));
    }
    let turns = latest_observed_turns(turns);
    remember_provider_observed_turns(
        "claude",
        provider_session_id,
        path,
        fingerprint,
        turns.clone(),
    );
    Some(turns)
}

fn claude_observed_turns_from_value(value: &Value) -> Vec<ObservedExternalProviderTurn> {
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

fn claude_user_observed_turns(
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

fn claude_assistant_observed_turns(
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

fn claude_assistant_completion_status(
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

fn claude_tool_use_text(item: &Value) -> String {
    let name = string_field(item, &["name"]).unwrap_or_else(|| "tool_use".to_string());
    compact_json_text(serde_json::json!({
        "tool": name,
        "status": "called",
        "id": string_field(item, &["id"]),
        "input": item.get("input").cloned().unwrap_or(Value::Null),
    }))
}

fn claude_tool_result_text(item: &Value) -> Option<String> {
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

fn claude_record_turn_id(
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

fn claude_content_turn_id(
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

fn claude_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
}

fn claude_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines
        .iter()
        .find_map(|value| string_field(value, &["sessionId", "session_id"]))
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
    remember_provider_transcript_path("opencode", &provider_session_id, path);
    let title = string_field(&value, &["title", "name"]);
    let first_prompt = title.clone().or_else(|| opencode_user_prompt(&value));
    let worktree_path = string_field(&value, &["cwd", "path", "workspace"]);
    let created_at_ms = string_field(&value, &["created", "createdAt", "timeCreated"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp));
    let last_modified_at_ms = string_field(&value, &["updated", "updatedAt", "timeUpdated"])
        .and_then(|timestamp| parse_timestamp_millis(&timestamp))
        .unwrap_or_else(|| file_modified_ms(path));
    let capabilities = observed_capabilities(true);
    Some(record_from_parts(
        "opencode",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        last_modified_at_ms,
        None,
        capabilities,
    ))
}

fn read_opencode_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let sqlite_turns = read_opencode_sqlite_observed_turns(root, provider_session_id);
    if !sqlite_turns.is_empty() {
        return latest_observed_turns(sqlite_turns);
    }
    if let Some(path) = cached_provider_transcript_path("opencode", provider_session_id) {
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

fn opencode_jsonl_observed_turns_from_path(
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
    remember_provider_transcript_path("opencode", provider_session_id, path);
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

fn opencode_session_id_from_values(lines: &[Value]) -> Option<String> {
    lines
        .iter()
        .find_map(|value| string_field(value, &["sessionID", "sessionId", "id"]))
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

fn opencode_observed_turns_from_value(value: &Value) -> Vec<ObservedExternalProviderTurn> {
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

fn opencode_message_observed_turns(value: &Value) -> Vec<ObservedExternalProviderTurn> {
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

fn opencode_tool_text(part: &Value) -> String {
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

fn opencode_message_status_turn(
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

fn opencode_part_turn_id(
    part: &Value,
    message_id: Option<&str>,
    label: &str,
    index: usize,
) -> Option<String> {
    string_field(part, &["id", "partID", "partId", "part_id"])
        .or_else(|| message_id.map(|id| format!("{label}-{id}-{index}")))
}

fn opencode_metadata_text(label: &str, payload: &Value) -> String {
    format!("{label}\n{}", compact_json_text(payload.clone()))
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
    let capabilities = observed_capabilities(true);
    Some(record_from_parts(
        "opencode",
        provider_session_id,
        first_prompt,
        worktree_path,
        created_at_ms,
        file_modified_ms(path),
        None,
        capabilities,
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

fn read_opencode_sqlite_observed_turns(
    root: &Path,
    provider_session_id: &str,
) -> Vec<ObservedExternalProviderTurn> {
    let db_path = opencode_sqlite_db_path(root);
    let Some(connection) = open_opencode_sqlite(&db_path) else {
        return Vec::new();
    };
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
    turns
}

fn latest_observed_turns(
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

fn opencode_sqlite_part_observed_turn(
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
                .and_then(|text| clean_observed_turn_text(Some(role_text(role)), text))?;
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

fn role_text(role: ObservedExternalProviderTurnRole) -> &'static str {
    match role {
        ObservedExternalProviderTurnRole::User => "user",
        ObservedExternalProviderTurnRole::Assistant => "assistant",
        ObservedExternalProviderTurnRole::Reasoning => "reasoning",
        ObservedExternalProviderTurnRole::Tool => "tool",
        ObservedExternalProviderTurnRole::Status => "status",
    }
}

fn signed_millis_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

#[cfg(test)]
fn increment_jsonl_prefix_read_count() {
    JSONL_PREFIX_READ_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
fn increment_jsonl_recent_read_count() {
    JSONL_RECENT_READ_COUNT.with(|counter| counter.set(counter.get() + 1));
}

#[cfg(test)]
fn reset_jsonl_read_counts() {
    JSONL_PREFIX_READ_COUNT.with(|counter| counter.set(0));
    JSONL_RECENT_READ_COUNT.with(|counter| counter.set(0));
}

#[cfg(test)]
fn jsonl_prefix_read_count() -> usize {
    JSONL_PREFIX_READ_COUNT.with(Cell::get)
}

#[cfg(test)]
fn jsonl_recent_read_count() -> usize {
    JSONL_RECENT_READ_COUNT.with(Cell::get)
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
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

fn read_recent_jsonl_values(path: &Path) -> Vec<Value> {
    let lines = read_recent_jsonl_lines(path);
    let start = lines.len().saturating_sub(MAX_JSONL_LINES);
    lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line.as_str()).ok())
        .collect()
}

fn read_recent_codex_jsonl_values(path: &Path) -> Vec<Value> {
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

fn read_recent_claude_jsonl_values(path: &Path) -> Vec<Value> {
    let lines = read_recent_jsonl_lines(path);
    let start = lines.len().saturating_sub(MAX_JSONL_LINES);
    let recent = lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line.as_str()).ok())
        .collect::<Vec<_>>();
    let anchor = read_jsonl_values(path).into_iter().rev().find(|value| {
        claude_observed_turns_from_value(value)
            .into_iter()
            .any(|turn| turn.role == ObservedExternalProviderTurnRole::User)
    });
    let Some(anchor) = anchor else {
        return recent;
    };
    let anchor_id = claude_record_identity(&anchor);
    let already_in_recent = anchor_id.as_ref().is_some_and(|anchor_id| {
        recent
            .iter()
            .filter_map(claude_record_identity)
            .any(|recent_id| &recent_id == anchor_id)
    });
    if already_in_recent {
        return recent;
    }
    let mut values = Vec::with_capacity(recent.len() + 1);
    values.push(anchor);
    values.extend(recent);
    values
}

fn claude_record_identity(value: &Value) -> Option<String> {
    string_field(value, &["uuid", "id", "message_id"])
        .or_else(|| string_field(value.get("message").unwrap_or(value), &["id"]))
        .or_else(|| {
            string_field(value, &["sessionId", "session_id"]).and_then(|session_id| {
                let timestamp = string_field(value, &["timestamp"])?;
                let record_type = string_field(value, &["type"])?;
                Some(format!("{session_id}:{record_type}:{timestamp}"))
            })
        })
}

fn read_recent_jsonl_lines(path: &Path) -> Vec<String> {
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
        Some("reasoning") => Some(ObservedExternalProviderTurnRole::Reasoning),
        Some("tool") => Some(ObservedExternalProviderTurnRole::Tool),
        Some("status") => Some(ObservedExternalProviderTurnRole::Status),
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
        ObservedExternalProviderTurnRole::Reasoning
        | ObservedExternalProviderTurnRole::Tool
        | ObservedExternalProviderTurnRole::Status => {
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

fn parse_bounded_json_string_or_raw(value: &str) -> Value {
    if value.chars().count() > MAX_OBSERVED_METADATA_STRING_CHARS {
        return bounded_observed_string_value(value);
    }
    serde_json::from_str(value)
        .map(|value| bounded_observed_metadata_value(&value))
        .unwrap_or_else(|_| Value::String(value.to_string()))
}

fn compact_json_text(value: Value) -> String {
    let bounded = bounded_observed_metadata_value(&value);
    let text = serde_json::to_string_pretty(&bounded).unwrap_or_else(|_| bounded.to_string());
    truncate_chars(&text, MAX_OBSERVED_METADATA_TEXT_CHARS)
}

fn bounded_observed_metadata_value(value: &Value) -> Value {
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
                    "__arroba_truncated_items": items.len() - MAX_OBSERVED_METADATA_ARRAY_ITEMS,
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
                    "__arroba_truncated_fields".to_string(),
                    serde_json::json!(map.len() - MAX_OBSERVED_METADATA_OBJECT_FIELDS),
                );
            }
            Value::Object(bounded)
        }
        _ => value.clone(),
    }
}

fn bounded_observed_string_value(value: &str) -> Value {
    if value.chars().count() <= MAX_OBSERVED_METADATA_STRING_CHARS {
        return Value::String(value.to_string());
    }
    Value::String(format!(
        "{} [arroba truncated {} chars]",
        truncate_chars(value, MAX_OBSERVED_METADATA_STRING_CHARS),
        value.chars().count() - MAX_OBSERVED_METADATA_STRING_CHARS,
    ))
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
        capabilities,
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}

fn observed_capabilities(can_read_history: bool) -> ExternalProviderSessionCapabilities {
    ExternalProviderSessionCapabilities { can_read_history }
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
    provider_transcript_file_fingerprint(path)
        .map(|fingerprint| fingerprint.modified_at_ms)
        .unwrap_or_else(unix_epoch_ms)
}

fn provider_transcript_file_fingerprint(path: &Path) -> Option<ProviderTranscriptFileFingerprint> {
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
    use std::fs::OpenOptions;
    use std::io::{self, Write};

    #[test]
    fn observed_turn_model_derives_history_kind_and_external_keys() {
        let user = ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::User,
            text: "external prompt".to_string(),
            provider_turn_id: Some("provider-user-1".to_string()),
            observed_at_ms: Some(1_000),
        };
        assert_eq!(
            user.role.session_history_kind(),
            SessionHistoryEntryKind::UserPrompt
        );
        assert_eq!(user.provider_turn_id_or_fallback(), "provider-user-1");
        assert_eq!(
            user.external_merge_key("codex", "thread-1"),
            "external:codex:thread-1:provider-user-1"
        );

        let tool = ObservedExternalProviderTurn {
            role: ObservedExternalProviderTurnRole::Tool,
            text: "tool output".to_string(),
            provider_turn_id: None,
            observed_at_ms: Some(1_100),
        };
        assert_eq!(
            tool.role.session_history_kind(),
            SessionHistoryEntryKind::ProviderTool
        );
        let fallback_id = tool.provider_turn_id_or_fallback();
        assert!(fallback_id.starts_with("observed-tool-"));
        assert_eq!(
            tool.external_merge_key("claude", "thread-2"),
            format!("external:claude:thread-2:{fallback_id}")
        );

        assert_eq!(
            ObservedExternalProviderTurnRole::Assistant.session_history_kind(),
            SessionHistoryEntryKind::ProviderOutput
        );
        assert_eq!(
            ObservedExternalProviderTurnRole::Reasoning.session_history_kind(),
            SessionHistoryEntryKind::ProviderReasoning
        );
        assert_eq!(
            ObservedExternalProviderTurnRole::Status.session_history_kind(),
            SessionHistoryEntryKind::ProviderStatus
        );
    }

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
        assert!(sessions[0].capabilities.can_read_history);
    }

    #[test]
    fn file_candidate_collection_does_not_cap_before_recent_sort() {
        let temp = temp_dir("provider-file-cap");
        let root = temp.path();
        for index in 0..=MAX_PROVIDER_FILES {
            fs::write(root.join(format!("session-{index}.jsonl")), "{}\n").unwrap();
        }

        let candidates = jsonl_candidates(root, 1);

        assert_eq!(candidates.len(), MAX_PROVIDER_FILES + 1);
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
    fn reads_codex_observed_reasoning_tools_and_status_metadata() {
        let temp = temp_dir("codex-observed-metadata");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\",\"model_context_window\":258400}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Create, inspect, and delete a file.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:03.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"r1\",\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"Planning file changes.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:04.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"printf alpha > drill.txt\\\"}\",\"call_id\":\"call-create\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:05.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call-create\",\"output\":\"created\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:06.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"a1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:07.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":1,\"output_tokens\":2}}}}\n",
            ),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-1");
        assert_eq!(
            turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
            vec![
                ObservedExternalProviderTurnRole::Status,
                ObservedExternalProviderTurnRole::User,
                ObservedExternalProviderTurnRole::Reasoning,
                ObservedExternalProviderTurnRole::Tool,
                ObservedExternalProviderTurnRole::Tool,
                ObservedExternalProviderTurnRole::Assistant,
                ObservedExternalProviderTurnRole::Status,
            ]
        );
        assert!(turns[0].text.contains("codex task_started"));
        assert!(turns[2].text.contains("Planning file changes."));
        assert!(turns[3].text.contains("exec_command"));
        assert!(turns[3].text.contains("printf alpha > drill.txt"));
        assert!(turns[4].text.contains("created"));
        assert!(turns[6].text.contains("total_token_usage"));
    }

    #[test]
    fn reads_codex_observed_metadata_bounds_large_tool_payloads() {
        let temp = temp_dir("codex-observed-bounded-metadata");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        let large_output = "x".repeat(MAX_OBSERVED_METADATA_TEXT_CHARS * 2);
        let line = serde_json::json!({
            "timestamp": "2026-01-01T00:00:05.000Z",
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call-large",
                "output": large_output,
            }
        });
        fs::write(
            session_dir.join("rollout.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "timestamp": "2026-01-01T00:00:00.000Z",
                    "type": "session_meta",
                    "payload": {"id": "thread-large", "cwd": "/repo"},
                }),
                line
            ),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-large");

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::Tool);
        assert!(turns[0].text.len() <= MAX_OBSERVED_METADATA_TEXT_CHARS + 3);
        assert!(turns[0].text.contains("arroba truncated"));
        assert!(turns[0].text.contains("call-large"));
    }

    #[test]
    fn reads_codex_observed_turns_from_recent_jsonl_tail() {
        let temp = temp_dir("codex-observed-tail");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-tail\",\"cwd\":\"/repo\"}}".to_string(),
        ];
        for index in 0..320 {
            lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"noise-{index}\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"noise {index}\"}}]}}}}"
            ));
        }
        lines.push("{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-tail\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Latest external prompt.\"}]}}".to_string());
        fs::write(
            session_dir.join("rollout.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-tail");
        assert_eq!(turns.len(), MAX_OBSERVED_TURNS);
        assert_eq!(
            turns.last().map(|turn| turn.text.as_str()),
            Some("Latest external prompt.")
        );
        assert_eq!(
            turns
                .last()
                .and_then(|turn| turn.provider_turn_id.as_deref()),
            Some("u-tail")
        );
    }

    #[test]
    fn reads_codex_observed_turns_from_indexed_path_before_candidate_scan() {
        let temp = temp_dir("codex-observed-index");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("indexed-target.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-indexed\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"indexed-user\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Indexed external prompt.\"}]}}\n",
            ),
        )
        .unwrap();

        let sessions = discover_codex_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].external_session_id, "codex:thread-indexed");

        for index in 0..=MAX_PROVIDER_FILES {
            fs::write(
                session_dir.join(format!("newer-decoy-{index}.jsonl")),
                format!(
                    "{{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"decoy-{index}\"}}}}\n"
                ),
            )
            .unwrap();
        }

        let turns = read_codex_observed_turns(root, "thread-indexed");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text, "Indexed external prompt.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("indexed-user"));
    }

    #[test]
    fn reads_codex_observed_turns_from_unchanged_index_without_jsonl_reads() {
        let temp = temp_dir("codex-observed-unchanged-index");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        let transcript = session_dir.join("indexed-unchanged.jsonl");
        fs::write(
            &transcript,
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-unchanged-index\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"indexed-user\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Indexed prompt before cache.\"}]}}\n",
            ),
        )
        .unwrap();

        let sessions = discover_codex_external_sessions(root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].external_session_id,
            "codex:thread-unchanged-index"
        );

        reset_jsonl_read_counts();
        let first = read_codex_observed_turns(root, "thread-unchanged-index");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].text, "Indexed prompt before cache.");
        assert_eq!(jsonl_prefix_read_count(), 0);
        assert_eq!(jsonl_recent_read_count(), 1);

        reset_jsonl_read_counts();
        let unchanged = read_codex_observed_turns(root, "thread-unchanged-index");
        assert_eq!(unchanged, first);
        assert_eq!(jsonl_prefix_read_count(), 0);
        assert_eq!(jsonl_recent_read_count(), 0);

        let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
        writeln!(
            file,
            "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"response_item\",\"payload\":{{\"id\":\"indexed-assistant\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"Indexed assistant after append.\"}}]}}}}"
        )
        .unwrap();

        reset_jsonl_read_counts();
        let appended = read_codex_observed_turns(root, "thread-unchanged-index");
        assert_eq!(jsonl_prefix_read_count(), 0);
        assert_eq!(jsonl_recent_read_count(), 1);
        assert!(appended
            .iter()
            .any(|turn| turn.text == "Indexed assistant after append."));
    }

    #[test]
    fn reads_codex_observed_turns_preserves_latest_user_before_recent_tail() {
        let temp = temp_dir("codex-observed-tail-user");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-tail-user\",\"cwd\":\"/repo\"}}".to_string(),
            "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-active\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Long running external prompt.\"}]}}".to_string(),
        ];
        for index in 0..MAX_OBSERVED_TURNS + 25 {
            lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"total_tokens\":{index}}}}}}}}}"
            ));
        }
        fs::write(
            session_dir.join("rollout.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-tail-user");
        assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Long running external prompt.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-active"));
        assert!(turns[1..]
            .iter()
            .all(|turn| turn.role == ObservedExternalProviderTurnRole::Status));
    }

    #[test]
    fn reads_codex_observed_turns_preserves_latest_user_before_recent_jsonl_window() {
        let temp = temp_dir("codex-observed-window-user");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        let mut lines = vec![
            "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-window-user\",\"cwd\":\"/repo\"}}".to_string(),
            "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u-window\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Very long external prompt turn.\"}]}}".to_string(),
        ];
        for index in 0..MAX_JSONL_LINES + 25 {
            lines.push(format!(
                "{{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"total_tokens\":{index}}}}}}}}}"
            ));
        }
        fs::write(
            session_dir.join("rollout.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-window-user");
        assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Very long external prompt turn.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-window"));
        assert!(turns[1..]
            .iter()
            .all(|turn| turn.role == ObservedExternalProviderTurnRole::Status));
    }

    #[test]
    fn reads_codex_observed_turns_deduplicates_mirrored_visible_events() {
        let temp = temp_dir("codex-observed-mirror-dedupe");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"/repo\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"u1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Run a drill.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:01.001Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Run a drill.\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"The drill passed.\"}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:02.001Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"msg_rich\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"The drill passed.\"}]}}\n",
                "{\"timestamp\":\"2026-01-01T00:00:03.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}}\n",
            ),
        )
        .unwrap();

        let turns = read_codex_observed_turns(root, "thread-1");
        assert_eq!(
            turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
            vec![
                ObservedExternalProviderTurnRole::User,
                ObservedExternalProviderTurnRole::Assistant,
                ObservedExternalProviderTurnRole::Status,
            ]
        );
        assert_eq!(turns[1].text, "The drill passed.");
        assert_eq!(turns[1].provider_turn_id.as_deref(), Some("msg_rich"));
        assert!(turns[2].text.contains("total_token_usage"));
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
    fn reads_claude_observed_turns_preserves_latest_user_before_recent_jsonl_window() {
        let temp = temp_dir("claude-observed-window-user");
        let root = temp.path();
        let session_dir = root.join("projects").join("-repo");
        fs::create_dir_all(&session_dir).unwrap();
        let mut lines = vec![
            "{\"type\":\"user\",\"uuid\":\"u-window\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Run a long Claude external drill.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}".to_string(),
        ];
        for index in 0..MAX_JSONL_LINES + 25 {
            lines.push(format!(
                "{{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.{index:03}Z\"}}"
            ));
        }
        lines.push(
            "{\"type\":\"assistant\",\"uuid\":\"a-final\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"FINAL_EXTERNAL_PARITY_SUMMARY\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:03.000Z\"}".to_string(),
        );
        fs::write(
            session_dir.join("session-1.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();

        let turns = read_claude_observed_turns(root, "session-1");
        assert_eq!(turns.len(), MAX_OBSERVED_TURNS + 1);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Run a long Claude external drill.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("u-window"));
        assert!(turns
            .iter()
            .any(|turn| turn.text == "FINAL_EXTERNAL_PARITY_SUMMARY"));
        assert!(turns
            .iter()
            .any(|turn| turn.text.starts_with("claude message completed")));
    }

    #[test]
    fn reads_claude_end_turn_as_completion_status() {
        let temp = temp_dir("claude-observed-completion");
        let root = temp.path();
        let session_dir = root.join("projects").join("-repo");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Run a drill.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"role\":\"assistant\",\"stop_reason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"FINAL_EXTERNAL_PARITY_SUMMARY\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
                "{\"type\":\"last-prompt\",\"sessionId\":\"session-1\",\"leafUuid\":\"a1\"}\n",
            ),
        )
        .unwrap();

        let turns = read_claude_observed_turns(root, "session-1");
        assert_eq!(
            turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
            vec![
                ObservedExternalProviderTurnRole::User,
                ObservedExternalProviderTurnRole::Assistant,
                ObservedExternalProviderTurnRole::Status,
                ObservedExternalProviderTurnRole::Status,
            ]
        );
        assert_eq!(turns[1].text, "FINAL_EXTERNAL_PARITY_SUMMARY");
        assert_eq!(turns[2].provider_turn_id.as_deref(), Some("a1:completed"));
        assert!(turns[2].text.starts_with("claude message completed"));
        assert!(turns[2].text.contains("stop_reason"));
        assert!(turns[2].text.contains("end_turn"));
        assert_eq!(
            turns[3].provider_turn_id.as_deref(),
            Some("last-prompt-leaf-a1")
        );
        assert!(turns[3].text.starts_with("claude last-prompt"));
    }

    #[test]
    fn reads_claude_observed_reasoning_tools_and_status_metadata() {
        let temp = temp_dir("claude-observed-metadata");
        let root = temp.path();
        let session_dir = root.join("projects").join("-repo");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.jsonl"),
            concat!(
                "{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:00.000Z\"}\n",
                "{\"type\":\"permission-mode\",\"permissionMode\":\"acceptEdits\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:00.500Z\"}\n",
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Create, inspect, and delete a file.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:01.000Z\"}\n",
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"Planning file changes.\"},{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{\"command\":\"printf alpha > drill.txt\"}},{\"type\":\"text\",\"text\":\"I will inspect the file next.\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:02.000Z\"}\n",
                "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"created\"}]},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:03.000Z\"}\n",
                "{\"type\":\"last-prompt\",\"prompt\":\"Create, inspect, and delete a file.\",\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:04.000Z\"}\n",
                "{\"type\":\"file-history-snapshot\",\"snapshot\":{\"large\":true},\"sessionId\":\"session-1\",\"timestamp\":\"2026-02-01T00:00:05.000Z\"}\n",
            ),
        )
        .unwrap();

        let turns = read_claude_observed_turns(root, "session-1");
        assert_eq!(
            turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
            vec![
                ObservedExternalProviderTurnRole::Status,
                ObservedExternalProviderTurnRole::Status,
                ObservedExternalProviderTurnRole::User,
                ObservedExternalProviderTurnRole::Reasoning,
                ObservedExternalProviderTurnRole::Tool,
                ObservedExternalProviderTurnRole::Assistant,
                ObservedExternalProviderTurnRole::Tool,
                ObservedExternalProviderTurnRole::Status,
            ]
        );
        assert!(turns[0].text.contains("claude mode"));
        assert!(turns[1].text.contains("permissionMode"));
        assert_eq!(turns[2].text, "Create, inspect, and delete a file.");
        assert_eq!(turns[3].text, "Planning file changes.");
        assert!(turns[4].text.contains("Bash"));
        assert!(turns[4].text.contains("printf alpha > drill.txt"));
        assert_eq!(turns[5].text, "I will inspect the file next.");
        assert!(turns[6].text.contains("tool_result"));
        assert!(turns[6].text.contains("created"));
        assert!(turns[7].text.contains("claude last-prompt"));
        assert!(turns
            .iter()
            .all(|turn| !turn.text.contains("file-history-snapshot")));
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
    fn reads_opencode_observed_message_parts_and_completion_metadata() {
        let temp = temp_dir("opencode-observed-parts");
        let root = temp.path();
        let session_dir = root.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session-1.json"),
            r#"{
              "id": "open-1",
              "messages": [
                {
                  "info": { "id": "msg-user", "sessionID": "open-1", "role": "user" },
                  "parts": [{ "id": "part-user", "type": "text", "text": "Create, inspect, and delete a file." }]
                },
                {
                  "info": {
                    "id": "msg-assistant",
                    "sessionID": "open-1",
                    "role": "assistant",
                    "providerID": "moonshot",
                    "modelID": "kimi-k2-6",
                    "finish": "stop",
                    "tokens": { "input": 10, "output": 5, "reasoning": 2 },
                    "time": { "completed": 1782113000000 }
                  },
                  "parts": [
                    { "id": "part-reasoning", "type": "reasoning", "text": "Planning file changes." },
                    {
                      "id": "part-tool",
                      "type": "tool",
                      "tool": "bash",
                      "state": {
                        "status": "completed",
                        "input": { "command": "printf alpha > drill.txt" },
                        "output": "created"
                      }
                    },
                    { "id": "part-answer", "type": "text", "text": "Done." }
                  ]
                }
              ]
            }"#,
        )
        .unwrap();

        let turns = read_opencode_observed_turns(root, "open-1");
        assert_eq!(
            turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
            vec![
                ObservedExternalProviderTurnRole::User,
                ObservedExternalProviderTurnRole::Reasoning,
                ObservedExternalProviderTurnRole::Tool,
                ObservedExternalProviderTurnRole::Assistant,
                ObservedExternalProviderTurnRole::Status,
            ]
        );
        assert_eq!(turns[0].text, "Create, inspect, and delete a file.");
        assert_eq!(turns[1].text, "Planning file changes.");
        assert!(turns[2].text.contains("bash"));
        assert!(turns[2].text.contains("printf alpha > drill.txt"));
        assert!(turns[2].text.contains("created"));
        assert_eq!(turns[3].text, "Done.");
        assert!(turns[4].text.contains("opencode message completed"));
        assert!(turns[4].text.contains("kimi-k2-6"));
    }

    #[test]
    fn reads_opencode_observed_sqlite_turns() {
        let temp = temp_dir("opencode-sqlite-observed-turns");
        let root = temp.path();
        let db_path = root.join("opencode.db");
        seed_opencode_sqlite(&db_path);

        let turns = read_opencode_observed_turns(root, "ses_sqlite_1");
        assert_eq!(turns.len(), 5);
        assert_eq!(turns[0].role, ObservedExternalProviderTurnRole::User);
        assert_eq!(turns[0].text, "Investigate SQLite-backed OpenCode imports.");
        assert_eq!(turns[0].provider_turn_id.as_deref(), Some("prt_user_text"));
        assert_eq!(turns[1].role, ObservedExternalProviderTurnRole::Reasoning);
        assert_eq!(turns[1].text, "Internal reasoning");
        assert_eq!(turns[1].provider_turn_id.as_deref(), Some("prt_reasoning"));
        assert_eq!(turns[2].role, ObservedExternalProviderTurnRole::Tool);
        assert!(turns[2].text.contains("TOOL_STEP_01"));
        assert!(turns[2].text.contains("created"));
        assert_eq!(turns[2].provider_turn_id.as_deref(), Some("prt_tool"));
        assert_eq!(turns[3].role, ObservedExternalProviderTurnRole::Assistant);
        assert_eq!(turns[3].text, "Use the session, message, and part tables.");
        assert_eq!(
            turns[3].provider_turn_id.as_deref(),
            Some("prt_assistant_text")
        );
        assert_eq!(turns[4].role, ObservedExternalProviderTurnRole::Status);
        assert_eq!(
            turns[4].provider_turn_id.as_deref(),
            Some("message-status-msg_assistant")
        );
        assert!(turns[4].text.contains("opencode message completed"));
        assert!(turns[4].text.contains("kimi-k2.6"));
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
                    '{"role":"assistant","modelID":"kimi-k2.6","tokens":{"input":10,"output":5},"time":{"completed":1782113003000},"finish":"stop"}'
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
                    'prt_tool', 'msg_assistant', 'ses_sqlite_1',
                    1782113002002, 1782113002002,
                    '{"type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"printf TOOL_STEP_01"},"output":"created"}}'
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
