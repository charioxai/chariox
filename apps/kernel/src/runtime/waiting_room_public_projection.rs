use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chariox_relay::protocol::RelayKernelPresence;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    ExternalProviderSessionRecord, RelayStatus, RemoteMachineRecord, TerminalRecord,
    WaitingRoomAgentRuntimePlacement, WaitingRoomLaunchTarget, WaitingRoomPublicAgentSummary,
    WaitingRoomPublicProjectSummary, WaitingRoomPublicSessionSummary, WaitingRoomPublicSnapshot,
    WaitingRoomPublicWorkflowEdgeSummary, WaitingRoomPublicWorkflowEndpointSummary,
    WaitingRoomPublicWorkflowNodeSummary, WaitingRoomPublicWorkflowSummary,
};
use crate::runtime::metaagent_event::MetaagentEventStore;
use crate::runtime::waiting_room_activity::{
    waiting_room_agent_activity_summary, waiting_room_session_activity_summary,
    waiting_room_workflow_activity_summary,
};
use crate::runtime::workspace_git_common::{
    detect_git_branch, workspace_display_label, worktree_display_label,
};
use crate::session::{unix_epoch_ms, RuntimeProject, RuntimeSession};
use crate::slice::SliceRecord;

const WAITING_ROOM_GIT_LABEL_CACHE_TTL_MS: u64 = 30_000;
static LAUNCH_TARGET_CACHE: OnceLock<StdMutex<Option<CachedLaunchTarget>>> = OnceLock::new();
static WORKSPACE_LABEL_CACHE: OnceLock<StdMutex<HashMap<String, CachedWorktreeLabel>>> =
    OnceLock::new();
static WORKTREE_LABEL_CACHE: OnceLock<StdMutex<HashMap<(String, String), CachedWorktreeLabel>>> =
    OnceLock::new();
static GIT_BRANCH_CACHE: OnceLock<StdMutex<HashMap<String, CachedWorktreeLabel>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct CachedLaunchTarget {
    cwd: String,
    expires_at_ms: u64,
    target: Option<WaitingRoomLaunchTarget>,
}

#[derive(Debug, Clone)]
struct CachedWorktreeLabel {
    expires_at_ms: u64,
    label: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct WaitingRoomSessionSummaryProjectionStore {
    state: Arc<StdMutex<HashMap<String, CachedSessionSummaries>>>,
    snapshots: Arc<StdMutex<HashMap<String, CachedWaitingRoomPublicSnapshot>>>,
    #[cfg(test)]
    snapshot_build_count: Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone)]
struct CachedSessionSummaries {
    session_revision: u64,
    metaagent_event_revision: u64,
    projection_revision: u64,
    entries: HashMap<String, CachedSessionSummaryEntry>,
    summaries: Arc<[WaitingRoomPublicSessionSummary]>,
}

#[derive(Clone)]
struct CachedSessionSummaryEntry {
    source: Arc<RuntimeSession>,
    summary: WaitingRoomPublicSessionSummary,
}

struct ProjectedSessionSummaries {
    revision: u64,
    summaries: Arc<[WaitingRoomPublicSessionSummary]>,
}

#[derive(Clone)]
struct CachedWaitingRoomPublicSnapshot {
    session_summary_revision: u64,
    auxiliary_fingerprint: String,
    snapshot: WaitingRoomPublicSnapshot,
}

