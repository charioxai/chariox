//! Git turn observation around prompt dispatch and completion.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn observe_git_before_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) {
        let Some(worktree_path) = provider_run.working_directory().cloned() else {
            return;
        };
        let context = crate::git_observer::GitTurnContext {
            session_id: dispatch.session_id.clone(),
            agent_id: dispatch.agent_id.clone(),
            provider: provider_run.provider().to_string(),
            model: provider_run.model().to_string(),
            provider_run_id: dispatch.provider_run_id.clone(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: dispatch.prompt_id.clone(),
            turn_id: dispatch.prompt_id.clone(),
            worktree_path,
            workspace_live_sync_tracked: provider_run.tracks_workspace_live_sync(),
            machine_id: None,
            prompt_summary: crate::prompt_transcript::render_prompt_transcript(
                &dispatch.prompt,
                &dispatch.attachments,
            ),
        };
        match tokio::task::spawn_blocking(move || {
            crate::git_observer::capture_turn_snapshot(context)
        })
        .await
        {
            Ok(Some(snapshot)) => {
                self.owned.git_turn_snapshots.insert(snapshot);
            }
            Ok(None) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to join pre-turn git snapshot task",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "provider_run_id": dispatch.provider_run_id,
                    "prompt_id": dispatch.prompt_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    pub(super) async fn observe_git_after_prompt_completion(
        &self,
        provider_run_id: &str,
        completed_prompt: &crate::session::PromptQueueItem,
    ) {
        let Some(before) = self
            .owned
            .git_turn_snapshots
            .remove(provider_run_id, completed_prompt.id())
        else {
            return;
        };
        let candidates = self.owned.git_turn_snapshots.candidates_for(&before);
        let after_context = crate::git_observer::GitTurnContext {
            session_id: before.session_id.clone(),
            agent_id: before.agent_id.clone(),
            provider: before.provider.clone(),
            model: before.model.clone(),
            provider_run_id: before.provider_run_id.clone(),
            provider_session_id: before.provider_session_id.clone(),
            prompt_id: before.prompt_id.clone(),
            turn_id: before.turn_id.clone(),
            worktree_path: std::path::PathBuf::from(before.worktree_path.clone()),
            workspace_live_sync_tracked: before.workspace_live_sync_tracked,
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let history = self.owned.operational_history_store.clone();
        let observation = tokio::task::spawn_blocking(move || {
            let after = crate::git_observer::capture_turn_snapshot(after_context)?;
            let tracked_change = if before.workspace_live_sync_tracked {
                crate::git_observer::tracked_workspace_live_sync_change_after_turn(&before, &after)
            } else {
                None
            };
            Some(
                crate::git_observer::observe_after_turn(before, after, candidates, &history)
                    .map(|events| (events, tracked_change)),
            )
        })
        .await;
        match observation {
            Ok(Some(Ok((events, tracked_change)))) => {
                if let Some(change) = tracked_change {
                    let changed_path_count = change.changed_paths.len();
                    self.record_and_fanout_workspace_live_sync_change(change, None)
                        .await;
                    crate::logging::info_with_fields(
                        "daemon.workspace_live_sync",
                        "recorded tracked workspace live sync turn change",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": completed_prompt.id(),
                            "changed_path_count": changed_path_count,
                        }),
                    );
                }
                if !events.is_empty() {
                    crate::logging::info_with_fields(
                        "daemon.git_observer",
                        "recorded git history events after agent turn",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": completed_prompt.id(),
                            "event_count": events.len(),
                        }),
                    );
                }
            }
            Ok(Some(Err(error))) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to record git history events after agent turn",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                    "error": error.to_string(),
                }),
            ),
            Ok(None) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to join post-turn git observation task",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                    "error": error.to_string(),
                }),
            ),
        }
    }

    pub(super) async fn record_and_fanout_workspace_live_sync_change(
        &self,
        change: crate::git_observer::WorkspaceLiveSyncChange,
        source_kernel_id: Option<&str>,
    ) {
        let target_results = self
            .apply_workspace_live_sync_change_to_link_targets(&change, source_kernel_id)
            .await;
        self.record_workspace_live_sync_notices(&change, &target_results);
        self.owned.workspace_live_sync_journal.append(change);
        self.owned
            .workspace_live_sync_journal
            .record_target_results(target_results);
    }

    async fn apply_workspace_live_sync_change_to_link_targets(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        source_kernel_id: Option<&str>,
    ) -> Vec<crate::git_observer::WorkspaceLiveSyncTargetResult> {
        let Ok(session) = self.owned.session_store.get_session(&change.session_id) else {
            return Vec::new();
        };
        let source_root = std::path::Path::new(&change.worktree_path);
        let Some(link) = session.workspace_link_for_repo_root(source_root).cloned() else {
            return Vec::new();
        };
        let config = self.config_snapshot().await;
        let mut results = Vec::new();
        for attachment in link.attachments() {
            if attachment.repo_root() == change.worktree_path
                && source_kernel_id.is_none_or(|kernel_id| kernel_id == attachment.kernel_id())
            {
                continue;
            }
            if attachment.kernel_id() == config.daemon_id {
                let target_root = std::path::Path::new(attachment.repo_root());
                let path_results = crate::git_observer::apply_workspace_live_sync_change_to_target(
                    change,
                    target_root,
                );
                results.push(crate::git_observer::WorkspaceLiveSyncTargetResult {
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
                });
                continue;
            }
            results.push(
                self.apply_workspace_live_sync_change_to_remote_target(
                    &config, change, &link, attachment,
                )
                .await,
            );
        }
        results
    }

    async fn apply_workspace_live_sync_change_to_remote_target(
        &self,
        config: &crate::config::DaemonConfig,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        link: &crate::session::WorkspaceLinkDefinition,
        attachment: &crate::session::WorkspaceLinkAttachment,
    ) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
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
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            config,
            arroba_relay::protocol::ClientTarget {
                daemon_id: Some(attachment.kernel_id().to_string()),
                daemon_alias: None,
            },
            crate::transport::relay_peer::RelayPeerRequest::ApplyWorkspaceLiveSyncChange {
                context: context.clone(),
                change: change.clone(),
            },
        )
        .await;
        match response {
            Ok(
                crate::transport::relay_peer::RelayPeerResponse::WorkspaceLiveSyncChangeApplied {
                    target_result,
                },
            ) => target_result,
            Ok(other) => workspace_live_sync_remote_failed_result(
                &context,
                format!("unexpected relay apply response: {other:?}"),
            ),
            Err(error) => workspace_live_sync_remote_failed_result(
                &context,
                format!("failed to relay workspace live sync change: {error}"),
            ),
        }
    }

    pub(crate) fn apply_forwarded_workspace_live_sync_change(
        &self,
        context: crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
        change: crate::git_observer::WorkspaceLiveSyncChange,
    ) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
        if let Some(message) = self.forwarded_workspace_live_sync_rejection(&context) {
            return workspace_live_sync_remote_failed_result(&context, message);
        }
        let path_results = crate::git_observer::apply_workspace_live_sync_change_to_target(
            &change,
            std::path::Path::new(&context.target_repo_root),
        );
        let target_result = crate::git_observer::WorkspaceLiveSyncTargetResult {
            session_id: context.home_session_id.clone(),
            link_id: context.link_id.clone(),
            link_name: context.link_name.clone(),
            source_agent_id: context.source_agent_id.clone(),
            source_worktree_path: context.source_worktree_path.clone(),
            target_user_id: context.target_user_id.clone(),
            target_machine_id: context.target_machine_id.clone(),
            target_kernel_id: context.target_kernel_id.clone(),
            target_repo_root: context.target_repo_root.clone(),
            path_results,
        };
        self.owned
            .workspace_live_sync_journal
            .record_target_results(vec![target_result.clone()]);
        target_result
    }

    fn forwarded_workspace_live_sync_rejection(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    ) -> Option<String> {
        let config = self.owned.config_projection.snapshot();
        if context.target_kernel_id != config.daemon_id {
            return Some(format!(
                "target kernel `{}` does not match local kernel `{}`",
                context.target_kernel_id, config.daemon_id
            ));
        }
        let target_root =
            crate::session::normalize_workspace_link_repo_root(context.target_repo_root.clone());
        let target_is_known = self
            .owned
            .session_store
            .list_all_sessions()
            .iter()
            .any(|session| {
                crate::session::normalize_workspace_link_repo_root(session.worktree_id())
                    == target_root
                    || session.workspace_links().iter().any(|link| {
                        link.attachments().iter().any(|attachment| {
                            attachment.kernel_id() == config.daemon_id
                                && attachment.repo_root() == target_root.as_str()
                        })
                    })
            });
        if target_is_known {
            None
        } else {
            Some(format!(
                "target repo root `{}` is not attached to this kernel",
                context.target_repo_root
            ))
        }
    }

    fn record_workspace_live_sync_notices(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
    ) {
        for message in workspace_live_sync_notice_messages(change, target_results) {
            self.owned.record_notice(
                &change.session_id,
                Some(&change.provider_run_id),
                Vec::new(),
                message,
            );
        }
    }
}

