use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::ProjectionMetadata;
use crate::agent::{AgentInstance, AgentState};
use crate::extension::{ExtensionKind, RemoteExtensionManifestSyncState};
use crate::runtime::capability_executor::CapabilityExecutorHealthSnapshot;
use crate::runtime::process_health::KernelProcessHealthSnapshot;
use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::slice::{SliceOperationStatus, SliceRecord, SliceStatus};
use crate::slice_provider_auth::{SliceProviderAuthState, SliceProviderAuthSummary};
use crate::terminal::TerminalStreamHealthSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorQueueSnapshot {
    pub lane_id: String,
    pub queue_limit: usize,
    pub queued_commands: usize,
}

impl ActorQueueSnapshot {
    pub fn new(lane_id: impl Into<String>, queue_limit: usize, queued_commands: usize) -> Self {
        Self {
            lane_id: lane_id.into(),
            queue_limit,
            queued_commands,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjectionHealthSnapshot {
    pub projected_sessions: usize,
    pub projected_session_list_entries: Option<usize>,
    pub active_prompts: usize,
    pub queued_prompts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProjectionHealthSnapshot {
    pub projected_agents: usize,
    pub active_prompts: usize,
    pub queued_prompts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectionInvariantHealthSnapshot {
    pub checked_sessions: usize,
    pub checked_agents: usize,
    pub mismatches: Vec<ProjectionInvariantMismatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionInvariantMismatch {
    pub kind: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderRunActorHealthSnapshot {
    pub enqueued_commands: u64,
    pub enqueue_rejections: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCatalogHealthSnapshot {
    pub cached: bool,
    pub expired: bool,
    pub age_ms: Option<u64>,
    pub ttl_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderRunHealthSnapshot {
    pub projected_runs: usize,
    pub active_runs: usize,
    pub arroba_active_runs: usize,
    pub native_tui_active_runs: usize,
    pub terminal_diagnostics: Vec<ProviderRunTerminalDiagnosticIssue>,
    pub duplicate_arroba_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
    pub duplicate_native_tui_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
    pub multi_interface_agent_bindings: Vec<ProviderRunAgentBindingConflict>,
    pub orphaned_active_runs: Vec<ProviderRunIdentityIssue>,
    pub session_active_run_mismatches: Vec<ProviderRunSessionPointerIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunAgentBindingConflict {
    pub session_id: String,
    pub agent_id: String,
    pub provider_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunIdentityIssue {
    pub provider_run_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunSessionPointerIssue {
    pub session_id: String,
    pub active_provider_run_id: Option<String>,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunTerminalDiagnosticIssue {
    pub provider_run_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub provider: String,
    pub state: String,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeClaimSnapshot {
    pub workspace_id: String,
    pub worktree_id: String,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceCoordinationHealthSnapshot {
    pub active_worktree_claims: Vec<WorktreeClaimSnapshot>,
    pub worktree_collisions: Vec<WorktreeClaimSnapshot>,
    pub active_operation_claims: Vec<WorkspaceOperationClaimSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncHealthSnapshot {
    pub active_reservations: usize,
    pub active_reservation_artifacts: usize,
    pub managed_mode: WorkspaceLiveSyncManagedModeHealthSnapshot,
    pub workspace_identity:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot,
    pub external_changes: crate::io::ArtifactExternalChangeHealthSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncManagedModeHealthSnapshot {
    pub write_fence_supported: bool,
    pub write_fence_backend: Option<String>,
    pub unavailable_reason: Option<String>,
}

impl WorkspaceLiveSyncManagedModeHealthSnapshot {
    pub fn current() -> Self {
        Self {
            write_fence_supported: crate::provider::workspace_write_fence_supported(),
            write_fence_backend: crate::provider::workspace_write_fence_backend()
                .map(ToString::to_string),
            unavailable_reason: crate::provider::workspace_write_fence_unavailable_reason()
                .map(ToString::to_string),
        }
    }
}

impl Default for WorkspaceLiveSyncHealthSnapshot {
    fn default() -> Self {
        Self {
            active_reservations: 0,
            active_reservation_artifacts: 0,
            managed_mode: WorkspaceLiveSyncManagedModeHealthSnapshot::current(),
            workspace_identity:
                crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitorHealthSnapshot {
                    tracked_provider_runs: 0,
                    identity_changed_provider_runs: 0,
                    invalid_provider_runs: 0,
                    current_generation_total: 0,
                    issues: Vec::new(),
                },
            external_changes: crate::io::ArtifactExternalChangeHealthSnapshot {
                tracked_artifacts: 0,
                externally_changed_artifacts: 0,
                external_change_events: 0,
                live_watcher_started: false,
                live_watcher_scans: 0,
                live_watcher_scan_errors: 0,
                issues: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SliceLifecycleHealthSnapshot {
    pub total_slices: usize,
    pub running_slices: usize,
    pub starting_slices: usize,
    pub stopping_slices: usize,
    pub stopped_slices: usize,
    pub unhealthy_slices: usize,
    pub attached_agents: usize,
    pub failed_operations: usize,
    pub in_progress_operations: usize,
    pub issues: Vec<SliceLifecycleIssue>,
    pub provider_auth_missing_slices: usize,
    pub provider_auth_unconfigured_slices: usize,
    pub provider_auth_issues: Vec<SliceProviderAuthIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLifecycleIssue {
    pub slice_id: String,
    pub name: String,
    pub status: String,
    pub last_operation: Option<String>,
    pub last_operation_status: Option<String>,
    pub last_error: Option<String>,
    pub session_ids: Vec<String>,
    pub agent_ids: Vec<String>,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceProviderAuthIssue {
    pub slice_id: String,
    pub name: String,
    pub status: String,
    pub session_ids: Vec<String>,
    pub agent_ids: Vec<String>,
    pub worktree_id: Option<String>,
    pub provider: Option<String>,
    pub provider_auth_state: Option<String>,
    pub alias: Option<String>,
    pub identity: Option<String>,
    pub details: String,
}

impl SliceLifecycleHealthSnapshot {
    pub(crate) fn from_slices(slices: &[SliceRecord]) -> Self {
        let mut snapshot = Self {
            total_slices: slices.len(),
            ..Self::default()
        };
        for slice in slices {
            match slice.status {
                SliceStatus::Running => snapshot.running_slices += 1,
                SliceStatus::Starting => snapshot.starting_slices += 1,
                SliceStatus::Stopping => snapshot.stopping_slices += 1,
                SliceStatus::Stopped => snapshot.stopped_slices += 1,
                SliceStatus::Unhealthy => snapshot.unhealthy_slices += 1,
            }
            snapshot.attached_agents += slice.agent_ids.len();
            match slice.last_operation_status {
                Some(SliceOperationStatus::Failed) => snapshot.failed_operations += 1,
                Some(SliceOperationStatus::InProgress) => snapshot.in_progress_operations += 1,
                _ => {}
            }
            if slice.status == SliceStatus::Unhealthy
                || slice.last_operation_status == Some(SliceOperationStatus::Failed)
                || (slice.status == SliceStatus::Stopped && !slice.agent_ids.is_empty())
            {
                snapshot.issues.push(SliceLifecycleIssue {
                    slice_id: slice.id.clone(),
                    name: slice.name.clone(),
                    status: slice_status_key(&slice.status).to_string(),
                    last_operation: slice.last_operation.clone(),
                    last_operation_status: slice
                        .last_operation_status
                        .as_ref()
                        .map(slice_operation_status_key)
                        .map(ToString::to_string),
                    last_error: slice.last_error.clone(),
                    session_ids: slice.session_ids.clone(),
                    agent_ids: slice.agent_ids.clone(),
                    worktree_id: slice.worktree_id.clone(),
                });
            }

            if !slice.agent_ids.is_empty() {
                let expected_providers = slice_expected_auth_providers(slice);
                let missing_providers = slice_missing_auth_providers(slice, &expected_providers);
                if !missing_providers.is_empty()
                    || (expected_providers.is_empty() && slice.provider_auth.is_empty())
                {
                    snapshot.provider_auth_missing_slices += 1;
                    if expected_providers.is_empty() {
                        snapshot.provider_auth_issues.push(SliceProviderAuthIssue {
                            slice_id: slice.id.clone(),
                            name: slice.name.clone(),
                            status: slice_status_key(&slice.status).to_string(),
                            session_ids: slice.session_ids.clone(),
                            agent_ids: slice.agent_ids.clone(),
                            worktree_id: slice.worktree_id.clone(),
                            provider: None,
                            provider_auth_state: None,
                            alias: None,
                            identity: None,
                            details: "slice has attached agents but no provider account configured"
                                .to_string(),
                        });
                    } else {
                        for provider in &missing_providers {
                            snapshot.provider_auth_issues.push(SliceProviderAuthIssue {
                                slice_id: slice.id.clone(),
                                name: slice.name.clone(),
                                status: slice_status_key(&slice.status).to_string(),
                                session_ids: slice.session_ids.clone(),
                                agent_ids: slice.agent_ids.clone(),
                                worktree_id: slice.worktree_id.clone(),
                                provider: Some(provider.clone()),
                                provider_auth_state: None,
                                alias: None,
                                identity: None,
                                details: format!(
                                    "slice has attached agents but no {provider} provider account configured"
                                ),
                            });
                        }
                    }
                }
                let mut slice_has_unconfigured_auth = false;
                for auth in slice.provider_auth.iter().filter(|auth| {
                    slice_provider_auth_needs_attention(&auth.state)
                        && slice_provider_auth_targets_expected_provider(
                            &auth.provider,
                            &expected_providers,
                        )
                }) {
                    if !slice_has_unconfigured_auth {
                        snapshot.provider_auth_unconfigured_slices += 1;
                        slice_has_unconfigured_auth = true;
                    }
                    snapshot.provider_auth_issues.push(SliceProviderAuthIssue {
                        slice_id: slice.id.clone(),
                        name: slice.name.clone(),
                        status: slice_status_key(&slice.status).to_string(),
                        session_ids: slice.session_ids.clone(),
                        agent_ids: slice.agent_ids.clone(),
                        worktree_id: slice.worktree_id.clone(),
                        provider: Some(auth.provider.clone()),
                        provider_auth_state: Some(
                            slice_provider_auth_state_key(&auth.state).to_string(),
                        ),
                        alias: auth.alias.clone(),
                        identity: slice_provider_auth_identity(auth),
                        details: "slice provider account needs login or import".to_string(),
                    });
                }
            }
        }
        snapshot
    }
}

fn slice_expected_auth_providers(slice: &SliceRecord) -> Vec<String> {
    let providers = normalized_provider_names(&slice.providers);
    if providers.is_empty() {
        normalized_provider_names(
            &slice
                .provider_auth
                .iter()
                .map(|auth| auth.provider.clone())
                .collect::<Vec<_>>(),
        )
    } else {
        providers
    }
}

fn slice_missing_auth_providers(slice: &SliceRecord, expected_providers: &[String]) -> Vec<String> {
    if expected_providers.is_empty() {
        return Vec::new();
    }
    expected_providers
        .iter()
        .filter(|provider| {
            !slice
                .provider_auth
                .iter()
                .any(|auth| slice_provider_matches(&auth.provider, provider))
        })
        .cloned()
        .collect()
}

fn normalized_provider_names(providers: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for provider in providers {
        let provider = provider.trim();
        if !provider.is_empty() && !normalized.iter().any(|entry| entry == provider) {
            normalized.push(provider.to_string());
        }
    }
    normalized
}

fn slice_provider_auth_targets_expected_provider(
    auth_provider: &str,
    expected_providers: &[String],
) -> bool {
    expected_providers.is_empty()
        || expected_providers
            .iter()
            .any(|provider| slice_provider_matches(auth_provider, provider))
}

fn slice_provider_matches(auth_provider: &str, advertised_provider: &str) -> bool {
    auth_provider == advertised_provider
        || auth_provider
            .strip_prefix(advertised_provider)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn slice_provider_auth_needs_attention(state: &SliceProviderAuthState) -> bool {
    matches!(
        state,
        SliceProviderAuthState::Unknown | SliceProviderAuthState::NotConfigured
    )
}

fn slice_provider_auth_state_key(state: &SliceProviderAuthState) -> &'static str {
    match state {
        SliceProviderAuthState::Unknown => "unknown",
        SliceProviderAuthState::NotConfigured => "not_configured",
        SliceProviderAuthState::Configured => "configured",
        SliceProviderAuthState::Authenticated => "authenticated",
    }
}

fn slice_provider_auth_identity(auth: &SliceProviderAuthSummary) -> Option<String> {
    auth.alias_or_identity()
        .or(auth.organization_name.as_deref())
        .or(auth.organization_id.as_deref())
        .or(auth.auth_type.as_deref())
        .map(str::to_string)
}

fn slice_status_key(status: &SliceStatus) -> &'static str {
    match status {
        SliceStatus::Stopped => "stopped",
        SliceStatus::Starting => "starting",
        SliceStatus::Stopping => "stopping",
        SliceStatus::Running => "running",
        SliceStatus::Unhealthy => "unhealthy",
    }
}

fn slice_operation_status_key(status: &SliceOperationStatus) -> &'static str {
    match status {
        SliceOperationStatus::Accepted => "accepted",
        SliceOperationStatus::InProgress => "in_progress",
        SliceOperationStatus::Completed => "completed",
        SliceOperationStatus::Failed => "failed",
        SliceOperationStatus::Reconciled => "reconciled",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteExtensionSyncHealthSnapshot {
    pub remote_agents: usize,
    pub home_proxy_agents: usize,
    pub home_proxy_grants: usize,
    pub manifest_missing_agents: usize,
    pub synced_agents: usize,
    pub syncing_agents: usize,
    pub pending_agents: usize,
    pub failed_agents: usize,
    pub stale_agents: usize,
    pub pending_revoke_agents: usize,
    #[serde(default)]
    pub worker_extension_agents: usize,
    #[serde(default)]
    pub worker_extension_grants: usize,
    #[serde(default)]
    pub worker_manifest_missing_agents: usize,
    #[serde(default)]
    pub worker_synced_agents: usize,
    #[serde(default)]
    pub worker_syncing_agents: usize,
    #[serde(default)]
    pub worker_pending_agents: usize,
    #[serde(default)]
    pub worker_failed_agents: usize,
    #[serde(default)]
    pub worker_stale_agents: usize,
    #[serde(default)]
    pub worker_pending_revoke_agents: usize,
    pub issues: Vec<RemoteExtensionSyncIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionSyncIssue {
    pub session_id: String,
    pub agent_id: String,
    pub agent_ref: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    pub active_worker_provider_run_id: Option<String>,
    pub state: String,
    pub manifest_hash: Option<String>,
    pub last_error: Option<String>,
    pub pending_revoke: bool,
    #[serde(default)]
    pub source: crate::extension::ExtensionSource,
    pub home_proxy_grants: Vec<String>,
    #[serde(default)]
    pub worker_grants: Vec<String>,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteExecutionHealthSnapshot {
    pub remote_agents: usize,
    pub active_remote_agents: usize,
    pub missing_active_worker_runs: usize,
    pub malformed_bindings: usize,
    pub issues: Vec<RemoteExecutionIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExecutionIssue {
    pub kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub agent_ref: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub execution_lease_id: String,
    pub leased_agent_id: String,
    pub active_worker_provider_run_id: Option<String>,
    pub state: String,
    pub is_processing: bool,
    pub worktree_id: Option<String>,
    pub details: String,
}

impl RemoteExecutionHealthSnapshot {
    pub(crate) fn from_agents_with_active_agent_ids(
        agents: &[AgentInstance],
        active_agent_ids: &BTreeSet<String>,
    ) -> Self {
        let mut snapshot = Self::default();
        for agent in agents {
            let Some(remote_execution) = agent.remote_execution() else {
                continue;
            };
            snapshot.remote_agents += 1;

            let active = active_agent_ids.contains(agent.id());
            if active {
                snapshot.active_remote_agents += 1;
            }

            let mut malformed_fields = Vec::new();
            if remote_execution.worker_kernel_id.is_empty() {
                malformed_fields.push("worker_kernel_id");
            }
            if remote_execution.worker_machine_id.is_empty() {
                malformed_fields.push("worker_machine_id");
            }
            if remote_execution.execution_lease_id.is_empty() {
                malformed_fields.push("execution_lease_id");
            }
            if remote_execution.leased_agent_id.is_empty() {
                malformed_fields.push("leased_agent_id");
            }
            if remote_execution
                .active_worker_provider_run_id
                .as_deref()
                .is_some_and(str::is_empty)
            {
                malformed_fields.push("active_worker_provider_run_id");
            }
            if !malformed_fields.is_empty() {
                snapshot.malformed_bindings += 1;
                snapshot.issues.push(remote_execution_issue(
                    agent,
                    remote_execution,
                    "malformed_binding",
                    format!(
                        "remote execution binding is missing {}",
                        malformed_fields.join(", ")
                    ),
                ));
            }

            if active
                && remote_execution
                    .active_worker_provider_run_id
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                snapshot.missing_active_worker_runs += 1;
                snapshot.issues.push(remote_execution_issue(
                    agent,
                    remote_execution,
                    "missing_active_worker_provider_run",
                    "active remote agent has no active worker provider run id".to_string(),
                ));
            }
        }
        snapshot
    }
}

fn remote_execution_issue(
    agent: &AgentInstance,
    remote_execution: &crate::agent::RemoteAgentBinding,
    kind: &str,
    details: String,
) -> RemoteExecutionIssue {
    RemoteExecutionIssue {
        kind: kind.to_string(),
        session_id: agent.session_id().to_string(),
        agent_id: agent.id().to_string(),
        agent_ref: agent.agent_ref().to_string(),
        worker_kernel_id: remote_execution.worker_kernel_id.clone(),
        worker_machine_id: remote_execution.worker_machine_id.clone(),
        execution_lease_id: remote_execution.execution_lease_id.clone(),
        leased_agent_id: remote_execution.leased_agent_id.clone(),
        active_worker_provider_run_id: remote_execution.active_worker_provider_run_id.clone(),
        state: agent_state_key(agent.state()).to_string(),
        is_processing: agent.is_processing(),
        worktree_id: agent.worktree_id().map(ToString::to_string),
        details,
    }
}

fn agent_state_key(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Focused => "focused",
        AgentState::Error => "error",
    }
}

impl RemoteExtensionSyncHealthSnapshot {
    pub(crate) fn from_agents(agents: &[AgentInstance]) -> Self {
        let mut snapshot = Self::default();
        for agent in agents {
            let Some(remote_execution) = agent.remote_execution() else {
                continue;
            };
            snapshot.remote_agents += 1;
            let home_proxy_grants = agent
                .extension_grants()
                .iter()
                .filter(|grant| {
                    grant.source == crate::extension::ExtensionSource::Home
                        && grant.kind != ExtensionKind::Skill
                })
                .map(|grant| format!("{}:{}", grant.kind.as_str(), grant.name))
                .collect::<Vec<_>>();
            let home_proxy_grant_count = home_proxy_grants.len();
            let status = agent.remote_extension_manifest_sync();
            let pending_revoke = status
                .and_then(|status| status.pending_revoke)
                .unwrap_or(false);
            if home_proxy_grant_count > 0 || pending_revoke {
                snapshot.home_proxy_agents += 1;
                snapshot.home_proxy_grants += home_proxy_grant_count;
                match status {
                    None => {
                        snapshot.manifest_missing_agents += 1;
                        snapshot.issues.push(remote_extension_sync_issue(
                            agent,
                            remote_execution,
                            "missing",
                            None,
                            None,
                            false,
                            crate::extension::ExtensionSource::Home,
                            home_proxy_grants,
                        ));
                    }
                    Some(status) => {
                        match status.state {
                            RemoteExtensionManifestSyncState::Synced => snapshot.synced_agents += 1,
                            RemoteExtensionManifestSyncState::Syncing => {
                                snapshot.syncing_agents += 1
                            }
                            RemoteExtensionManifestSyncState::Pending => {
                                snapshot.pending_agents += 1
                            }
                            RemoteExtensionManifestSyncState::Failed => snapshot.failed_agents += 1,
                            RemoteExtensionManifestSyncState::Stale => snapshot.stale_agents += 1,
                        }
                        if pending_revoke {
                            snapshot.pending_revoke_agents += 1;
                        }
                        if matches!(
                            status.state,
                            RemoteExtensionManifestSyncState::Failed
                                | RemoteExtensionManifestSyncState::Stale
                        ) || pending_revoke
                        {
                            snapshot.issues.push(remote_extension_sync_issue(
                                agent,
                                remote_execution,
                                remote_extension_sync_state_key(status.state),
                                status.manifest_hash.clone(),
                                status.last_error.clone(),
                                pending_revoke,
                                crate::extension::ExtensionSource::Home,
                                home_proxy_grants,
                            ));
                        }
                    }
                }
            }

            let worker_grants = agent
                .extension_grants()
                .iter()
                .filter(|grant| grant.source == crate::extension::ExtensionSource::Worker)
                .map(|grant| format!("{}:{}", grant.kind.as_str(), grant.name))
                .collect::<Vec<_>>();
            let worker_grant_count = worker_grants.len();
            let worker_status = agent.worker_extension_grant_sync();
            let worker_pending_revoke = worker_status
                .and_then(|status| status.pending_revoke)
                .unwrap_or(false);
            if worker_grant_count == 0 && !worker_pending_revoke {
                continue;
            }
            snapshot.worker_extension_agents += 1;
            snapshot.worker_extension_grants += worker_grant_count;
            let Some(worker_status) = worker_status else {
                snapshot.worker_manifest_missing_agents += 1;
                snapshot.issues.push(remote_extension_sync_issue(
                    agent,
                    remote_execution,
                    "missing",
                    None,
                    None,
                    false,
                    crate::extension::ExtensionSource::Worker,
                    worker_grants,
                ));
                continue;
            };
            match worker_status.state {
                RemoteExtensionManifestSyncState::Synced => snapshot.worker_synced_agents += 1,
                RemoteExtensionManifestSyncState::Syncing => snapshot.worker_syncing_agents += 1,
                RemoteExtensionManifestSyncState::Pending => snapshot.worker_pending_agents += 1,
                RemoteExtensionManifestSyncState::Failed => snapshot.worker_failed_agents += 1,
                RemoteExtensionManifestSyncState::Stale => snapshot.worker_stale_agents += 1,
            }
            if worker_pending_revoke {
                snapshot.worker_pending_revoke_agents += 1;
            }
            if matches!(
                worker_status.state,
                RemoteExtensionManifestSyncState::Failed | RemoteExtensionManifestSyncState::Stale
            ) || worker_pending_revoke
            {
                snapshot.issues.push(remote_extension_sync_issue(
                    agent,
                    remote_execution,
                    remote_extension_sync_state_key(worker_status.state),
                    worker_status.manifest_hash.clone(),
                    worker_status.last_error.clone(),
                    worker_pending_revoke,
                    crate::extension::ExtensionSource::Worker,
                    worker_grants,
                ));
            }
        }
        snapshot
    }
}

fn remote_extension_sync_issue(
    agent: &AgentInstance,
    remote_execution: &crate::agent::RemoteAgentBinding,
    state: &str,
    manifest_hash: Option<String>,
    last_error: Option<String>,
    pending_revoke: bool,
    source: crate::extension::ExtensionSource,
    extension_grants: Vec<String>,
) -> RemoteExtensionSyncIssue {
    RemoteExtensionSyncIssue {
        session_id: agent.session_id().to_string(),
        agent_id: agent.id().to_string(),
        agent_ref: agent.agent_ref().to_string(),
        worker_kernel_id: remote_execution.worker_kernel_id.clone(),
        worker_machine_id: remote_execution.worker_machine_id.clone(),
        execution_lease_id: remote_execution.execution_lease_id.clone(),
        leased_agent_id: remote_execution.leased_agent_id.clone(),
        active_worker_provider_run_id: remote_execution.active_worker_provider_run_id.clone(),
        state: state.to_string(),
        manifest_hash,
        last_error,
        pending_revoke,
        source,
        home_proxy_grants: if source == crate::extension::ExtensionSource::Home {
            extension_grants.clone()
        } else {
            Vec::new()
        },
        worker_grants: if source == crate::extension::ExtensionSource::Worker {
            extension_grants
        } else {
            Vec::new()
        },
        worktree_id: agent.worktree_id().map(ToString::to_string),
    }
}

fn remote_extension_sync_state_key(state: RemoteExtensionManifestSyncState) -> &'static str {
    match state {
        RemoteExtensionManifestSyncState::Synced => "synced",
        RemoteExtensionManifestSyncState::Syncing => "syncing",
        RemoteExtensionManifestSyncState::Pending => "pending",
        RemoteExtensionManifestSyncState::Failed => "failed",
        RemoteExtensionManifestSyncState::Stale => "stale",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHealthProjection {
    pub metadata: ProjectionMetadata,
    pub session_command_lanes: Vec<ActorQueueSnapshot>,
    pub agent_command_lanes: Vec<ActorQueueSnapshot>,
    pub workflow_command_lanes: Vec<ActorQueueSnapshot>,
    pub provider_runtime_lanes: Vec<ActorQueueSnapshot>,
    pub provider_run_actor: ProviderRunActorHealthSnapshot,
    pub process: KernelProcessHealthSnapshot,
    pub capability_executor: CapabilityExecutorHealthSnapshot,
    pub session_projection: SessionProjectionHealthSnapshot,
    pub agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
    pub provider_catalog: ProviderCatalogHealthSnapshot,
    pub provider_runs: ProviderRunHealthSnapshot,
    pub transport: super::TransportHealthSnapshot,
    pub terminal_stream: TerminalStreamHealthSnapshot,
    pub slice_lifecycle: SliceLifecycleHealthSnapshot,
    pub remote_execution: RemoteExecutionHealthSnapshot,
    pub remote_extension_sync: RemoteExtensionSyncHealthSnapshot,
    pub workspace_coordination: WorkspaceCoordinationHealthSnapshot,
    pub workspace_live_sync: WorkspaceLiveSyncHealthSnapshot,
    pub projection_invariants: ProjectionInvariantHealthSnapshot,
    #[serde(default)]
    pub app_lock: crate::runtime::app_lock::AppLockHealthSnapshot,
}

impl DaemonHealthProjection {
    pub fn new(
        last_event_id: u64,
        session_command_lanes: Vec<ActorQueueSnapshot>,
        agent_command_lanes: Vec<ActorQueueSnapshot>,
        workflow_command_lanes: Vec<ActorQueueSnapshot>,
        provider_runtime_lanes: Vec<ActorQueueSnapshot>,
        provider_run_actor: ProviderRunActorHealthSnapshot,
        process: KernelProcessHealthSnapshot,
        capability_executor: CapabilityExecutorHealthSnapshot,
        mut session_projection: SessionProjectionHealthSnapshot,
        agent_runtime_projection: AgentRuntimeProjectionHealthSnapshot,
        provider_catalog: ProviderCatalogHealthSnapshot,
        provider_runs: ProviderRunHealthSnapshot,
        transport: super::TransportHealthSnapshot,
        terminal_stream: TerminalStreamHealthSnapshot,
        slice_lifecycle: SliceLifecycleHealthSnapshot,
        remote_execution: RemoteExecutionHealthSnapshot,
        remote_extension_sync: RemoteExtensionSyncHealthSnapshot,
        workspace_coordination: WorkspaceCoordinationHealthSnapshot,
        workspace_live_sync: WorkspaceLiveSyncHealthSnapshot,
        projection_invariants: ProjectionInvariantHealthSnapshot,
    ) -> Self {
        // Compatibility: legacy clients may still read prompt counts from the
        // session projection object. The agent runtime projection is the
        // canonical health source for prompt work during the ownership
        // migration, so mirror its counts here until the old fields can be
        // retired from the wire shape.
        session_projection.active_prompts = agent_runtime_projection.active_prompts;
        session_projection.queued_prompts = agent_runtime_projection.queued_prompts;
        Self {
            metadata: ProjectionMetadata::new(1, last_event_id),
            session_command_lanes,
            agent_command_lanes,
            workflow_command_lanes,
            provider_runtime_lanes,
            provider_run_actor,
            process,
            capability_executor,
            session_projection,
            agent_runtime_projection,
            provider_catalog,
            provider_runs,
            transport,
            terminal_stream,
            slice_lifecycle,
            remote_execution,
            remote_extension_sync,
            workspace_coordination,
            workspace_live_sync,
            projection_invariants,
            app_lock: crate::runtime::app_lock::AppLockHealthSnapshot::default(),
        }
    }
}

#[cfg(test)]
mod tests;
