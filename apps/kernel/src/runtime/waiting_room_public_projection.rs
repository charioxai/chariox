use std::collections::HashMap;
use std::sync::{Mutex as StdMutex, OnceLock};

use arroba_relay::protocol::RelayKernelPresence;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    ExternalProviderSessionRecord, RelayStatus, RemoteMachineRecord, TerminalRecord,
    WaitingRoomLaunchTarget, WaitingRoomPublicAgentSummary, WaitingRoomPublicSessionSummary,
    WaitingRoomPublicSnapshot, WaitingRoomPublicWorkflowEdgeSummary,
    WaitingRoomPublicWorkflowEndpointSummary, WaitingRoomPublicWorkflowNodeSummary,
    WaitingRoomPublicWorkflowSummary,
};
use crate::runtime::metaagent_event::MetaagentEventStore;
use crate::runtime::waiting_room_activity::{
    waiting_room_agent_activity_summary, waiting_room_session_activity_summary,
    waiting_room_workflow_activity_summary,
};
use crate::runtime::workspace_git_common::{
    detect_git_branch, workspace_display_label, worktree_display_label,
};
use crate::session::{unix_epoch_ms, RuntimeSession};

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
    let sessions =
        waiting_room_session_summaries(runtime_sessions, metaagent_events, caller_user_id);
    let launch_target = infer_waiting_room_launch_target();
    let inventory_version = waiting_room_inventory_version(
        &sessions,
        &external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor.as_deref(),
        &relay_status,
        &remote_machines,
        &remote_kernels,
        &terminals,
        launch_target.as_ref(),
    )?;
    Ok(WaitingRoomPublicSnapshot {
        schema_version: 9,
        inventory_version,
        generated_at_ms,
        sessions,
        external_provider_sessions,
        external_provider_sessions_has_more,
        external_provider_sessions_next_cursor,
        relay_status,
        remote_machines,
        remote_kernels,
        terminals,
        launch_target,
    })
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
    sessions: &[WaitingRoomPublicSessionSummary],
    external_provider_sessions: &[ExternalProviderSessionRecord],
    external_provider_sessions_has_more: bool,
    external_provider_sessions_next_cursor: Option<&str>,
    relay_status: &RelayStatus,
    remote_machines: &[RemoteMachineRecord],
    remote_kernels: &[RelayKernelPresence],
    terminals: &[TerminalRecord],
    launch_target: Option<&WaitingRoomLaunchTarget>,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "sessions": sessions,
        "external_provider_sessions": external_provider_sessions,
        "external_provider_sessions_has_more": external_provider_sessions_has_more,
        "external_provider_sessions_next_cursor": external_provider_sessions_next_cursor,
        "relay_status": relay_status,
        "remote_machines": remote_machines,
        "remote_kernels": remote_kernels,
        "terminals": terminals,
        "launch_target": launch_target,
    }))
    .map_err(|error| DaemonError::LocalTransport {
        operation: "serialize waiting room inventory snapshot",
        message: error.to_string(),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(payload)))
}

fn waiting_room_session_summaries(
    sessions: Vec<RuntimeSession>,
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
                model: agent.primary_model().map(ToOwned::to_owned),
                variant: agent.primary_effort().map(ToOwned::to_owned),
                mode: effective_config.mode.as_str().to_string(),
                permission: Some(effective_config.permission_level.as_str().to_string()),
                workspace_id: workspace_id.clone(),
                worktree_id,
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id.clone()),
                worktree_label,
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
    use arroba_relay::protocol::RelayKernelPresence;

    use crate::agent::{AgentInstance, GridPosition};
    use crate::local::{RelayStatus, RemoteMachineRecord, RemoteMachineTrustStatus, TerminalType};
    use crate::runtime::metaagent_event::{MetaagentEventStore, NewMetaagentEvent};
    use crate::runtime::waiting_room_public_projection::{
        build_waiting_room_public_snapshot, waiting_room_session_summaries,
    };
    use crate::session::RuntimeSession;

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

        assert_eq!(snapshot.schema_version, 9);
        assert_eq!(snapshot.generated_at_ms, 42);
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot.external_provider_sessions.is_empty());
        assert_eq!(snapshot.remote_machines.len(), 1);
        assert_eq!(snapshot.remote_kernels.len(), 1);
        assert_eq!(snapshot.terminals.len(), 1);
        assert!(!snapshot.inventory_version.is_empty());
    }
}