fn workspace_live_sync_notice_messages(
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
) -> Vec<String> {
    if target_results.is_empty() {
        return Vec::new();
    }
    let mode_label = if change.status_fingerprint == "managed_workspace_live_sync" {
        "managed"
    } else {
        "tracked turn"
    };
    let mut applied_targets = 0usize;
    let mut rebased_count = 0usize;
    let mut conflict_count = 0usize;
    let mut failed_count = 0usize;
    let mut target_details = Vec::new();
    let mut notices = Vec::new();
    for target_result in target_results {
        let mut target_has_applied = false;
        for path_result in &target_result.path_results {
            match path_result.status {
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied => {
                    target_has_applied = true;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "applied",
                    ));
                }
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased => {
                    target_has_applied = true;
                    rebased_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "rebased",
                    ));
                }
                crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict => {
                    conflict_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "conflict",
                    ));
                    notices.push(format!(
                        "Workspace live sync conflict: source agent `{}` changed `{}` but target user `{}` worktree `{}` could not apply it: {}. Next action: assign a resolver agent to reread and reconcile the target worktree.",
                        change.agent_id,
                        path_result.path,
                        target_result.target_user_id,
                        target_result.target_repo_root,
                        path_result.message
                    ));
                }
                crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo => {
                    failed_count += 1;
                    target_details.push(workspace_live_sync_target_detail(
                        target_result,
                        path_result,
                        "failed_io",
                    ));
                    notices.push(format!(
                        "Workspace live sync failed: source agent `{}` changed `{}` but target user `{}` worktree `{}` could not apply it: {}. Next action: verify the target worktree is attached and writable, then ask a resolver agent to recheck and re-edit if needed.",
                        change.agent_id,
                        path_result.path,
                        target_result.target_user_id,
                        target_result.target_repo_root,
                        path_result.message
                    ));
                }
            }
        }
        if target_has_applied {
            applied_targets += 1;
        }
    }
    let next_action = if conflict_count > 0 || failed_count > 0 {
        "review the listed conflict/failure notices and assign a resolver agent where needed"
    } else {
        "none"
    };
    notices.push(format!(
        "Workspace live sync {} summary: source agent `{}` changed {} path{}; applied to {} target{}; rebased={}; conflicts={}; failed_io={}; target results: {}; Next action: {}.",
        mode_label,
        change.agent_id,
        change.changed_paths.len(),
        if change.changed_paths.len() == 1 { "" } else { "s" },
        applied_targets,
        if applied_targets == 1 { "" } else { "s" },
        rebased_count,
        conflict_count,
        failed_count,
        workspace_live_sync_target_details_summary(&target_details),
        next_action
    ));
    notices
}

