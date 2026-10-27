use chariox_relay::protocol::ClientTarget;

use crate::app::DaemonApp;
use crate::git_observer::{WorkspaceLiveSyncPathApplyResult, WorkspaceLiveSyncTargetResult};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

impl DaemonApp {
    pub(crate) fn fanout_remote_workspace_live_sync_change(
        &mut self,
        change: crate::git_observer::WorkspaceLiveSyncChange,
        source_kernel_id: Option<&str>,
    ) {
        let Some(link) = self.workspace_live_sync_link_for_change(&change) else {
            return;
        };
        let source_kernel_id = source_kernel_id.unwrap_or(&self.config.daemon_id);
        let source_repo_root =
            crate::session::normalize_workspace_link_repo_root(change.worktree_path.clone());
        let mut target_results = Vec::new();
        for attachment in link.attachments() {
            if attachment.repo_root() == source_repo_root
                && attachment.kernel_id() == source_kernel_id
            {
                continue;
            }
            if attachment.kernel_id() == self.config.daemon_id {
                target_results.push(
                    self.apply_workspace_live_sync_change_to_local_target(
                        &change, &link, attachment,
                    ),
                );
            } else {
                target_results.push(
                    self.apply_workspace_live_sync_change_to_remote_target(
                        &change, &link, attachment,
                    ),
                );
            }
        }
        self.persist_workspace_live_sync_target_results(&target_results);
        self.log_workspace_live_sync_target_results(&change, &target_results);
        self.record_workspace_live_sync_fanout_notice(&change, &target_results);
    }

    fn workspace_live_sync_link_for_change(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
    ) -> Option<crate::session::WorkspaceLinkDefinition> {
        let session = self.sessions.get_session(&change.session_id).ok()?;
        let source_root = std::path::Path::new(&change.worktree_path);
        session.workspace_link_for_repo_root(source_root).cloned()
    }

    fn apply_workspace_live_sync_change_to_local_target(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        link: &crate::session::WorkspaceLinkDefinition,
        attachment: &crate::session::WorkspaceLinkAttachment,
    ) -> WorkspaceLiveSyncTargetResult {
        let target_root = std::path::Path::new(attachment.repo_root());
        let path_results = if let Some(message) =
            crate::git_observer::workspace_live_sync_identity_conflict(
                target_root,
                attachment.branch(),
                attachment.repo_fingerprint(),
            ) {
            workspace_live_sync_identity_conflict_path_results(change, message)
        } else {
            crate::git_observer::apply_workspace_live_sync_change_to_target(change, target_root)
        };
        WorkspaceLiveSyncTargetResult {
            session_id: change.session_id.clone(),
            link_id: link.link_id().to_string(),
            link_name: link.name().to_string(),
            source_agent_id: change.agent_id.clone(),
            source_worktree_path: change.worktree_path.clone(),
            target_user_id: attachment.user_id().to_string(),
            target_machine_id: attachment.machine_id().to_string(),
            target_kernel_id: attachment.kernel_id().to_string(),
            target_repo_root: attachment.repo_root().to_string(),
            path_results,
        }
    }

