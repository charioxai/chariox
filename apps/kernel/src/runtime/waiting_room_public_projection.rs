use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::local::{
    RelayStatus, TerminalRecord, WaitingRoomLaunchTarget, WaitingRoomPublicAgentSummary,
    WaitingRoomPublicSessionSummary, WaitingRoomPublicSnapshot,
    WaitingRoomPublicWorkflowEdgeSummary, WaitingRoomPublicWorkflowEndpointSummary,
    WaitingRoomPublicWorkflowNodeSummary, WaitingRoomPublicWorkflowSummary,
};
use crate::runtime::waiting_room_activity::{
    waiting_room_agent_activity_summary, waiting_room_session_activity_summary,
    waiting_room_workflow_activity_summary,
};
use crate::runtime::workspace_git_common::{
    detect_git_branch, workspace_display_label, worktree_display_label,
};
use crate::session::RuntimeSession;

pub(crate) fn build_waiting_room_public_snapshot(
    runtime_sessions: Vec<RuntimeSession>,
    relay_status: RelayStatus,
    terminals: Vec<TerminalRecord>,
    generated_at_ms: u64,
) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
    let sessions = waiting_room_session_summaries(runtime_sessions);
    let launch_target = infer_waiting_room_launch_target();
    let inventory_version = waiting_room_inventory_version(
        &sessions,
        &relay_status,
        &terminals,
        launch_target.as_ref(),
    )?;
    Ok(WaitingRoomPublicSnapshot {
        schema_version: 5,
        inventory_version,
        generated_at_ms,
        sessions,
        relay_status,
        terminals,
        launch_target,
    })
}

pub(crate) fn infer_waiting_room_launch_target() -> Option<WaitingRoomLaunchTarget> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_string = cwd.display().to_string();
    let worktree = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| cwd_string.clone());
    let workspace = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(&cwd)
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
        .unwrap_or_else(|| cwd_string.clone());
    let branch = detect_git_branch(&worktree).ok();
    Some(WaitingRoomLaunchTarget {
        workspace_label: workspace_display_label(&workspace),
        directory: Some(workspace.clone()),
        worktree_label: worktree_display_label(&worktree, &workspace, branch.as_deref()),
        workspace_id: workspace,
        worktree_id: worktree,
    })
}

fn waiting_room_inventory_version(
    sessions: &[WaitingRoomPublicSessionSummary],
    relay_status: &RelayStatus,
    terminals: &[TerminalRecord],
    launch_target: Option<&WaitingRoomLaunchTarget>,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "sessions": sessions,
        "relay_status": relay_status,
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
) -> Vec<WaitingRoomPublicSessionSummary> {
    let mut workspace_labels: HashMap<String, Option<String>> = HashMap::new();
    let mut worktree_labels: HashMap<(String, String), Option<String>> = HashMap::new();
    sessions
        .into_iter()
        .map(|session| {
            let workspace_id = session.workspace_id().to_string();
            let worktree_id = session.worktree_id().to_string();
            let workspace_label = workspace_labels
                .entry(workspace_id.clone())
                .or_insert_with(|| workspace_display_label(&workspace_id))
                .clone();
            let worktree_label = worktree_labels
                .entry((workspace_id.clone(), worktree_id.clone()))
                .or_insert_with(|| {
                    let branch = detect_git_branch(&worktree_id).ok();
                    worktree_display_label(&worktree_id, &workspace_id, branch.as_deref())
                })
                .clone();
            WaitingRoomPublicSessionSummary {
                id: session.id().to_string(),
                alias: session.alias().map(ToOwned::to_owned),
                workspace_id: workspace_id.clone(),
                worktree_id: worktree_id.clone(),
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id),
                worktree_label,
                created_at_ms: session.created_at_ms(),
                last_used_at_ms: session.last_used_at_ms(),
                status: session.status(),
                connected_cli_count: session.attachment_ids().len(),
                activity: waiting_room_session_activity_summary(&session),
                agents: waiting_room_public_agent_summaries(
                    &session,
                    workspace_label.clone(),
                    &mut worktree_labels,
                ),
                workflows: waiting_room_public_workflow_summaries(&session),
            }
        })
        .collect()
}

fn waiting_room_public_agent_summaries(
    session: &RuntimeSession,
    workspace_label: Option<String>,
    worktree_labels: &mut HashMap<(String, String), Option<String>>,
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
                .or_insert_with(|| {
                    let branch = detect_git_branch(&worktree_id).ok();
                    worktree_display_label(&worktree_id, &workspace_id, branch.as_deref())
                })
                .clone();
            WaitingRoomPublicAgentSummary {
                id: agent.id().to_string(),
                agent_ref: agent.agent_ref().to_string(),
                alias: agent.alias().map(ToOwned::to_owned),
                created_at_ms: agent.created_at_ms(),
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
                activity: waiting_room_agent_activity_summary(session, agent),
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
    use crate::local::{RelayStatus, TerminalType};
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
        session.add_attachment("cli-1".to_string());

        let summaries = waiting_room_session_summaries(vec![session]);

        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.id, "session-1");
        assert_eq!(summary.alias.as_deref(), Some("alias"));
        assert_eq!(summary.workspace_id, "workspace");
        assert_eq!(summary.worktree_id, "worktree");
        assert_eq!(summary.directory.as_deref(), Some("workspace"));
        assert_eq!(summary.connected_cli_count, 1);
        assert_eq!(summary.activity.agent_count, 0);
    }

    #[test]
    fn waiting_room_public_snapshot_inventory_version_includes_projection_inputs() {
        let snapshot = build_waiting_room_public_snapshot(
            vec![RuntimeSession::new(
                "session-1",
                None,
                "workspace",
                "worktree",
                "machine",
                "daemon",
            )],
            RelayStatus {
                configured: false,
                connected: false,
                relay_url: None,
                relay_token_configured: false,
                daemon_id: "daemon".to_string(),
                machine_id: "machine".to_string(),
                machine_alias: None,
            },
            vec![crate::local::TerminalRecord {
                terminal_id: "terminal-1".to_string(),
                terminal_type: TerminalType::Cli,
                alias: Some("local".to_string()),
                paired_at_ms: 7,
                revoked: false,
            }],
            42,
        )
        .expect("snapshot builds");

        assert_eq!(snapshot.schema_version, 5);
        assert_eq!(snapshot.generated_at_ms, 42);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.terminals.len(), 1);
        assert!(!snapshot.inventory_version.is_empty());
    }
}
