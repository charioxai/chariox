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
        let tracked_workspace_live_sync_journal =
            self.owned.tracked_workspace_live_sync_journal.clone();
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
                    let target_results =
                        self.apply_tracked_workspace_live_sync_change_to_link_targets(&change);
                    self.record_tracked_workspace_live_sync_notices(&change, &target_results);
                    tracked_workspace_live_sync_journal.append(change);
                    tracked_workspace_live_sync_journal.record_target_results(target_results);
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

    fn apply_tracked_workspace_live_sync_change_to_link_targets(
        &self,
        change: &crate::git_observer::TrackedWorkspaceLiveSyncTurnChange,
    ) -> Vec<crate::git_observer::TrackedWorkspaceLiveSyncTargetResult> {
        let Ok(session) = self.owned.session_store.get_session(&change.session_id) else {
            return Vec::new();
        };
        let source_root = std::path::Path::new(&change.worktree_path);
        let Some(link) = session.workspace_link_for_repo_root(source_root).cloned() else {
            return Vec::new();
        };
        link.attachments()
            .iter()
            .filter(|attachment| attachment.repo_root() != change.worktree_path)
            .map(|attachment| {
                let target_root = std::path::Path::new(attachment.repo_root());
                let path_results =
                    crate::git_observer::apply_tracked_workspace_live_sync_change_to_target(
                        change,
                        target_root,
                    );
                crate::git_observer::TrackedWorkspaceLiveSyncTargetResult {
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
            })
            .collect()
    }

    fn record_tracked_workspace_live_sync_notices(
        &self,
        change: &crate::git_observer::TrackedWorkspaceLiveSyncTurnChange,
        target_results: &[crate::git_observer::TrackedWorkspaceLiveSyncTargetResult],
    ) {
        if target_results.is_empty() {
            return;
        }
        let mut applied_targets = 0usize;
        let mut rebased_count = 0usize;
        let mut conflict_count = 0usize;
        let mut failed_count = 0usize;
        for target_result in target_results {
            let mut target_has_applied = false;
            for path_result in &target_result.path_results {
                match path_result.status {
                    crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::Applied => {
                        target_has_applied = true;
                    }
                    crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::Rebased => {
                        target_has_applied = true;
                        rebased_count += 1;
                    }
                    crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::SkippedConflict => {
                        conflict_count += 1;
                        self.owned.record_notice(
                            &change.session_id,
                            Some(&change.provider_run_id),
                            Vec::new(),
                            format!(
                                "Workspace live sync conflict: source agent `{}` changed `{}` but target `{}` at `{}` could not apply it: {}. Next action: assign a resolver agent to reread and reconcile the target.",
                                change.agent_id,
                                path_result.path,
                                target_result.target_user_id,
                                target_result.target_repo_root,
                                path_result.message
                            ),
                        );
                    }
                    crate::git_observer::TrackedWorkspaceLiveSyncApplyStatus::FailedIo => {
                        failed_count += 1;
                    }
                }
            }
            if target_has_applied {
                applied_targets += 1;
            }
        }
        self.owned.record_notice(
            &change.session_id,
            Some(&change.provider_run_id),
            Vec::new(),
            format!(
                "Workspace live sync tracked turn summary: source agent `{}` changed {} path{}; applied to {} target{}; rebased={}; conflicts={}; failed_io={}.",
                change.agent_id,
                change.changed_paths.len(),
                if change.changed_paths.len() == 1 { "" } else { "s" },
                applied_targets,
                if applied_targets == 1 { "" } else { "s" },
                rebased_count,
                conflict_count,
                failed_count
            ),
        );
    }
}