impl WaitingRoomSessionSummaryProjectionStore {
    fn project(
        &self,
        runtime_sessions: &[Arc<RuntimeSession>],
        session_revision: u64,
        metaagent_events: &MetaagentEventStore,
        external_working_agents: &BTreeMap<String, BTreeSet<String>>,
        caller_user_id: &str,
    ) -> ProjectedSessionSummaries {
        let metaagent_event_revision = metaagent_events.revision();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = state.get(caller_user_id).filter(|cached| {
            cached.session_revision == session_revision
                && cached.metaagent_event_revision == metaagent_event_revision
                && cached_session_sources_match(cached, runtime_sessions, caller_user_id)
        }) {
            return ProjectedSessionSummaries {
                revision: cached.projection_revision,
                summaries: Arc::clone(&cached.summaries),
            };
        }

        let previous = state.get(caller_user_id);
        let can_reuse_entries = previous
            .is_some_and(|cached| cached.metaagent_event_revision == metaagent_event_revision);
        let entries = runtime_sessions
            .iter()
            .filter(|session| session.has_member(caller_user_id))
            .map(|session| {
                let session_id = session.id().to_string();
                let previous_entry = previous.and_then(|cached| cached.entries.get(&session_id));
                let cached = can_reuse_entries
                    .then_some(previous_entry)
                    .flatten()
                    .filter(|cached| Arc::ptr_eq(&cached.source, session));
                let mut summary = cached.map_or_else(
                    || {
                        waiting_room_session_summaries_from_refs(
                            std::iter::once(session.as_ref()),
                            metaagent_events,
                            caller_user_id,
                        )
                        .into_iter()
                        .next()
                        .expect("visible waiting-room session should project")
                    },
                    |cached| cached.summary.clone(),
                );
                refresh_waiting_room_projected_activity(
                    &mut summary,
                    session,
                    external_working_agents.get(&session_id),
                    caller_user_id,
                );
                if cached.is_none() && can_reuse_entries {
                    if let Some(previous_entry) = previous_entry {
                        let last_used_at_ms = summary.last_used_at_ms;
                        summary.last_used_at_ms = previous_entry.summary.last_used_at_ms;
                        if summary != previous_entry.summary {
                            summary.last_used_at_ms = last_used_at_ms;
                        }
                    }
                }
                (
                    session_id,
                    CachedSessionSummaryEntry {
                        source: Arc::clone(session),
                        summary,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let projected = runtime_sessions
            .iter()
            .filter_map(|session| entries.get(session.id()).map(|entry| entry.summary.clone()))
            .collect::<Vec<_>>();
        let unchanged = previous.is_some_and(|cached| cached.summaries.as_ref() == projected);
        let projection_revision = previous.map_or(1, |cached| {
            if unchanged {
                cached.projection_revision
            } else {
                cached.projection_revision.saturating_add(1)
            }
        });
        let summaries = if unchanged {
            Arc::clone(
                &previous
                    .expect("unchanged projection has prior state")
                    .summaries,
            )
        } else {
            Arc::from(projected.into_boxed_slice())
        };
        state.insert(
            caller_user_id.to_string(),
            CachedSessionSummaries {
                session_revision,
                metaagent_event_revision,
                projection_revision,
                entries,
                summaries: Arc::clone(&summaries),
            },
        );
        ProjectedSessionSummaries {
            revision: projection_revision,
            summaries,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn project_snapshot(
        &self,
        runtime_sessions: &[Arc<RuntimeSession>],
        session_revision: u64,
        metaagent_events: &MetaagentEventStore,
        external_working_agents: &BTreeMap<String, BTreeSet<String>>,
        runtime_projects: &[RuntimeProject],
        slices: &[SliceRecord],
        external_provider_sessions: Vec<ExternalProviderSessionRecord>,
        external_provider_sessions_has_more: bool,
        external_provider_sessions_next_cursor: Option<String>,
        relay_status: RelayStatus,
        remote_machines: Vec<RemoteMachineRecord>,
        remote_kernels: Vec<RelayKernelPresence>,
        terminals: Vec<TerminalRecord>,
        generated_at_ms: u64,
        caller_user_id: &str,
    ) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
        let projected_sessions = self.project(
            runtime_sessions,
            session_revision,
            metaagent_events,
            external_working_agents,
            caller_user_id,
        );
        let auxiliary_fingerprint = waiting_room_snapshot_auxiliary_fingerprint(
            runtime_projects,
            slices,
            &external_provider_sessions,
            external_provider_sessions_has_more,
            external_provider_sessions_next_cursor.as_deref(),
            &relay_status,
            &remote_machines,
            &remote_kernels,
            &terminals,
        )?;
        let mut snapshots = self
            .snapshots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = snapshots.get(caller_user_id).filter(|cached| {
            cached.session_summary_revision == projected_sessions.revision
                && cached.auxiliary_fingerprint == auxiliary_fingerprint
        }) {
            let mut snapshot = cached.snapshot.clone();
            snapshot.generated_at_ms = generated_at_ms;
            return Ok(snapshot);
        }
        let mut sessions = projected_sessions
            .summaries
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let archived_project_ids = runtime_projects
            .iter()
            .filter(|project| project.status() == crate::session::RuntimeProjectStatus::Archived)
            .map(|project| project.id())
            .collect::<BTreeSet<_>>();
        sessions.retain(|session| {
            session.status != crate::session::SessionStatus::Ended
                || archived_project_ids.contains(session.project_id.as_str())
        });
        enrich_waiting_room_agent_slice_placements(&mut sessions, slices);
        let projects = waiting_room_public_project_summaries(
            runtime_projects,
            runtime_sessions,
            caller_user_id,
        );
        let snapshot = build_waiting_room_public_snapshot_from_summaries(
            sessions,
            projects,
            external_provider_sessions,
            external_provider_sessions_has_more,
            external_provider_sessions_next_cursor,
            relay_status,
            remote_machines,
            remote_kernels,
            terminals,
            generated_at_ms,
        )?;
        #[cfg(test)]
        self.snapshot_build_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        snapshots.insert(
            caller_user_id.to_string(),
            CachedWaitingRoomPublicSnapshot {
                session_summary_revision: projected_sessions.revision,
                auxiliary_fingerprint,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    #[cfg(test)]
    fn snapshot_build_count(&self) -> u64 {
        self.snapshot_build_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn cached_session_sources_match(
    cached: &CachedSessionSummaries,
    runtime_sessions: &[Arc<RuntimeSession>],
    caller_user_id: &str,
) -> bool {
    let mut visible_session_count = 0;
    for session in runtime_sessions
        .iter()
        .filter(|session| session.has_member(caller_user_id))
    {
        visible_session_count += 1;
        if !cached
            .entries
            .get(session.id())
            .is_some_and(|entry| Arc::ptr_eq(&entry.source, session))
        {
            return false;
        }
    }
    visible_session_count == cached.entries.len()
}

pub(crate) fn build_waiting_room_public_snapshot(
    runtime_sessions: Vec<RuntimeSession>,
    metaagent_events: &MetaagentEventStore,
    external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<String>,
    relay_status: RelayStatus,
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    terminals: Vec<TerminalRecord>,
    generated_at_ms: u64,
    caller_user_id: &str,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let runtime_sessions = runtime_sessions
        .into_iter()
        .map(Arc::new)
        .collect::<Vec<_>>();
    build_waiting_room_public_snapshot_from_shared(
        &runtime_sessions,
        metaagent_events,
        external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        generated_at_ms,
        caller_user_id,
    )
}

pub(crate) fn build_waiting_room_public_snapshot_from_shared(
    runtime_sessions: &[Arc<RuntimeSession>],
    metaagent_events: &MetaagentEventStore,
    external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<String>,
    relay_status: RelayStatus,
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    terminals: Vec<TerminalRecord>,
    generated_at_ms: u64,
    caller_user_id: &str,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let sessions = waiting_room_session_summaries_from_refs(
        runtime_sessions
            .iter()
            .map(AsRef::as_ref)
            .filter(|session| session.status() != crate::session::SessionStatus::Ended),
        metaagent_events,
        caller_user_id,
    );
    build_waiting_room_public_snapshot_from_summaries(
        sessions,
        Vec::new(),
        external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        generated_at_ms,
    )
}

pub(crate) fn build_waiting_room_public_snapshot_from_cached_shared(
    runtime_sessions: &[Arc<RuntimeSession>],
    session_revision: u64,
    summary_projection: &WaitingRoomSessionSummaryProjectionStore,
    metaagent_events: &MetaagentEventStore,
    external_working_agents: &BTreeMap<String, BTreeSet<String>>,
    runtime_projects: &[RuntimeProject],
    slices: &[SliceRecord],
    external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<String>,
    relay_status: RelayStatus,
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    terminals: Vec<TerminalRecord>,
    generated_at_ms: u64,
    caller_user_id: &str,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    summary_projection.project_snapshot(
        runtime_sessions,
        session_revision,
        metaagent_events,
        external_working_agents,
        runtime_projects,
        slices,
        external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        generated_at_ms,
        caller_user_id,
    )
}

fn waiting_room_snapshot_auxiliary_fingerprint(
    runtime_projects: &[RuntimeProject],
    slices: &[SliceRecord],
    external_provider_sessions: &[ExternalProviderSessionRecord],
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<&str>,
    relay_status: &RelayStatus,
    remote_machines: &[RemoteMachineRecord],
    remote_kernels: &[RelayKernelPresence],
    terminals: &[TerminalRecord],
) -> Result<String, DaemonError> {
    hash_waiting_room_version(
        "serialize waiting room snapshot auxiliary inputs",
        &serde_json::json!({
            "projects": runtime_projects,
            "slices": slices,
            "external_provider_sessions": external_provider_sessions,
            "external_provider_sessions_has_more": external_provider_sessions_has_more,
            "external_provider_sessions_next_cursor": external_provider_sessions_next_cursor,
            "relay_status": relay_status,
            "remote_machines": remote_machines,
            "remote_kernels": remote_kernels,
            "terminals": terminals,
        }),
    )
}

fn build_waiting_room_public_snapshot_from_summaries(
    sessions: Vec<WaitingRoomPublicSessionSummary>,
    projects: Vec<WaitingRoomPublicProjectSummary>,
    external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<String>,
    relay_status: RelayStatus,
    remote_machines: Vec<RemoteMachineRecord>,
    remote_kernels: Vec<RelayKernelPresence>,
    terminals: Vec<TerminalRecord>,
    generated_at_ms: u64,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let remote_machines = remote_machines
        .into_iter()
        .map(filter_remote_machine_product_providers)
        .collect::<Vec<_>>();
    let remote_kernels = remote_kernels
        .into_iter()
        .map(filter_remote_kernel_product_providers)
        .collect::<Vec<_>>();
    let launch_target = infer_waiting_room_launch_target();
    let structural_version = waiting_room_structural_version(
        &sessions,
        &projects,
        &external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor.as_deref(),
        launch_target.as_ref(),
    )?;
    let activity_revision =
        waiting_room_activity_revision(&sessions, &projects, &external_provider_sessions)?;
    let inventory_version = waiting_room_inventory_version(
        &structural_version,
        &activity_revision,
        &relay_status,
        &remote_machines,
        &remote_kernels,
        &terminals,
    )?;
    Ok(WaitingRoomPublicSnapshot {
        schema_version: 12,
        inventory_version,
        structural_version,
        activity_revision,
        generated_at_ms,
        sessions,
        projects,
        external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        launch_target,
        provider_accounts: Vec::new(),
    })
}

fn filter_remote_machine_product_providers(
    mut machine: RemoteMachineRecord,
) -> RemoteMachineRecord {
    crate::provider::retain_public_inventory_providers(&mut machine.available_providers);
    machine
}

fn filter_remote_kernel_product_providers(mut kernel: RelayKernelPresence) -> RelayKernelPresence {
    crate::provider::retain_public_inventory_providers(&mut kernel.available_providers);
    kernel
}

pub(crate) fn infer_waiting_room_launch_target() -> Option<WaitingRoomLaunchTarget> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_string = cwd.display().to_string();
    let now_ms = unix_epoch_ms();
    let cache = LAUNCH_TARGET_CACHE.get_or_init(|| StdMutex::new(None));
    let Ok(mut guard) = cache.lock() else {
        return compute_waiting_room_launch_target(&cwd, &cwd_string, now_ms);
    };
    if let Some(target) = guard
        .as_ref()
        .filter(|cached| cached.cwd == cwd_string && cached.expires_at_ms > now_ms)
        .map(|cached| cached.target.clone())
    {
        return target;
    }
    let target = compute_waiting_room_launch_target(&cwd, &cwd_string, now_ms);
    *guard = Some(CachedLaunchTarget {
        cwd: cwd_string,
        expires_at_ms: now_ms.saturating_add(WAITING_ROOM_GIT_LABEL_CACHE_TTL_MS),
        target: target.clone(),
    });
    target
}

fn compute_waiting_room_launch_target(
    cwd: &std::path::Path,
    cwd_string: &str,
    now_ms: u64,
) -> Option<WaitingRoomLaunchTarget> {
    let worktree = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| cwd_string.to_string());
    let workspace = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|common_dir| {
            if let Some(stripped) = common_dir.strip_suffix("/.git") {
                stripped.to_string()
            } else {
                worktree.clone()
            }
        })
        .unwrap_or_else(|| cwd_string.to_string());
    let branch = cached_git_branch(&worktree, now_ms);
    Some(WaitingRoomLaunchTarget {
        workspace_label: cached_workspace_label(&workspace),
        directory: Some(workspace.clone()),
        worktree_label: worktree_display_label(&worktree, &workspace, branch.as_deref()),
        workspace_id: workspace,
        worktree_id: worktree,
    })
}

fn waiting_room_inventory_version(
    structural_version: &str,
    activity_revision: &str,
    relay_status: &RelayStatus,
    remote_machines: &[RemoteMachineRecord],
    remote_kernels: &[RelayKernelPresence],
    terminals: &[TerminalRecord],
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "structural_version": structural_version,
        "activity_revision": activity_revision,
        "relay_status": relay_status,
        "remote_machines": remote_machines,
        "remote_kernels": remote_kernels,
        "terminals": terminals,
    }))
    .map_err(|error| DaemonError::LocalTransport {
        operation: "serialize waiting room inventory snapshot",
        message: error.to_string(),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(payload)))
}

fn waiting_room_structural_version(
    sessions: &[WaitingRoomPublicSessionSummary],
    projects: &[WaitingRoomPublicProjectSummary],
    external_provider_sessions: &[ExternalProviderSessionRecord],
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<&str>,
    launch_target: Option<&WaitingRoomLaunchTarget>,
) -> Result<String, DaemonError> {
    let mut sessions =
        serde_json::to_value(sessions).map_err(|error| DaemonError::LocalTransport {
            operation: "serialize waiting room structural inventory",
            message: error.to_string(),
        })?;
    if let Some(sessions) = sessions.as_array_mut() {
        for session in sessions {
            let Some(session) = session.as_object_mut() else {
                continue;
            };
            session.remove("activity");
            session.remove("last_used_at_ms");
            session.remove("last_prompt_sent_at_ms");
            session.remove("connected_cli_count");
            if let Some(agents) = session
                .get_mut("agents")
                .and_then(|value| value.as_array_mut())
            {
                for agent in agents {
                    if let Some(agent) = agent.as_object_mut() {
                        agent.remove("activity");
                        agent.remove("last_prompt_sent_at_ms");
                        agent.remove("metaagent_event_counts");
                    }
                }
            }
            if let Some(workflows) = session
                .get_mut("workflows")
                .and_then(|value| value.as_array_mut())
            {
                for workflow in workflows {
                    if let Some(workflow) = workflow.as_object_mut() {
                        workflow.remove("activity");
                    }
                }
            }
        }
    }
    let mut external_provider_sessions =
        serde_json::to_value(external_provider_sessions).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "serialize waiting room structural inventory",
                message: error.to_string(),
            }
        })?;
    if let Some(external_sessions) = external_provider_sessions.as_array_mut() {
        for external_session in external_sessions {
            if let Some(external_session) = external_session.as_object_mut() {
                external_session.remove("last_modified_at_ms");
            }
        }
    }
    hash_waiting_room_version(
        "serialize waiting room structural inventory",
        &serde_json::json!({
            "sessions": sessions,
            "projects": projects.iter().map(|project| serde_json::json!({
                "id": project.id,
                "owner_user_id": project.owner_user_id,
                "workspace_id": project.workspace_id,
                "name": project.name,
                "kind": project.kind,
                "status": project.status,
                "created_at_ms": project.created_at_ms,
                "updated_at_ms": project.updated_at_ms,
                "archived_at_ms": project.archived_at_ms,
                "session_count": project.session_count,
                "joined_collaborator_count": project.joined_collaborator_count,
                "pending_collaboration_invite_count": project.pending_collaboration_invite_count,
            })).collect::<Vec<_>>(),
            "external_provider_sessions": external_provider_sessions,
            "external_provider_sessions_has_more": external_provider_sessions_has_more,
            "external_provider_sessions_next_cursor": external_provider_sessions_next_cursor,
            "launch_target": launch_target,
        }),
    )
}

fn waiting_room_activity_revision(
    sessions: &[WaitingRoomPublicSessionSummary],
    projects: &[WaitingRoomPublicProjectSummary],
    external_provider_sessions: &[ExternalProviderSessionRecord],
) -> Result<String, DaemonError> {
    let activity = sessions
        .iter()
        .map(|session| {
            serde_json::json!({
                "id": session.id,
                "last_used_at_ms": session.last_used_at_ms,
                "last_prompt_sent_at_ms": session.last_prompt_sent_at_ms,
                "connected_cli_count": session.connected_cli_count,
                "activity": session.activity,
                "agents": session.agents.iter().map(|agent| serde_json::json!({
                    "id": agent.id,
                    "last_prompt_sent_at_ms": agent.last_prompt_sent_at_ms,
                    "activity": agent.activity,
                    "metaagent_event_counts": agent.metaagent_event_counts,
                })).collect::<Vec<_>>(),
                "workflows": session.workflows.iter().map(|workflow| serde_json::json!({
                    "id": workflow.id,
                    "activity": workflow.activity,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    hash_waiting_room_version(
        "serialize waiting room activity inventory",
        &serde_json::json!({
            "sessions": activity,
            "projects": projects.iter().map(|project| serde_json::json!({
                "id": project.id,
                "last_session_activity_at_ms": project.last_session_activity_at_ms,
            })).collect::<Vec<_>>(),
            "external_provider_sessions": external_provider_sessions.iter().map(|session| serde_json::json!({
                "external_session_id": session.external_session_id,
                "last_modified_at_ms": session.last_modified_at_ms,
            })).collect::<Vec<_>>(),
        }),
    )
}

fn hash_waiting_room_version(
    operation: &'static str,
    value: &serde_json::Value,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(value).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(payload)))
}

fn waiting_room_session_summaries(
    sessions: Vec<RuntimeSession>,
    metaagent_events: &MetaagentEventStore,
    caller_user_id: &str,
) -> Vec<WaitingRoomPublicSessionSummary> {
    waiting_room_session_summaries_from_refs(sessions.iter(), metaagent_events, caller_user_id)
}

fn refresh_waiting_room_projected_activity(
    summary: &mut WaitingRoomPublicSessionSummary,
    session: &RuntimeSession,
    external_working_agent_ids: Option<&BTreeSet<String>>,
    caller_user_id: &str,
) {
    summary.activity = waiting_room_session_activity_summary(session, caller_user_id);
    for agent_summary in &mut summary.agents {
        let Some(agent) = session
            .agents()
            .iter()
            .find(|agent| agent.id() == agent_summary.id)
        else {
            continue;
        };
        agent_summary.activity =
            waiting_room_agent_activity_summary(session, agent, caller_user_id);
    }
    let Some(external_working_agent_ids) = external_working_agent_ids else {
        return;
    };
    for agent_id in external_working_agent_ids {
        let Some(agent_summary) = summary
            .agents
            .iter_mut()
            .find(|agent| agent.id == *agent_id)
        else {
            continue;
        };
        if agent_summary.activity.error {
            continue;
        }
        let was_working = agent_summary.activity.working;
        let had_active_prompt = agent_summary.activity.active_prompt_count > 0;
        agent_summary.activity.working = true;
        agent_summary.activity.active_prompt_count =
            agent_summary.activity.active_prompt_count.max(1);
        agent_summary.activity.unread_idle_output = false;
        if !was_working {
            summary.activity.working_agent_count =
                summary.activity.working_agent_count.saturating_add(1);
        }
        if !had_active_prompt {
            summary.activity.active_prompt_count =
                summary.activity.active_prompt_count.saturating_add(1);
        }
    }
}

fn waiting_room_session_summaries_from_refs<'a>(
    sessions: impl IntoIterator<Item = &'a RuntimeSession>,
    metaagent_events: &MetaagentEventStore,
    caller_user_id: &str,
) -> Vec<WaitingRoomPublicSessionSummary> {
    let mut workspace_labels: HashMap<String, Option<String>> = HashMap::new();
    let mut worktree_labels: HashMap<(String, String), Option<String>> = HashMap::new();
    sessions
        .into_iter()
        .filter(|session| session.has_member(caller_user_id))
        .map(|session| {
            let workspace_id = session.workspace_id().to_string();
            let worktree_id = session.worktree_id().to_string();
            let workspace_label = workspace_labels
                .entry(workspace_id.clone())
                .or_insert_with(|| cached_workspace_label(&workspace_id))
                .clone();
            let worktree_label = worktree_labels
                .entry((workspace_id.clone(), worktree_id.clone()))
                .or_insert_with(|| cached_worktree_label(&worktree_id, &workspace_id))
                .clone();
            WaitingRoomPublicSessionSummary {
                id: session.id().to_string(),
                project_id: session.project_id().to_string(),
                alias: session.alias().map(ToOwned::to_owned),
                workspace_id: workspace_id.clone(),
                worktree_id: worktree_id.clone(),
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id),
                worktree_label,
                workspace_live_sync_mode: session.workspace_live_sync_mode(),
                created_at_ms: session.created_at_ms(),
                last_used_at_ms: session.last_used_at_ms(),
                last_prompt_sent_at_ms: session.last_prompt_sent_at_ms(),
                status: session.status(),
                connected_cli_count: session.attachment_ids().len(),
                joined_collaborator_count: session
                    .members()
                    .iter()
                    .filter(|member| member.user_id() != session.owner_user_id())
                    .count(),
                pending_collaboration_invite_count: pending_session_invite_count(session),
                activity: waiting_room_session_activity_summary(&session, caller_user_id),
                agents: waiting_room_public_agent_summaries(
                    &session,
                    metaagent_events,
                    workspace_label.clone(),
                    &mut worktree_labels,
                    caller_user_id,
                ),
                workflows: waiting_room_public_workflow_summaries(&session),
            }
        })
        .collect()
}

fn waiting_room_public_agent_summaries(
    session: &RuntimeSession,
    metaagent_events: &MetaagentEventStore,
    workspace_label: Option<String>,
    worktree_labels: &mut HashMap<(String, String), Option<String>>,
    caller_user_id: &str,
) -> Vec<WaitingRoomPublicAgentSummary> {
    let mut agents = session
        .agents()
        .iter()
        .map(|agent| {
            let effective_config =
                crate::session::effective_agent_execution_config(session, Some(agent));
            let workspace_id = agent
                .workspace_id()
                .unwrap_or_else(|| session.workspace_id())
                .to_string();
            let worktree_id = agent
                .worktree_id()
                .unwrap_or_else(|| session.worktree_id())
                .to_string();
            let worktree_label = worktree_labels
                .entry((workspace_id.clone(), worktree_id.clone()))
                .or_insert_with(|| cached_worktree_label(&worktree_id, &workspace_id))
                .clone();
            WaitingRoomPublicAgentSummary {
                id: agent.id().to_string(),
                agent_ref: agent.agent_ref().to_string(),
                alias: agent.alias().map(ToOwned::to_owned),
                created_at_ms: agent.created_at_ms(),
                last_prompt_sent_at_ms: agent.last_prompt_sent_at_ms(),
                provider: agent.primary_provider().to_string(),
                account_profile: agent.account_profile().unwrap_or("default").to_string(),
                account_label: None,
                model: agent.primary_model().map(ToOwned::to_owned),
                variant: agent.primary_effort().map(ToOwned::to_owned),
                mode: effective_config.mode.as_str().to_string(),
                permission: Some(effective_config.permission_level.as_str().to_string()),
                workspace_id: workspace_id.clone(),
                worktree_id,
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id.clone()),
                worktree_label,
                runtime_placement: waiting_room_agent_runtime_placement(session, agent),
                extension_grants: agent.extension_grants().to_vec(),
                activity: waiting_room_agent_activity_summary(session, agent, caller_user_id),
                metaagent_event_counts: agent
                    .is_metaagent()
                    .then(|| metaagent_events.counts(agent.id())),
            }
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    agents
}

fn waiting_room_agent_runtime_placement(
    session: &RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> WaitingRoomAgentRuntimePlacement {
    let (kernel_id, machine_id) = agent.remote_execution().map_or_else(
        || {
            (
                session.host_daemon_id().to_string(),
                session.host_machine_id().to_string(),
            )
        },
        |remote| {
            (
                remote.worker_kernel_id.clone(),
                remote.worker_machine_id.clone(),
            )
        },
    );
    WaitingRoomAgentRuntimePlacement {
        kernel_id,
        machine_id,
        slice_id: None,
        slice_name: None,
        slice_display_endpoint: None,
    }
}

fn pending_session_invite_count(session: &RuntimeSession) -> usize {
    let now_ms = unix_epoch_ms();
    session
        .invites()
        .iter()
        .filter(|invite| {
            !invite.is_revoked() && !invite.is_expired(now_ms) && !invite.is_exhausted()
        })
        .count()
}

fn enrich_waiting_room_agent_slice_placements(
    sessions: &mut [WaitingRoomPublicSessionSummary],
    slices: &[SliceRecord],
) {
    for session in sessions {
        for agent in &mut session.agents {
            let Some(slice) = slices.iter().find(|slice| {
                slice.agent_ids.iter().any(|agent_id| agent_id == &agent.id)
                    || (slice.agent_ids.is_empty()
                        && (slice.session_id.as_deref() == Some(session.id.as_str())
                            || slice.session_ids.iter().any(|id| id == &session.id))
                        && slice.worker_kernel_id.as_deref()
                            == Some(agent.runtime_placement.kernel_id.as_str()))
            }) else {
                continue;
            };
            agent.runtime_placement.slice_id = Some(slice.id.clone());
            agent.runtime_placement.slice_name = Some(slice.name.clone());
            agent.runtime_placement.slice_display_endpoint = slice.display_endpoint.clone();
            if !matches!(
                slice.status,
                crate::slice::SliceStatus::Starting | crate::slice::SliceStatus::Running
            ) && !agent.activity.error
            {
                agent.activity.error = true;
                if agent.activity.working {
                    agent.activity.working = false;
                    session.activity.working_agent_count =
                        session.activity.working_agent_count.saturating_sub(1);
                }
                session.activity.error_agent_count =
                    session.activity.error_agent_count.saturating_add(1);
            }
        }
    }
}

fn waiting_room_public_project_summaries(
    projects: &[RuntimeProject],
    sessions: &[Arc<RuntimeSession>],
    caller_user_id: &str,
) -> Vec<WaitingRoomPublicProjectSummary> {
    let mut summaries = projects
        .iter()
        .map(|project| {
            let project_sessions = sessions
                .iter()
                .map(AsRef::as_ref)
                .filter(|session| {
                    session.project_id() == project.id() && session.has_member(caller_user_id)
                })
                .collect::<Vec<_>>();
            WaitingRoomPublicProjectSummary {
                id: project.id().to_string(),
                owner_user_id: project.owner_user_id().to_string(),
                workspace_id: project.workspace_id().to_string(),
                name: project.name().to_string(),
                kind: project.kind(),
                status: project.status(),
                created_at_ms: project.created_at_ms(),
                updated_at_ms: project.updated_at_ms(),
                archived_at_ms: project.archived_at_ms(),
                session_count: project_sessions.len(),
                last_session_activity_at_ms: project_sessions
                    .iter()
                    .filter_map(|session| {
                        session
                            .last_prompt_sent_at_ms()
                            .or(session.last_used_at_ms())
                            .or(Some(session.created_at_ms()))
                    })
                    .max(),
                joined_collaborator_count: project_sessions
                    .iter()
                    .map(|session| {
                        session
                            .members()
                            .iter()
                            .filter(|member| member.user_id() != session.owner_user_id())
                            .count()
                    })
                    .sum(),
                pending_collaboration_invite_count: project_sessions
                    .iter()
                    .map(|session| pending_session_invite_count(session))
                    .sum(),
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .last_session_activity_at_ms
            .cmp(&left.last_session_activity_at_ms)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    summaries
}

fn cached_worktree_label(worktree_id: &str, workspace_id: &str) -> Option<String> {
    let now_ms = unix_epoch_ms();
    let key = (workspace_id.to_string(), worktree_id.to_string());
    let cache = WORKTREE_LABEL_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&key) {
            if cached.expires_at_ms > now_ms {
                return cached.label.clone();
            }
        }
    }
    let branch = cached_git_branch(worktree_id, now_ms);
    let label = worktree_display_label(worktree_id, workspace_id, branch.as_deref());
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            key,
            CachedWorktreeLabel {
                expires_at_ms: now_ms.saturating_add(WAITING_ROOM_GIT_LABEL_CACHE_TTL_MS),
                label: label.clone(),
            },
        );
    }
    label
}

fn cached_git_branch(worktree_id: &str, now_ms: u64) -> Option<String> {
    let cache = GIT_BRANCH_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(worktree_id) {
            if cached.expires_at_ms > now_ms {
                return cached.label.clone();
            }
        }
    }
    let branch = detect_git_branch(worktree_id).ok();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            worktree_id.to_string(),
            CachedWorktreeLabel {
                expires_at_ms: now_ms.saturating_add(WAITING_ROOM_GIT_LABEL_CACHE_TTL_MS),
                label: branch.clone(),
            },
        );
    }
    branch
}

fn cached_workspace_label(workspace_id: &str) -> Option<String> {
    let now_ms = unix_epoch_ms();
    let cache = WORKSPACE_LABEL_CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(workspace_id) {
            if cached.expires_at_ms > now_ms {
                return cached.label.clone();
            }
        }
    }
    let label = workspace_display_label(workspace_id);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            workspace_id.to_string(),
            CachedWorktreeLabel {
                expires_at_ms: now_ms.saturating_add(WAITING_ROOM_GIT_LABEL_CACHE_TTL_MS),
                label: label.clone(),
            },
        );
    }
    label
}

