#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::local::{ExternalProviderSessionCapabilities, ExternalProviderSessionRecord};
#[cfg(test)]
use crate::provider::ExternalProviderObservationPolicy;
use crate::provider::{
    clean_observed_turn_text, clean_provider_prompt, observed_role, text_from_content,
    ObservedExternalProviderTurn, ObservedExternalProviderTurnRole,
};
use crate::session::unix_epoch_ms;

mod cache;
mod claude;
mod codex;
mod common;
mod jsonl;
mod opencode;
mod paths;
#[cfg(test)]
mod tests;

use self::cache::*;
use self::claude::*;
use self::codex::*;
use self::common::*;
use self::jsonl::*;
use self::opencode::*;
use self::paths::*;

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
const DISCOVERED_EXTERNAL_PROVIDER_IDS: &[&str] = &["codex", "claude", "opencode"];

static PROVIDER_TRANSCRIPT_PATH_INDEX: OnceLock<
    Mutex<BTreeMap<String, ExternalProviderTranscriptIndexEntry>>,
> = OnceLock::new();
static PROVIDER_TRANSCRIPT_DISCOVERY_PATH_INDEX: OnceLock<
    Mutex<BTreeMap<(String, PathBuf), ExternalProviderTranscriptDiscoveryPathEntry>>,
> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static JSONL_PREFIX_READ_COUNT: Cell<usize> = const { Cell::new(0) };
    static JSONL_RECENT_READ_COUNT: Cell<usize> = const { Cell::new(0) };
    static JSONL_INCREMENTAL_READ_COUNT: Cell<usize> = const { Cell::new(0) };
    static FILE_CANDIDATE_SCAN_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalProviderTranscriptIndexEntry {
    provider_session_id: String,
    path: PathBuf,
    len: u64,
    modified_at_ms: u64,
    discovery_record: Option<ExternalProviderSessionRecord>,
    last_observed_offset: u64,
    observed_len: Option<u64>,
    observed_modified_at_ms: Option<u64>,
    observed_turns: Option<Vec<ObservedExternalProviderTurn>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalProviderTranscriptDiscoveryPathEntry {
    len: u64,
    modified_at_ms: u64,
    record: ExternalProviderSessionRecord,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ProviderTranscriptFileFingerprint {
    len: u64,
    modified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedProviderObservedTranscript {
    last_observed_offset: u64,
    observed_turns: Vec<ObservedExternalProviderTurn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalProviderSessionDiscoverySignature {
    files: Vec<ExternalProviderSessionFileSignature>,
}

impl ExternalProviderSessionDiscoverySignature {
    pub(crate) fn same_candidate_files(&self, other: &Self) -> bool {
        self.files.len() == other.files.len()
            && self
                .files
                .iter()
                .zip(other.files.iter())
                .all(|(left, right)| left.provider == right.provider && left.path == right.path)
    }

    pub(crate) fn providers_with_changed_candidate_files(&self, other: &Self) -> BTreeSet<String> {
        let providers = self
            .files
            .iter()
            .chain(other.files.iter())
            .map(|file| file.provider.as_str())
            .collect::<BTreeSet<_>>();

        providers
            .into_iter()
            .filter(|provider| {
                let left = self
                    .files
                    .iter()
                    .filter(|file| file.provider == **provider)
                    .map(|file| file.path.as_path())
                    .collect::<Vec<_>>();
                let right = other
                    .files
                    .iter()
                    .filter(|file| file.provider == **provider)
                    .map(|file| file.path.as_path())
                    .collect::<Vec<_>>();
                left != right
            })
            .map(str::to_string)
            .collect()
    }
}

pub(crate) fn external_provider_session_transcript_needs_refresh(
    provider: &str,
    provider_session_id: &str,
) -> bool {
    provider_observed_transcript_needs_refresh(provider, provider_session_id)
}

pub(crate) fn external_provider_session_transcript_needs_refresh_for_profile(
    provider: &str,
    provider_session_id: &str,
    roots: &[PathBuf],
) -> bool {
    provider_observed_transcript_needs_refresh_in_roots(provider, provider_session_id, roots)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalProviderSessionFileSignature {
    provider: String,
    path: PathBuf,
    len: u64,
    modified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalProviderSessionProfileRoot {
    pub(crate) owner_user_id: String,
    pub(crate) provider: String,
    pub(crate) account_profile: String,
    pub(crate) roots: Vec<PathBuf>,
}

pub(crate) fn discover_external_provider_sessions_for_profiles(
    profiles: &[ExternalProviderSessionProfileRoot],
    provider_filter: Option<&str>,
) -> Vec<ExternalProviderSessionRecord> {
    let mut sessions = Vec::new();
    for profile in profiles
        .iter()
        .filter(|profile| provider_matches(provider_filter, &profile.provider))
    {
        for root in &profile.roots {
            let discovered = match profile.provider.as_str() {
                "codex" => discover_codex_external_sessions(root),
                "claude" => discover_claude_external_sessions(root),
                "opencode" => discover_opencode_external_sessions(root),
                _ => Vec::new(),
            };
            sessions.extend(discovered.into_iter().map(|mut session| {
                session.owner_user_id = profile.owner_user_id.clone();
                session.account_profile = profile.account_profile.clone();
                session.external_session_id = format!(
                    "{}:{}:{}",
                    session.provider, session.account_profile, session.provider_session_id
                );
                session
            }));
        }
    }
    deduplicate_external_sessions(sessions)
}

pub(crate) fn external_provider_session_candidate_paths_for_profiles(
    profiles: &[ExternalProviderSessionProfileRoot],
    provider_filter: Option<&str>,
) -> Vec<(String, PathBuf)> {
    let mut paths = Vec::new();
    for profile in profiles
        .iter()
        .filter(|profile| provider_matches(provider_filter, &profile.provider))
    {
        for root in &profile.roots {
            let candidates = match profile.provider.as_str() {
                "codex" => codex_candidate_paths(root),
                "claude" => claude_candidate_paths(root),
                "opencode" => opencode_candidate_paths(root),
                _ => Vec::new(),
            };
            paths.extend(
                candidates
                    .into_iter()
                    .map(|path| (profile.provider.clone(), path)),
            );
        }
    }
    paths
}

pub(crate) fn read_external_provider_observed_turns_for_profile(
    provider: &str,
    provider_session_id: &str,
    roots: &[PathBuf],
) -> Vec<ObservedExternalProviderTurn> {
    let turns = match normalized_external_provider_id(provider) {
        "codex" => roots
            .iter()
            .flat_map(|root| read_codex_observed_turns(root, provider_session_id))
            .collect::<Vec<_>>(),
        "claude" => roots
            .iter()
            .flat_map(|root| read_claude_observed_turns(root, provider_session_id))
            .collect::<Vec<_>>(),
        "opencode" => roots
            .iter()
            .flat_map(|root| read_opencode_observed_turns(root, provider_session_id))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    latest_observed_turns(turns)
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

pub(crate) fn discovered_external_provider_ids() -> &'static [&'static str] {
    DISCOVERED_EXTERNAL_PROVIDER_IDS
}

pub(crate) fn external_provider_session_discovery_candidate_paths(
    provider_filter: Option<&str>,
) -> Vec<(String, PathBuf)> {
    provider_session_candidate_paths(provider_filter)
}

pub(crate) fn external_provider_session_discovery_signature_for_candidates(
    paths: &[(String, PathBuf)],
) -> ExternalProviderSessionDiscoverySignature {
    let mut files = paths
        .iter()
        .filter_map(|(provider, path)| {
            let metadata = fs::metadata(&path).ok()?;
            Some(ExternalProviderSessionFileSignature {
                provider: provider.clone(),
                path: path.clone(),
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
    let turns = match normalized_external_provider_id(provider) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalProviderPromptRecoveryMatch {
    pub(crate) provider_session_id: String,
    pub(crate) original_prompt_observed: bool,
    pub(crate) recovery_operation_observed: bool,
    pub(crate) provider_activity_after_anchor: bool,
    pub(crate) settled_after_anchor: bool,
}

pub(crate) fn find_external_provider_prompt_recovery_match(
    provider: &str,
    prompt: &str,
    worktree_path: Option<&str>,
    recovery_operation_id: Option<&str>,
) -> Option<ExternalProviderPromptRecoveryMatch> {
    let provider = normalized_external_provider_id(provider);
    if provider.is_empty() {
        return None;
    }
    let sessions = discover_external_provider_sessions(Some(provider));
    select_external_provider_prompt_recovery_match(
        provider,
        prompt,
        worktree_path,
        recovery_operation_id,
        sessions,
        |provider_session_id| read_external_provider_observed_turns(provider, provider_session_id),
    )
}

fn select_external_provider_prompt_recovery_match(
    provider: &str,
    prompt: &str,
    worktree_path: Option<&str>,
    recovery_operation_id: Option<&str>,
    sessions: Vec<ExternalProviderSessionRecord>,
    mut read_turns: impl FnMut(&str) -> Vec<ObservedExternalProviderTurn>,
) -> Option<ExternalProviderPromptRecoveryMatch> {
    let expected_prompt = crate::provider::normalized_observed_prompt_text(prompt)?;
    let mut matches = sessions
        .into_iter()
        .filter(|session| provider_matches(Some(&session.provider), provider))
        .filter_map(|session| {
            let worktree_exact = match (worktree_path, session.worktree_path.as_deref()) {
                (Some(expected), Some(actual)) if !same_path(expected, actual) => return None,
                (Some(_), Some(_)) => true,
                _ => false,
            };
            let turns = read_turns(&session.provider_session_id);
            let original_index = turns.iter().rposition(|turn| {
                turn.role == ObservedExternalProviderTurnRole::User
                    && crate::provider::normalized_observed_prompt_text(&turn.text).as_deref()
                        == Some(expected_prompt.as_str())
            });
            let recovery_index = recovery_operation_id.and_then(|operation_id| {
                turns.iter().rposition(|turn| {
                    turn.role == ObservedExternalProviderTurnRole::User
                        && turn.text.contains(operation_id)
                })
            });
            let anchor_index = match (original_index, recovery_index) {
                (Some(original), Some(recovery)) => Some(original.max(recovery)),
                (Some(original), None) => Some(original),
                (None, Some(recovery)) => Some(recovery),
                (None, None) => None,
            }?;
            let after_anchor = &turns[anchor_index + 1..];
            let policy = crate::provider::ExternalProviderObservationPolicy::for_provider(provider);
            Some((
                worktree_exact,
                session.last_modified_at_ms,
                ExternalProviderPromptRecoveryMatch {
                    provider_session_id: session.provider_session_id,
                    original_prompt_observed: original_index.is_some(),
                    recovery_operation_observed: recovery_index.is_some(),
                    provider_activity_after_anchor: after_anchor.iter().any(|turn| {
                        turn.role != ObservedExternalProviderTurnRole::User
                            && !policy.turn_is_passive_telemetry(turn)
                    }),
                    settled_after_anchor: policy.latest_effective_turn_settles(after_anchor),
                },
            ))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.provider_session_id.cmp(&right.2.provider_session_id))
    });
    matches.into_iter().next().map(|(_, _, matched)| matched)
}

fn same_path(left: &str, right: &str) -> bool {
    let left = PathBuf::from(left);
    let right = PathBuf::from(right);
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn provider_matches(filter: Option<&str>, provider: &str) -> bool {
    filter.map_or(true, |filter| {
        normalized_external_provider_id(filter) == provider
    })
}

fn normalized_external_provider_id(provider: &str) -> &'static str {
    let provider = provider.trim();
    discovered_external_provider_ids()
        .iter()
        .copied()
        .find(|candidate| provider.eq_ignore_ascii_case(candidate))
        .unwrap_or("")
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
