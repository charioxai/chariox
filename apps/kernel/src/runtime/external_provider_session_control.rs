use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{watch, Mutex};

use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::{
    external_session_id_for_provider_session, AttachedProviderTranscriptCursorKey, DaemonApp,
    ExternalProviderSessionAttachmentRef,
};
use crate::error::DaemonError;
use crate::history::{
    ExternalImportHistoryEntry, SessionHistoryEntry,
    EXTERNAL_PROVIDER_ACTIVE_PROMPT_SETTLED_REASON, EXTERNAL_PROVIDER_ACTIVE_PROMPT_STARTED_REASON,
    EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
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
use crate::session::{CreateSessionRequest, PromptQueueItem, RuntimeSession, SessionAgentDefaults};
#[cfg(test)]
use crate::session::{PromptOrigin, PromptStatus};

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
const EXTERNAL_PROVIDER_ATTACHED_ACTIVE_INTERVAL: Duration = Duration::from_secs(1);
const EXTERNAL_PROVIDER_ATTACHED_IDLE_INTERVAL: Duration = Duration::from_secs(20);
const EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW: Duration = Duration::from_secs(120);
const EXTERNAL_PROVIDER_ATTACHED_SETTLE_GRACE: Duration = Duration::from_secs(4);
const EXTERNAL_PROVIDER_ATTACHED_MAX_POLLS_PER_TICK: usize = 2;
const EXTERNAL_PROVIDER_ATTACHED_SLOW_TICK: Duration = Duration::from_millis(250);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_SIGNATURE: Duration = Duration::from_millis(250);
const EXTERNAL_PROVIDER_DISCOVERY_SLOW_REFRESH: Duration = Duration::from_millis(500);
const EXTERNAL_PROVIDER_DISCOVERY_FULL_SCAN_AFTER_CACHED_CHECKS: u32 = 10;
const EXTERNAL_PROVIDER_IMPORT_ALIAS_MAX_LEN: usize = 64;

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
    let mut interval = tokio::time::interval(EXTERNAL_PROVIDER_SESSION_DISCOVERY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {
                refresh_external_provider_session_index(&app, Some(&runtime_state), Some(&mut cache), false).await;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverTarget {
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
    external_session_id: String,
    provider: String,
    provider_session_id: String,
    observed_cursor: ExternalProviderObservedCursor,
    cursor_source: AttachedExternalObserverCursorSource,
}

#[derive(Debug, Clone)]
enum AttachedExternalObserverCursorSource {
    Imported(ExternalProviderImportMetadata),
    ArrobaOwned(AttachedProviderTranscriptCursorKey),
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverRead {
    target: AttachedExternalObserverTarget,
    turns: Vec<ObservedExternalProviderTurn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttachedExternalObserverAppendOptions {
    allow_external_active_prompt_settlement: bool,
}

impl Default for AttachedExternalObserverAppendOptions {
    fn default() -> Self {
        Self {
            allow_external_active_prompt_settlement: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AttachedExternalObserverAppendOutcome {
    changed_count: usize,
    active_relevant_changed_count: usize,
    external_active_prompt_settled: bool,
    session_id: String,
    agent_id: String,
    provider_run_id: Option<String>,
}

#[derive(Debug, Clone)]
struct AttachedExternalObserverSchedule {
    next_due_at: tokio::time::Instant,
    active_until: Option<tokio::time::Instant>,
    last_changed_at: Option<tokio::time::Instant>,
    consecutive_errors: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AttachedExternalObserverPollOutcome {
    target_count: usize,
    due_count: usize,
}

impl AttachedExternalObserverSchedule {
    fn due_now(now: tokio::time::Instant) -> Self {
        Self {
            next_due_at: now,
            active_until: Some(now + EXTERNAL_PROVIDER_ATTACHED_ACTIVE_WINDOW),
            last_changed_at: None,
            consecutive_errors: 0,
        }
    }
}

pub(crate) async fn run_attached_provider_transcript_observer(
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: crate::runtime::state::KernelRuntimeState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut schedule: BTreeMap<String, AttachedExternalObserverSchedule> = BTreeMap::new();
    let mut idle = false;
    loop {
        let delay = if idle {
            EXTERNAL_PROVIDER_ATTACHED_IDLE_INTERVAL
        } else {
            EXTERNAL_PROVIDER_ATTACHED_ACTIVE_INTERVAL
        };
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = tokio::time::sleep(delay) => {
                let outcome = poll_attached_external_provider_transcripts(&app, &runtime_state, &mut schedule).await;
                idle = outcome.target_count == 0;
            }
        }
    }
}
