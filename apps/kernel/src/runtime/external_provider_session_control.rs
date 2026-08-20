use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{watch, Mutex};

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::{
    AttachedProviderTranscriptCursorKey, AttachedProviderTranscriptCursorStore, DaemonApp,
    ExternalProviderSessionAttachmentRef,
};
use crate::error::DaemonError;
use crate::history::{
    ExternalImportHistoryEntry, SessionHistoryEntry, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
};
use crate::local::{
    ExternalProviderSessionRecord, ImportExternalProviderAgentRequest,
    ImportExternalProviderSessionRequest, ListExternalProviderSessionsRequest, LocalDaemonRequest,
    LocalDaemonResponse,
};
use crate::provider::{
    external_provider_import_model, external_provider_session_providers,
    normalized_observed_prompt_text, ExternalProviderImportMetadata,
    ExternalProviderObservationPolicy, ExternalProviderObservedCursor, LaunchProviderRequest,
    ObservedExternalProviderTurn, ObservedExternalProviderTurnRole, ProviderResumeState,
    ProviderRunState, RuntimeProviderRun,
};
use crate::runtime::state::KernelRuntimeState;
#[cfg(test)]
use crate::session::PromptStatus;
use crate::session::{CreateSessionRequest, PromptQueueItem, RuntimeSession, SessionAgentDefaults};

mod alias;
mod history;
mod import;
mod poller;
mod targets;
#[cfg(test)]
mod tests;

use self::alias::*;
use self::history::*;
use self::import::*;
use self::poller::*;
pub(crate) use self::poller::{
    execute_external_provider_session_request,
    refresh_attached_external_provider_histories_for_session,
};
use self::targets::*;

const EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
const EXTERNAL_PROVIDER_ATTACHED_HISTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_SIGNATURE: Duration = Duration::from_millis(250);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_REFRESH: Duration = Duration::from_millis(500);
const EXTERNAL_PROVIDER_DISCOVERY_FULL_SCAN_AFTER_CACHED_CHECKS: u32 = 10;
const EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN: usize = 64;

fn registered_external_provider_profile_roots(
    runtime_state: &KernelRuntimeState,
    owner_filter: Option<&str>,
) -> Vec<crate::app::ExternalProviderSessionProfileRoot> {
    let registry = runtime_state.provider_account_profile_registry();
    let profiles = match owner_filter {
        Some(owner_user_id) => registry.list(owner_user_id, None),
        None => registry.list_all(),
    }
    .unwrap_or_default();
    profiles
        .into_iter()
        .filter_map(|profile| {
            let environment = registry
                .resolve_environment(
                    &profile.owner_user_id,
                    &profile.provider,
                    &profile.profile_id,
                )
                .ok()?;
            let roots = match profile.provider.as_str() {
                "codex" => environment
                    .get("CODEX_HOME")
                    .map(PathBuf::from)
                    .into_iter()
                    .collect(),
                "claude" => environment
                    .get("CLAUDE_CONFIG_DIR")
                    .map(PathBuf::from)
                    .into_iter()
                    .collect(),
                "opencode" => {
                    let mut roots = Vec::new();
                    if let Some(path) = environment.get("XDG_DATA_HOME") {
                        roots.push(PathBuf::from(path).join("opencode"));
                    }
                    if let Some(path) = environment.get("OPENCODE_CONFIG_DIR") {
                        roots.push(PathBuf::from(path));
                    }
                    roots.sort();
                    roots.dedup();
                    roots
                }
                _ => Vec::new(),
            };
            (!roots.is_empty()).then_some(crate::app::ExternalProviderSessionProfileRoot {
                owner_user_id: profile.owner_user_id,
                provider: profile.provider,
                account_profile: profile.profile_id,
                roots,
            })
        })
        .collect()
}

fn registered_external_provider_profile_roots_for(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
) -> Vec<PathBuf> {
    let account_owner_user_id =
        runtime_state.provider_account_authority_owner_user_id(owner_user_id);
    registered_external_provider_profile_roots(runtime_state, Some(&account_owner_user_id))
        .into_iter()
        .find(|profile| profile.provider == provider && profile.account_profile == account_profile)
        .map(|profile| profile.roots)
        .unwrap_or_default()
}

#[derive(Debug, Default)]
struct ExternalProviderSessionDiscoveryCache {
    signature: Option<crate::app::ExternalProviderSessionDiscoverySignature>,
    candidate_paths: Option<Vec<(String, PathBuf)>>,
    cached_signature_checks: u32,
}

#[derive(Debug)]
struct ExternalProviderSessionDiscoverySignatureRead {
    signature: crate::app::ExternalProviderSessionDiscoverySignature,
    candidate_paths: Vec<(String, PathBuf)>,
    full_scan: bool,
}

pub(crate) async fn run_external_provider_session_discovery_poller(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: KernelRuntimeState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut cache = ExternalProviderSessionDiscoveryCache::default();
    refresh_external_provider_session_index(&app, Some(&runtime_state), Some(&mut cache), false)
        .await;
    let mut discovery_interval =
        tokio::time::interval(EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL);
    discovery_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    discovery_interval.tick().await;
    let mut history_interval =
        tokio::time::interval(EXTERNAL_PROVIDER_ATTACHED_HISTORY_REFRESH_INTERVAL);
    history_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    history_interval.tick().await;
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = discovery_interval.tick() => {
                refresh_external_provider_session_index(&app, Some(&runtime_state), Some(&mut cache), false).await;
            }
            _ = history_interval.tick() => {
                refresh_attached_external_provider_histories_matching(
                    &app,
                    Some(&runtime_state),
                    None,
                    None,
                    true,
                    true,
                )
                .await;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverTarget {
    owner_user_id: String,
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
    external_session_id: String,
    provider: String,
    account_profile: String,
    provider_session_id: String,
    observed_cursor: ExternalProviderObservedCursor,
    cursor_source: AttachedExternalObserverCursorSource,
    needs_responsive_refresh: bool,
}

#[derive(Debug, Clone)]
enum AttachedExternalObserverCursorSource {
    Imported(ExternalProviderImportMetadata),
    CharioxOwned(AttachedProviderTranscriptCursorKey),
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverRead {
    target: AttachedExternalObserverTarget,
    turns: Vec<ObservedExternalProviderTurn>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AttachedExternalObserverAppendOutcome {
    changed_count: usize,
    active_relevant_changed_count: usize,
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
}