fn waiting_room_public_workflow_summaries(
    session: &RuntimeSession,
) -> Vec<WaitingRoomPublicWorkflowSummary> {
    let mut workflows = session
        .workflows()
        .iter()
        .map(|workflow| WaitingRoomPublicWorkflowSummary {
            id: workflow.id().to_string(),
            alias: workflow.alias().map(ToOwned::to_owned),
            prompt: workflow.prompt().map(ToOwned::to_owned),
            created_at_ms: workflow.created_at_ms(),
            revision: workflow.revision(),
            canvas_layout: workflow.canvas_layout().cloned(),
            activity: waiting_room_workflow_activity_summary(session, workflow.id()),
            nodes: workflow
                .nodes()
                .iter()
                .map(|node| WaitingRoomPublicWorkflowNodeSummary {
                    id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    label: node.public_label().to_string(),
                    wait_for_all_inputs: node.wait_for_all_inputs(),
                })
                .collect(),
            edges: workflow
                .edges()
                .iter()
                .map(|edge| WaitingRoomPublicWorkflowEdgeSummary {
                    id: edge.id().to_string(),
                    from_node_id: edge.from_node_id().to_string(),
                    to_node_id: edge.to_node_id().to_string(),
                })
                .collect(),
            endpoints: workflow
                .endpoints()
                .iter()
                .map(|endpoint| WaitingRoomPublicWorkflowEndpointSummary {
                    id: endpoint.id().to_string(),
                    alias: endpoint.alias().map(ToOwned::to_owned),
                    entry_node_id: endpoint.entry_node_id().to_string(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    workflows.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    workflows
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use chariox_relay::protocol::RelayKernelPresence;

    use crate::agent::{AgentInstance, GridPosition, RemoteAgentBinding};
    use crate::local::{RelayStatus, RemoteMachineRecord, RemoteMachineTrustStatus, TerminalType};
    use crate::runtime::metaagent_event::{MetaagentEventStore, NewMetaagentEvent};
    use crate::runtime::waiting_room_public_projection::{
        build_waiting_room_public_snapshot, build_waiting_room_public_snapshot_from_cached_shared,
        enrich_waiting_room_agent_slice_placements, waiting_room_activity_revision,
        waiting_room_session_summaries, waiting_room_structural_version,
        WaitingRoomSessionSummaryProjectionStore,
    };
    use crate::session::{
        RuntimeProject, RuntimeProjectKind, RuntimeSession, SessionStatus, WorkflowDefinition,
        WorkflowRun, WorkflowRunStatus,
    };

    fn slice_with_agent(
        id: &str,
        agent_id: &str,
        status: crate::slice::SliceStatus,
    ) -> crate::slice::SliceRecord {
        crate::slice::SliceRecord {
            id: id.to_string(),
            name: id.to_string(),
            owner_kernel_id: "daemon".to_string(),
            owner_machine_id: "machine".to_string(),
            session_id: Some("session-slice".to_string()),
            session_ids: vec!["session-slice".to_string()],
            agent_ids: vec![agent_id.to_string()],
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            status,
            last_operation: None,
            last_operation_status: None,
            last_error: None,
            last_operation_at_ms: None,
            workspace_id: Some("workspace".to_string()),
            worktree_id: Some("worktree".to_string()),
            workspace_mount: Some("workspace".to_string()),
            worker_kernel_ref: format!("slice:{id}"),
            worker_kernel_id: Some("worker-kernel".to_string()),
            worker_machine_id: Some("worker-machine".to_string()),
            relay_endpoint: None,
            local_docker_ports: None,
            providers: Vec::new(),
            provider_auth: Vec::new(),
            saved_state_ref: None,
            saved_state_status: None,
            saved_state_updated_at_ms: None,
            display_endpoint: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn disconnected_relay_status() -> RelayStatus {
        RelayStatus {
            configured: false,
            connected: false,
            relay_url: None,
            relay_token_configured: false,
            daemon_id: "daemon".to_string(),
            daemon_alias: None,
            machine_id: "machine".to_string(),
            machine_alias: None,
        }
    }

    #[test]
    fn stopped_slice_agents_project_error_consistently_in_waiting_room() {
        let mut session = RuntimeSession::new(
            "session-slice",
            Some("slice session".to_string()),
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        let mut agent = AgentInstance::new(
            "agent-slice",
            "A1",
            "session-slice",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_remote_execution(Some(RemoteAgentBinding {
            worker_kernel_id: "worker-kernel".to_string(),
            worker_machine_id: "worker-machine".to_string(),
            execution_lease_id: "lease-slice".to_string(),
            leased_agent_id: "leased-slice".to_string(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        }));
        session.set_agents(vec![agent]);
        let metaagent_events = MetaagentEventStore::default();
        let mut summaries = waiting_room_session_summaries(
            vec![session],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );

        assert_eq!(summaries[0].activity.error_agent_count, 0);
        assert!(!summaries[0].agents[0].activity.error);

        enrich_waiting_room_agent_slice_placements(
            &mut summaries,
            &[slice_with_agent(
                "slice-stopped",
                "agent-slice",
                crate::slice::SliceStatus::Stopped,
            )],
        );

        assert_eq!(summaries[0].activity.error_agent_count, 1);
        assert_eq!(summaries[0].activity.working_agent_count, 0);
        assert!(summaries[0].agents[0].activity.error);
        assert!(!summaries[0].agents[0].activity.working);
        assert_eq!(
            summaries[0].agents[0].runtime_placement.slice_id.as_deref(),
            Some("slice-stopped")
        );
    }

    #[test]
    fn running_and_starting_slice_agents_do_not_project_false_errors() {
        for status in [
            crate::slice::SliceStatus::Starting,
            crate::slice::SliceStatus::Running,
        ] {
            let mut session = RuntimeSession::new(
                "session-slice",
                None,
                "workspace",
                "worktree",
                "machine",
                "daemon",
            );
            let mut agent = AgentInstance::new(
                "agent-slice",
                "A1",
                "session-slice",
                None,
                "codex",
                None,
                None,
                None,
                GridPosition::new(0, 0, 1, 1),
            );
            agent.set_remote_execution(Some(RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-slice".to_string(),
                leased_agent_id: "leased-slice".to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            }));
            session.set_agents(vec![agent]);
            let mut summaries = waiting_room_session_summaries(
                vec![session],
                &MetaagentEventStore::default(),
                crate::session::DEFAULT_LOCAL_USER_ID,
            );

            enrich_waiting_room_agent_slice_placements(
                &mut summaries,
                &[slice_with_agent("slice-live", "agent-slice", status)],
            );

            assert_eq!(summaries[0].activity.error_agent_count, 0);
            assert!(!summaries[0].agents[0].activity.error);
        }
    }

    #[test]
    fn archived_project_retains_ended_rows_while_ordinary_ended_rows_stay_hidden() {
        let mut session = RuntimeSession::new(
            "session-project",
            Some("project-session".to_string()),
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        assert!(session.assign_project_id("project-1"));
        session.touch();
        let initial_activity_at_ms = session.last_used_at_ms();
        let mut project = RuntimeProject::new(
            "project-1",
            crate::session::DEFAULT_LOCAL_USER_ID,
            "workspace",
            "owner/repo",
            RuntimeProjectKind::Default,
        );
        let mut ordinary_ended_session = RuntimeSession::new(
            "ordinary-ended-session",
            Some("ordinary-ended".to_string()),
            "workspace",
            "ordinary-worktree",
            "machine",
            "daemon",
        );
        assert!(ordinary_ended_session.assign_project_id("project-2"));
        assert!(ordinary_ended_session.transition_to(SessionStatus::Ended));
        let ordinary_active_project = RuntimeProject::new(
            "project-2",
            crate::session::DEFAULT_LOCAL_USER_ID,
            "workspace",
            "Still active",
            RuntimeProjectKind::Named,
        );
        let metaagent_events = MetaagentEventStore::default();
        let projection = WaitingRoomSessionSummaryProjectionStore::default();

        let active_sessions = vec![
            Arc::new(session.clone()),
            Arc::new(ordinary_ended_session.clone()),
        ];
        let active = build_waiting_room_public_snapshot_from_cached_shared(
            &active_sessions,
            1,
            &projection,
            &metaagent_events,
            &BTreeMap::new(),
            &[project.clone(), ordinary_active_project.clone()],
            &[],
            Vec::new(),
            false,
            None,
            disconnected_relay_status(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("active project should project");
        assert_eq!(active.sessions.len(), 1);
        assert_eq!(active.sessions[0].id, "session-project");
        let active_project = active
            .projects
            .iter()
            .find(|summary| summary.id == "project-1")
            .expect("active project summary should exist");
        assert_eq!(active_project.session_count, 1);
        assert_eq!(
            active_project.last_session_activity_at_ms,
            initial_activity_at_ms
        );

        assert!(session.transition_to(SessionStatus::Ended));
        project.archive();
        let archived_sessions = vec![
            Arc::new(session.clone()),
            Arc::new(ordinary_ended_session.clone()),
        ];
        let archived = build_waiting_room_public_snapshot_from_cached_shared(
            &archived_sessions,
            2,
            &projection,
            &metaagent_events,
            &BTreeMap::new(),
            &[project.clone(), ordinary_active_project.clone()],
            &[],
            Vec::new(),
            false,
            None,
            disconnected_relay_status(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            2,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("archived project should project");
        assert_eq!(archived.sessions.len(), 1);
        assert_eq!(archived.sessions[0].id, "session-project");
        assert_eq!(archived.sessions[0].status, SessionStatus::Ended);
        let archived_project = archived
            .projects
            .iter()
            .find(|summary| summary.id == "project-1")
            .expect("archived project summary should exist");
        assert_eq!(archived_project.session_count, 1);
        assert_eq!(
            archived_project.last_session_activity_at_ms,
            initial_activity_at_ms
        );

        assert!(session.transition_to(SessionStatus::Parked));
        project.restore();
        let restored_sessions = vec![Arc::new(session), Arc::new(ordinary_ended_session)];
        let restored = build_waiting_room_public_snapshot_from_cached_shared(
            &restored_sessions,
            3,
            &projection,
            &metaagent_events,
            &BTreeMap::new(),
            &[project, ordinary_active_project],
            &[],
            Vec::new(),
            false,
            None,
            disconnected_relay_status(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            3,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("restored project should project");
        assert_eq!(restored.sessions.len(), 1);
        assert_eq!(restored.sessions[0].id, "session-project");
        assert_eq!(restored.sessions[0].status, SessionStatus::Parked);
        assert_eq!(
            restored
                .projects
                .iter()
                .find(|summary| summary.id == "project-1")
                .expect("restored project summary should exist")
                .session_count,
            1
        );
    }

    #[test]
    fn waiting_room_session_summaries_project_workspace_metadata() {
        let mut session = RuntimeSession::new(
            "session-1",
            Some("alias".to_string()),
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        session.set_workspace_live_sync_mode(Some(crate::config::WorkspaceLiveSyncMode::Managed));
        session.add_attachment("cli-1".to_string());

        let metaagent_events = MetaagentEventStore::default();
        let summaries = waiting_room_session_summaries(
            vec![session],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );

        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.id, "session-1");
        assert_eq!(summary.alias.as_deref(), Some("alias"));
        assert_eq!(summary.workspace_id, "workspace");
        assert_eq!(summary.worktree_id, "worktree");
        assert_eq!(summary.directory.as_deref(), Some("workspace"));
        assert_eq!(
            summary.workspace_live_sync_mode,
            Some(crate::config::WorkspaceLiveSyncMode::Managed)
        );
        assert_eq!(summary.connected_cli_count, 1);
        assert_eq!(summary.activity.agent_count, 0);
    }

    #[test]
    fn waiting_room_session_summaries_project_metaagent_event_counts() {
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        let mut metaagent = AgentInstance::new(
            "meta-1",
            "M1",
            "session-1",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        metaagent.activate_meta_mode(None);
        let worker = AgentInstance::new(
            "agent-1",
            "A1",
            "session-1",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(1, 0, 1, 1),
        );
        session.set_agents(vec![metaagent.clone(), worker]);
        let metaagent_events = MetaagentEventStore::default();
        let event = metaagent_events.record(NewMetaagentEvent {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            owner_user_id: metaagent.owner_user_id().to_string(),
            kind: "agent.turn.completed".to_string(),
            source_agent_id: Some("agent-1".to_string()),
            title: "Worker completed".to_string(),
            summary: "Worker completed a turn".to_string(),
            detail: serde_json::json!({ "prompt_id": "prompt-1" }),
            injected_prompt_id: None,
        });
        metaagent_events
            .read(metaagent.id(), &event.event_id)
            .expect("event should read");

        let summaries = waiting_room_session_summaries(
            vec![session],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );

        let agents = &summaries[0].agents;
        let metaagent_summary = agents
            .iter()
            .find(|agent| agent.id == "meta-1")
            .expect("metaagent summary should project");
        assert_eq!(
            metaagent_summary
                .metaagent_event_counts
                .as_ref()
                .and_then(|counts| counts.pointer("/total"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            metaagent_summary
                .metaagent_event_counts
                .as_ref()
                .and_then(|counts| counts.pointer("/unread"))
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert_eq!(
            metaagent_summary
                .metaagent_event_counts
                .as_ref()
                .and_then(|counts| counts.pointer("/unacked"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            metaagent_summary
                .metaagent_event_counts
                .as_ref()
                .and_then(|counts| counts.pointer("/by_kind/agent.turn.completed"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(agents
            .iter()
            .find(|agent| agent.id == "agent-1")
            .expect("regular agent summary should project")
            .metaagent_event_counts
            .is_none());
    }

    #[test]
    fn waiting_room_session_summaries_project_workflow_prompt() {
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        let mut workflow = WorkflowDefinition::new("workflow-1", Some("review".to_string()));
        workflow.set_prompt(Some("Shared workflow context".to_string()));
        session.create_workflow(workflow);

        let metaagent_events = MetaagentEventStore::default();
        let summaries = waiting_room_session_summaries(
            vec![session],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );

        assert_eq!(summaries[0].workflows.len(), 1);
        assert_eq!(
            summaries[0].workflows[0].prompt.as_deref(),
            Some("Shared workflow context")
        );
    }

    #[test]
    fn same_revision_cache_reprojects_completed_workflow_source() {
        fn session_with_workflow_status(status: WorkflowRunStatus) -> RuntimeSession {
            let mut session = RuntimeSession::new(
                "session-1",
                None,
                "workspace",
                "worktree",
                "machine",
                "daemon",
            );
            session.create_workflow(WorkflowDefinition::new(
                "workflow-1",
                Some("review".to_string()),
            ));
            let mut run = WorkflowRun::new(
                "workflow-run-1",
                "workflow-1",
                "endpoint-1",
                "node-1",
                None,
                None,
                Vec::new(),
                Vec::new(),
            );
            run.set_status(status);
            session.create_workflow_run(run);
            session
        }

        let metaagent_events = MetaagentEventStore::default();
        let projection = WaitingRoomSessionSummaryProjectionStore::default();
        let build = |session: RuntimeSession, generated_at_ms| {
            build_waiting_room_public_snapshot_from_cached_shared(
                &[Arc::new(session)],
                2,
                &projection,
                &metaagent_events,
                &BTreeMap::new(),
                &[],
                &[],
                Vec::new(),
                false,
                None,
                disconnected_relay_status(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                generated_at_ms,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("waiting-room workflow activity should project")
        };

        // This deliberately reproduces the former race: an older source was cached with the
        // post-completion revision, followed by the completed source at that same revision.
        let running = build(
            session_with_workflow_status(WorkflowRunStatus::Running),
            100,
        );
        let completed = build(
            session_with_workflow_status(WorkflowRunStatus::Completed),
            200,
        );

        assert!(running.sessions[0].workflows[0].activity.working);
        assert!(!completed.sessions[0].workflows[0].activity.working);
        assert_ne!(running.activity_revision, completed.activity_revision);
        assert_ne!(running.inventory_version, completed.inventory_version);

        let event = crate::transport::kernel_protocol::waiting_room_rows_changed_event(
            completed,
            Some(&running),
        )
        .expect("workflow completion should publish a waiting-room row delta");
        let crate::transport::kernel_protocol::KernelEvent::WaitingRoomRowsChanged {
            sessions, ..
        } = event
        else {
            panic!("workflow completion should emit waiting_room_rows_changed");
        };
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].workflows[0].activity.working);
    }

    #[test]
    fn session_summary_projection_builds_once_per_logical_revision() {
        let sessions = vec![Arc::new(RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        ))];
        let metaagent_events = MetaagentEventStore::default();
        let projection = WaitingRoomSessionSummaryProjectionStore::default();

        let first = projection.project(
            &sessions,
            7,
            &metaagent_events,
            &BTreeMap::new(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        let same_revision = projection.project(
            &sessions,
            7,
            &metaagent_events,
            &BTreeMap::new(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert!(Arc::ptr_eq(&first.summaries, &same_revision.summaries));
        assert_eq!(first.revision, same_revision.revision);

        let next_session_revision = projection.project(
            &sessions,
            8,
            &metaagent_events,
            &BTreeMap::new(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert!(Arc::ptr_eq(
            &first.summaries,
            &next_session_revision.summaries
        ));
        assert_eq!(first.revision, next_session_revision.revision);

        let touched_sessions = vec![Arc::new({
            let mut session = sessions[0].as_ref().clone();
            session.touch();
            session
        })];
        let touched_session_revision = projection.project(
            &touched_sessions,
            9,
            &metaagent_events,
            &BTreeMap::new(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert!(Arc::ptr_eq(
            &first.summaries,
            &touched_session_revision.summaries
        ));
        assert_eq!(first.revision, touched_session_revision.revision);
    }

    #[test]
    fn session_summary_projection_tracks_external_observed_work() {
        let mut session = RuntimeSession::new(
            "session-external",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        session.set_agents(vec![AgentInstance::new(
            "agent-external",
            "A1",
            "session-external",
            None,
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        )]);
        let sessions = vec![Arc::new(session)];
        let metaagent_events = MetaagentEventStore::default();
        let projection = WaitingRoomSessionSummaryProjectionStore::default();
        let external_working_agents = BTreeMap::from([(
            "session-external".to_string(),
            BTreeSet::from(["agent-external".to_string()]),
        )]);

        let working = projection.project(
            &sessions,
            7,
            &metaagent_events,
            &external_working_agents,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert_eq!(working.summaries[0].activity.working_agent_count, 1);
        assert_eq!(working.summaries[0].activity.active_prompt_count, 1);
        assert!(working.summaries[0].agents[0].activity.working);
        assert_eq!(
            working.summaries[0].agents[0].activity.active_prompt_count,
            1
        );

        let settled = projection.project(
            &sessions,
            8,
            &metaagent_events,
            &BTreeMap::new(),
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert_eq!(settled.summaries[0].activity.working_agent_count, 0);
        assert_eq!(settled.summaries[0].activity.active_prompt_count, 0);
        assert!(!settled.summaries[0].agents[0].activity.working);
        assert!(settled.revision > working.revision);
    }

    #[test]
    fn public_snapshot_projection_is_shared_across_subscribers_for_one_revision() {
        let sessions = vec![Arc::new(RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        ))];
        let metaagent_events = MetaagentEventStore::default();
        let projection = WaitingRoomSessionSummaryProjectionStore::default();
        let relay_status = RelayStatus {
            configured: false,
            connected: false,
            relay_url: None,
            relay_token_configured: false,
            daemon_id: "daemon".to_string(),
            daemon_alias: None,
            machine_id: "machine".to_string(),
            machine_alias: None,
        };
        let build = |revision, generated_at_ms| {
            build_waiting_room_public_snapshot_from_cached_shared(
                &sessions,
                revision,
                &projection,
                &metaagent_events,
                &BTreeMap::new(),
                &[],
                &[],
                Vec::new(),
                false,
                None,
                relay_status.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                generated_at_ms,
                crate::session::DEFAULT_LOCAL_USER_ID,
            )
            .expect("waiting room snapshot should project")
        };

        let first = build(7, 100);
        let second_subscriber = build(7, 200);
        assert_eq!(projection.snapshot_build_count(), 1);
        assert_eq!(first.inventory_version, second_subscriber.inventory_version);
        assert_eq!(second_subscriber.generated_at_ms, 200);

        let unchanged_projection = build(8, 300);
        assert_eq!(projection.snapshot_build_count(), 1);
        assert_eq!(
            first.inventory_version,
            unchanged_projection.inventory_version
        );

        let touched_sessions = vec![Arc::new({
            let mut session = sessions[0].as_ref().clone();
            session.touch();
            session
        })];
        let touched = build_waiting_room_public_snapshot_from_cached_shared(
            &touched_sessions,
            9,
            &projection,
            &metaagent_events,
            &BTreeMap::new(),
            &[],
            &[],
            Vec::new(),
            false,
            None,
            relay_status.clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            400,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("timestamp-only waiting room snapshot should project");
        assert_eq!(projection.snapshot_build_count(), 1);
        assert_eq!(first.inventory_version, touched.inventory_version);

        let changed_sessions = vec![Arc::new({
            let mut session = RuntimeSession::new(
                "session-1",
                None,
                "workspace",
                "worktree",
                "machine",
                "daemon",
            );
            session.set_alias(Some("renamed".to_string()));
            session
        })];
        let changed = build_waiting_room_public_snapshot_from_cached_shared(
            &changed_sessions,
            10,
            &projection,
            &metaagent_events,
            &BTreeMap::new(),
            &[],
            &[],
            Vec::new(),
            false,
            None,
            relay_status,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            500,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("changed waiting room snapshot should project");
        assert_eq!(projection.snapshot_build_count(), 2);
        assert_ne!(first.inventory_version, changed.inventory_version);
    }

    #[test]
    fn waiting_room_public_snapshot_inventory_version_includes_projection_inputs() {
        let metaagent_events = MetaagentEventStore::default();
        let snapshot = build_waiting_room_public_snapshot(
            vec![RuntimeSession::new(
                "session-1",
                None,
                "workspace",
                "worktree",
                "machine",
                "daemon",
            )],
            &metaagent_events,
            Vec::new(),
            false,
            None,
            RelayStatus {
                configured: false,
                connected: false,
                relay_url: None,
                relay_token_configured: false,
                daemon_id: "daemon".to_string(),
                daemon_alias: None,
                machine_id: "machine".to_string(),
                machine_alias: None,
            },
            vec![RemoteMachineRecord {
                machine_id: "machine-peer".to_string(),
                machine_alias: Some("peer".to_string()),
                registry_alias: None,
                display_name: "peer".to_string(),
                trust_status: RemoteMachineTrustStatus::Approved,
                online: true,
                pending: false,
                kernel_count: 1,
                available_providers: vec!["dev-stub".to_string()],
                provider_accounts: Vec::new(),
            }],
            vec![RelayKernelPresence {
                kernel_id: "kernel-peer".to_string(),
                machine_id: "machine-peer".to_string(),
                machine_alias: Some("peer".to_string()),
                relay_alias: None,
                kernel_alias: Some("peer-kernel".to_string()),
                available_providers: vec!["dev-stub".to_string()],
                provider_accounts: Vec::new(),
                capabilities: vec!["remote_leases".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
                public_key: "peer-public-key".to_string(),
            }],
            vec![crate::local::TerminalRecord {
                terminal_id: "terminal-1".to_string(),
                terminal_type: TerminalType::Cli,
                alias: Some("local".to_string()),
                paired_at_ms: 7,
                revoked: false,
            }],
            42,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .expect("snapshot builds");

        assert_eq!(snapshot.schema_version, 12);
        assert_eq!(snapshot.generated_at_ms, 42);
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot.external_provider_sessions.is_empty());
        assert_eq!(snapshot.remote_machines.len(), 1);
        assert!(snapshot.remote_machines[0].available_providers.is_empty());
        assert_eq!(snapshot.remote_kernels.len(), 1);
        assert!(snapshot.remote_kernels[0].available_providers.is_empty());
        assert_eq!(snapshot.terminals.len(), 1);
        assert!(!snapshot.inventory_version.is_empty());
        assert!(!snapshot.structural_version.is_empty());
        assert!(!snapshot.activity_revision.is_empty());
    }

    #[test]
    fn waiting_room_versions_separate_structure_from_activity() {
        let metaagent_events = MetaagentEventStore::default();
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "daemon",
        );
        let initial = waiting_room_session_summaries(
            vec![session.clone()],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );

        session.note_prompt_sent_at("missing-agent", 42);
        let activity_only = waiting_room_session_summaries(
            vec![session.clone()],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert_eq!(
            waiting_room_structural_version(&initial, &[], &[], false, None, None).unwrap(),
            waiting_room_structural_version(&activity_only, &[], &[], false, None, None).unwrap(),
        );
        assert_ne!(
            waiting_room_activity_revision(&initial, &[], &[]).unwrap(),
            waiting_room_activity_revision(&activity_only, &[], &[]).unwrap(),
        );

        session.set_alias(Some("renamed".to_string()));
        let structural_change = waiting_room_session_summaries(
            vec![session],
            &metaagent_events,
            crate::session::DEFAULT_LOCAL_USER_ID,
        );
        assert_ne!(
            waiting_room_structural_version(&activity_only, &[], &[], false, None, None).unwrap(),
            waiting_room_structural_version(&structural_change, &[], &[], false, None, None)
                .unwrap(),
        );
    }
}