    fn apply_workspace_live_sync_change_to_remote_target(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        link: &crate::session::WorkspaceLinkDefinition,
        attachment: &crate::session::WorkspaceLinkAttachment,
    ) -> WorkspaceLiveSyncTargetResult {
        let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
            home_session_id: change.session_id.clone(),
            link_id: link.link_id().to_string(),
            link_name: link.name().to_string(),
            source_agent_id: change.agent_id.clone(),
            source_worktree_path: change.worktree_path.clone(),
            target_user_id: attachment.user_id().to_string(),
            target_machine_id: attachment.machine_id().to_string(),
            target_kernel_id: attachment.kernel_id().to_string(),
            target_repo_root: attachment.repo_root().to_string(),
        };
        match self.block_on_relay_future(send_peer_request_via_temporary_connection(
            &self.config,
            ClientTarget {
                daemon_id: Some(attachment.kernel_id().to_string()),
                daemon_alias: None,
            },
            RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
                context: context.clone(),
                change: change.clone(),
            },
        )) {
            Ok(RelayPeerResponse::WorkspaceLiveSyncChangeApplied { target_result }) => {
                target_result
            }
            Ok(other) => workspace_live_sync_failed_result(
                &context,
                change,
                format!("unexpected relay apply response: {other:?}"),
            ),
            Err(error) => workspace_live_sync_failed_result(
                &context,
                change,
                format!("failed to relay workspace live sync change: {error}"),
            ),
        }
    }

    fn persist_workspace_live_sync_target_results(
        &self,
        target_results: &[WorkspaceLiveSyncTargetResult],
    ) {
        if target_results.is_empty() {
            return;
        }
        let session_id = target_results[0].session_id.clone();
        if let Err(error) = self.durable_state.append_event(
            "workspace_live_sync.target_results_recorded",
            Some(session_id.clone()),
            serde_json::json!({
                "target_results": target_results,
            }),
        ) {
            crate::logging::warn_with_fields(
                "daemon.workspace_live_sync",
                "failed to persist workspace live sync target results",
                serde_json::json!({
                    "session_id": session_id,
                    "target_result_count": target_results.len(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn log_workspace_live_sync_target_results(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_results: &[WorkspaceLiveSyncTargetResult],
    ) {
        crate::logging::info_with_fields(
            "daemon.workspace_live_sync",
            "remote tracked workspace live sync fanout completed",
            serde_json::json!({
                "session_id": change.session_id,
                "provider_run_id": change.provider_run_id,
                "source_agent_id": change.agent_id,
                "source_worktree_path": change.worktree_path,
                "changed_paths": change.changed_paths,
                "targets": target_results.iter().map(|result| {
                    serde_json::json!({
                        "target_repo_root": result.target_repo_root,
                        "target_kernel_id": result.target_kernel_id,
                        "paths": result.path_results.iter().map(|path| {
                            serde_json::json!({
                                "path": path.path,
                                "status": path.status,
                                "message": path.message,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            }),
        );
    }

    fn record_workspace_live_sync_fanout_notice(
        &mut self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_results: &[WorkspaceLiveSyncTargetResult],
    ) {
        for message in crate::workspace_live_sync_journal::workspace_live_sync_notice_messages(
            change,
            target_results,
        ) {
            self.record_notice(
                &change.session_id,
                Some(&change.provider_run_id),
                Vec::new(),
                message,
            );
        }
    }
}

fn workspace_live_sync_failed_result(
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    message: String,
) -> WorkspaceLiveSyncTargetResult {
    WorkspaceLiveSyncTargetResult {
        session_id: context.home_session_id.clone(),
        link_id: context.link_id.clone(),
        link_name: context.link_name.clone(),
        source_agent_id: context.source_agent_id.clone(),
        source_worktree_path: context.source_worktree_path.clone(),
        target_user_id: context.target_user_id.clone(),
        target_machine_id: context.target_machine_id.clone(),
        target_kernel_id: context.target_kernel_id.clone(),
        target_repo_root: context.target_repo_root.clone(),
        path_results: workspace_live_sync_failed_path_results(change, message),
    }
}

fn workspace_live_sync_identity_conflict_path_results(
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    message: String,
) -> Vec<WorkspaceLiveSyncPathApplyResult> {
    let paths = if change.changed_paths.is_empty() {
        vec!["*".to_string()]
    } else {
        change.changed_paths.clone()
    };
    paths
        .into_iter()
        .map(|path| WorkspaceLiveSyncPathApplyResult {
            path,
            status: crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
            message: message.clone(),
        })
        .collect()
}

fn workspace_live_sync_failed_path_results(
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    message: String,
) -> Vec<WorkspaceLiveSyncPathApplyResult> {
    let paths = if change.changed_paths.is_empty() {
        vec!["*".to_string()]
    } else {
        change.changed_paths.clone()
    };
    paths
        .into_iter()
        .map(|path| WorkspaceLiveSyncPathApplyResult {
            path,
            status: crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
            message: message.clone(),
        })
        .collect()
}