fn workspace_live_sync_target_detail(
    target_result: &crate::git_observer::WorkspaceLiveSyncTargetResult,
    path_result: &crate::git_observer::WorkspaceLiveSyncPathApplyResult,
    status: &str,
) -> String {
    format!(
        "target user `{}` worktree `{}` path `{}` {}",
        target_result.target_user_id, target_result.target_repo_root, path_result.path, status
    )
}

fn workspace_live_sync_target_details_summary(details: &[String]) -> String {
    const MAX_DETAILS: usize = 6;
    if details.is_empty() {
        return "none".to_string();
    }
    let mut shown = details
        .iter()
        .take(MAX_DETAILS)
        .cloned()
        .collect::<Vec<_>>();
    if details.len() > MAX_DETAILS {
        shown.push(format!("{} more", details.len() - MAX_DETAILS));
    }
    shown.join("; ")
}

fn workspace_live_sync_remote_failed_result(
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    message: String,
) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
    crate::git_observer::WorkspaceLiveSyncTargetResult {
        session_id: context.home_session_id.clone(),
        link_id: context.link_id.clone(),
        link_name: context.link_name.clone(),
        source_agent_id: context.source_agent_id.clone(),
        source_worktree_path: context.source_worktree_path.clone(),
        target_user_id: context.target_user_id.clone(),
        target_machine_id: context.target_machine_id.clone(),
        target_kernel_id: context.target_kernel_id.clone(),
        target_repo_root: context.target_repo_root.clone(),
        path_results: vec![crate::git_observer::WorkspaceLiveSyncPathApplyResult {
            path: "*".to_string(),
            status: crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
            message,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_live_sync_summary_names_targets_paths_and_next_action() {
        let messages = workspace_live_sync_notice_messages(
            &change("managed_workspace_live_sync"),
            &[target_result(vec![
                path_result(
                    "src/lib.rs",
                    crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                    "applied cleanly",
                ),
                path_result(
                    "src/main.rs",
                    crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
                    "rebased over non-overlapping target change",
                ),
            ])],
        );

        assert_eq!(messages.len(), 1);
        let summary = &messages[0];
        assert!(summary.contains("Workspace live sync managed summary"));
        assert!(summary.contains("source agent `agent-1`"));
        assert!(summary
            .contains("target user `user-2` worktree `/tmp/target` path `src/lib.rs` applied"));
        assert!(summary
            .contains("target user `user-2` worktree `/tmp/target` path `src/main.rs` rebased"));
        assert!(summary.contains("Next action: none."));
    }

    #[test]
    fn workspace_live_sync_conflict_notice_names_source_target_path_and_action() {
        let messages = workspace_live_sync_notice_messages(
            &change("tracked_workspace_live_sync"),
            &[target_result(vec![path_result(
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
                "overlapping edits",
            )])],
        );

        assert_eq!(messages.len(), 2);
        let conflict = &messages[0];
        assert!(conflict.contains("Workspace live sync conflict"));
        assert!(conflict.contains("source agent `agent-1`"));
        assert!(conflict.contains("changed `src/lib.rs`"));
        assert!(conflict.contains("target user `user-2` worktree `/tmp/target`"));
        assert!(conflict.contains("overlapping edits"));
        assert!(conflict.contains("Next action: assign a resolver agent"));
        assert!(messages[1].contains("conflicts=1"));
        assert!(messages[1].contains("Next action: review the listed conflict/failure notices"));
    }

    #[test]
    fn workspace_live_sync_failed_io_notice_names_source_target_path_and_action() {
        let messages = workspace_live_sync_notice_messages(
            &change("tracked_workspace_live_sync"),
            &[target_result(vec![path_result(
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
                "permission denied",
            )])],
        );

        assert_eq!(messages.len(), 2);
        let failure = &messages[0];
        assert!(failure.contains("Workspace live sync failed"));
        assert!(failure.contains("source agent `agent-1`"));
        assert!(failure.contains("changed `src/lib.rs`"));
        assert!(failure.contains("target user `user-2` worktree `/tmp/target`"));
        assert!(failure.contains("permission denied"));
        assert!(
            failure.contains("Next action: verify the target worktree is attached and writable")
        );
        assert!(messages[1].contains("failed_io=1"));
    }

    fn change(status_fingerprint: &str) -> crate::git_observer::WorkspaceLiveSyncChange {
        crate::git_observer::WorkspaceLiveSyncChange {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            provider_run_id: "run-1".to_string(),
            prompt_id: "prompt-1".to_string(),
            repo_root: "/tmp/source".to_string(),
            worktree_path: "/tmp/source".to_string(),
            branch: Some("main".to_string()),
            changed_paths: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            file_changes: Vec::new(),
            status_fingerprint: status_fingerprint.to_string(),
        }
    }

    fn target_result(
        path_results: Vec<crate::git_observer::WorkspaceLiveSyncPathApplyResult>,
    ) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
        crate::git_observer::WorkspaceLiveSyncTargetResult {
            session_id: "session-1".to_string(),
            link_id: "link-1".to_string(),
            link_name: "pair".to_string(),
            source_agent_id: "agent-1".to_string(),
            source_worktree_path: "/tmp/source".to_string(),
            target_user_id: "user-2".to_string(),
            target_machine_id: "machine-2".to_string(),
            target_kernel_id: "kernel-2".to_string(),
            target_repo_root: "/tmp/target".to_string(),
            path_results,
        }
    }

    fn path_result(
        path: &str,
        status: crate::git_observer::WorkspaceLiveSyncApplyStatus,
        message: &str,
    ) -> crate::git_observer::WorkspaceLiveSyncPathApplyResult {
        crate::git_observer::WorkspaceLiveSyncPathApplyResult {
            path: path.to_string(),
            status,
            message: message.to_string(),
        }
    }
}
