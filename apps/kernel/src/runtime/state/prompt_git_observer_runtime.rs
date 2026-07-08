//! Git turn observation around prompt dispatch and completion.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn observe_git_before_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) {
        if self
            .owned
            .git_turn_snapshots
            .get(provider_run.id(), &dispatch.prompt_id)
            .is_some()
        {
            return;
        }
        let Some(worktree_path) = provider_run.working_directory().cloned().or_else(|| {
            self.owned
                .session_store
                .get_session(&dispatch.session_id)
                .ok()
                .map(|session| std::path::PathBuf::from(session.worktree_id()))
        }) else {
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
            started_at_ms: self
                .owned
                .active_turns
                .snapshot()
                .get(provider_run.id())
                .map(|turn| turn.started_at_ms),
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
                if snapshot.workspace_live_sync_tracked {
                    crate::logging::info_with_fields(
                        "daemon.git_observer",
                        "captured tracked workspace live sync pre-turn snapshot",
                        serde_json::json!({
                            "session_id": snapshot.session_id,
                            "agent_id": snapshot.agent_id,
                            "provider_run_id": snapshot.provider_run_id,
                            "prompt_id": snapshot.prompt_id,
                            "worktree_path": snapshot.worktree_path,
                            "is_dirty": snapshot.is_dirty,
                        }),
                    );
                }
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
        let session_id = self
            .owned
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .map(|run| run.session_id().to_string());
        self.observe_git_after_completed_prompt(
            session_id.as_deref(),
            Some(provider_run_id),
            completed_prompt,
        )
        .await;
    }

    pub(super) async fn observe_git_after_completed_prompt(
        &self,
        session_id: Option<&str>,
        provider_run_id: Option<&str>,
        completed_prompt: &crate::session::PromptQueueItem,
    ) {
        let before = provider_run_id.and_then(|provider_run_id| {
            self.owned
                .git_turn_snapshots
                .get(provider_run_id, completed_prompt.id())
                .or_else(|| {
                    self.owned
                        .git_turn_snapshots
                        .get_for_provider_run(provider_run_id)
                })
        });
        let before = before.or_else(|| {
            session_id.and_then(|session_id| {
                self.owned.git_turn_snapshots.get_for_session_agent_prompt(
                    session_id,
                    completed_prompt.target_agent_id(),
                    completed_prompt.id(),
                )
            })
        });
        let Some(before) = before else {
            let provider_run_id = provider_run_id.unwrap_or("<none>");
            let session_id = session_id.unwrap_or("<unknown>");
            crate::logging::warn_with_fields(
                "daemon.git_observer",
                "missing pre-turn git snapshot for completed prompt",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": completed_prompt.target_agent_id(),
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                }),
            );
            return;
        };
        let provider_run_id = before.provider_run_id.clone();
        self.observe_git_after_turn_snapshot(&provider_run_id, completed_prompt.id(), before, true)
            .await;
    }

    pub(super) async fn observe_git_after_provider_activity_if_pending(
        &self,
        provider_run_id: &str,
    ) {
        let Ok(provider_run) = self.owned.provider_store.get_run(provider_run_id) else {
            return;
        };
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return;
        };
        let Ok(session) = self
            .owned
            .session_store
            .get_session(provider_run.session_id())
        else {
            return;
        };
        let prompt_is_active = session.active_prompt_for_agent(agent_id).is_some();
        if prompt_is_active {
            return;
        }
        let Some(before) = self
            .owned
            .git_turn_snapshots
            .get_for_provider_run(provider_run_id)
        else {
            crate::logging::debug_with_fields(
                "daemon.git_observer",
                "no pending pre-turn git snapshot after provider activity",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                }),
            );
            return;
        };
        let prompt_id = before.prompt_id.clone();
        self.observe_git_after_turn_snapshot(provider_run_id, &prompt_id, before, false)
            .await;
    }

    async fn observe_git_after_turn_snapshot(
        &self,
        provider_run_id: &str,
        prompt_id: &str,
        before: crate::git_observer::GitTurnSnapshot,
        record_history: bool,
    ) {
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
            started_at_ms: before.started_at_ms,
            worktree_path: std::path::PathBuf::from(before.worktree_path.clone()),
            workspace_live_sync_tracked: before.workspace_live_sync_tracked,
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let history = self.owned.operational_history_store.clone();
        let before_workspace_live_sync_tracked = before.workspace_live_sync_tracked;
        let before_status_fingerprint = before.status_fingerprint.clone();
        let pending_provider_run_id = before.provider_run_id.clone();
        let pending_prompt_id = before.prompt_id.clone();
        let observation = tokio::task::spawn_blocking(move || {
            let retry_delays_ms: &[u64] = if before.workspace_live_sync_tracked {
                &[50, 150, 300, 500]
            } else {
                &[]
            };
            let mut attempts = 0usize;
            loop {
                let Some(after) = crate::git_observer::capture_turn_snapshot(after_context.clone())
                else {
                    if attempts >= retry_delays_ms.len() {
                        return None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
                    attempts += 1;
                    continue;
                };
                let turn_change =
                    crate::git_observer::tracked_workspace_live_sync_change_after_turn(
                        &before, &after,
                    );
                let tracked_change = if before.workspace_live_sync_tracked {
                    turn_change.clone()
                } else {
                    None
                };
                let should_retry = before.workspace_live_sync_tracked && tracked_change.is_none();
                if !should_retry || attempts >= retry_delays_ms.len() {
                    let after_status_fingerprint = after.status_fingerprint.clone();
                    let completed_turn = crate::git_observer::CompletedGitTurnSnapshot::new(
                        before.clone(),
                        after.clone(),
                        turn_change,
                        crate::session::unix_epoch_ms(),
                    );
                    let history_events = if record_history {
                        crate::git_observer::observe_after_turn(before, after, candidates, &history)
                    } else {
                        Ok(Vec::new())
                    };
                    return Some((
                        history_events,
                        tracked_change,
                        attempts,
                        after_status_fingerprint,
                        completed_turn,
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
                attempts += 1;
            }
        })
        .await;
        match observation {
            Ok(Some((
                history_events,
                tracked_change,
                retry_attempts,
                after_status_fingerprint,
                completed_turn,
            ))) => {
                self.owned
                    .completed_git_turn_snapshots
                    .record(completed_turn);
                self.owned
                    .git_turn_snapshots
                    .remove(&pending_provider_run_id, &pending_prompt_id);
                if let Some(change) = tracked_change {
                    let changed_path_count = change.changed_paths.len();
                    self.record_and_fanout_workspace_live_sync_change(change, None, None)
                        .await;
                    crate::logging::info_with_fields(
                        "daemon.workspace_live_sync",
                        "recorded tracked workspace live sync turn change",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": prompt_id,
                            "changed_path_count": changed_path_count,
                            "retry_attempts": retry_attempts,
                        }),
                    );
                } else if before_workspace_live_sync_tracked {
                    crate::logging::info_with_fields(
                        "daemon.workspace_live_sync",
                        "tracked workspace live sync turn had no changed paths",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": prompt_id,
                            "retry_attempts": retry_attempts,
                            "before_status_fingerprint": before_status_fingerprint,
                            "after_status_fingerprint": after_status_fingerprint,
                        }),
                    );
                }
                match history_events {
                    Ok(events) => {
                        if !events.is_empty() {
                            crate::logging::info_with_fields(
                                "daemon.git_observer",
                                "recorded git history events after agent turn",
                                serde_json::json!({
                                    "provider_run_id": provider_run_id,
                                    "prompt_id": prompt_id,
                                    "event_count": events.len(),
                                }),
                            );
                        }
                    }
                    Err(error) => crate::logging::warn_with_fields(
                        "daemon.git_observer",
                        "failed to record git history events after agent turn",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": prompt_id,
                            "error": error.to_string(),
                        }),
                    ),
                }
            }
            Ok(None) => {
                if before_workspace_live_sync_tracked {
                    crate::logging::warn_with_fields(
                        "daemon.workspace_live_sync",
                        "tracked workspace live sync post-turn capture failed",
                        serde_json::json!({
                            "provider_run_id": provider_run_id,
                            "prompt_id": prompt_id,
                            "before_status_fingerprint": before_status_fingerprint,
                        }),
                    );
                }
            }
            Err(error) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to join post-turn git observation task",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": prompt_id,
                    "error": error.to_string(),
                }),
            ),
        }
    }

    pub(super) async fn record_and_fanout_workspace_live_sync_change(
        &self,
        change: crate::git_observer::WorkspaceLiveSyncChange,
        source_kernel_id: Option<&str>,
        source_machine_id: Option<&str>,
    ) {
        let link = self.workspace_live_sync_link_for_change(&change);
        let target_results = match link.as_ref() {
            Some(link) => {
                self.apply_workspace_live_sync_change_to_link_targets(
                    &change,
                    source_kernel_id,
                    source_machine_id,
                    link,
                )
                .await
            }
            None => Vec::new(),
        };
        self.record_workspace_live_sync_notices(&change, &target_results);
        if let Some(link) = link {
            let entry = self.owned.workspace_live_sync_journal.append_for_link(
                link.link_id(),
                link.name(),
                change,
            );
            self.persist_workspace_live_sync_journal_entry(&entry);
        }
        self.persist_workspace_live_sync_target_results(&target_results);
        self.owned
            .workspace_live_sync_journal
            .record_target_results(target_results);
    }

    fn workspace_live_sync_link_for_change(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
    ) -> Option<crate::session::WorkspaceLinkDefinition> {
        let session = self
            .owned
            .session_store
            .get_session(&change.session_id)
            .ok()?;
        let source_root = std::path::Path::new(&change.worktree_path);
        session.workspace_link_for_repo_root(source_root).cloned()
    }

    async fn apply_workspace_live_sync_change_to_link_targets(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        source_kernel_id: Option<&str>,
        source_machine_id: Option<&str>,
        link: &crate::session::WorkspaceLinkDefinition,
    ) -> Vec<crate::git_observer::WorkspaceLiveSyncTargetResult> {
        let config = self.config_snapshot().await;
        let source_kernel_id = source_kernel_id.unwrap_or(&config.daemon_id);
        let source_machine_id = source_machine_id.unwrap_or(&config.host_machine_id);
        let source_repo_root =
            crate::session::normalize_workspace_link_repo_root(change.worktree_path.clone());
        let mut results = Vec::new();
        for attachment in link.attachments() {
            if workspace_live_sync_should_skip_source_attachment(
                attachment,
                &source_repo_root,
                source_kernel_id,
                source_machine_id,
            ) {
                continue;
            }
            if attachment.kernel_id() == config.daemon_id {
                let target_root = std::path::Path::new(attachment.repo_root());
                if let Some(message) = crate::git_observer::workspace_live_sync_identity_conflict(
                    target_root,
                    attachment.branch(),
                    attachment.repo_fingerprint(),
                ) {
                    results.push(workspace_live_sync_identity_conflict_result(
                        change, &link, attachment, message,
                    ));
                    continue;
                }
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
            let target_result = workspace_live_sync_remote_failed_result(&context, message);
            self.record_forwarded_workspace_live_sync_target_result(None, &change, &target_result);
            return target_result;
        }
        if let Some(message) = self.forwarded_workspace_live_sync_identity_conflict(&context) {
            let target_result =
                workspace_live_sync_remote_conflict_result(&context, &change, message);
            let local_session_id = self.forwarded_workspace_live_sync_target_session_id(&context);
            self.record_forwarded_workspace_live_sync_target_result(
                local_session_id.as_deref(),
                &change,
                &target_result,
            );
            return target_result;
        }
        let local_session_id = self.forwarded_workspace_live_sync_target_session_id(&context);
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
        self.record_forwarded_workspace_live_sync_target_result(
            local_session_id.as_deref(),
            &change,
            &target_result,
        );
        target_result
    }

    fn record_forwarded_workspace_live_sync_target_result(
        &self,
        local_session_id: Option<&str>,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_result: &crate::git_observer::WorkspaceLiveSyncTargetResult,
    ) {
        self.owned
            .workspace_live_sync_journal
            .record_target_results(vec![target_result.clone()]);
        self.persist_workspace_live_sync_target_results(std::slice::from_ref(target_result));
        if let Some(local_session_id) = local_session_id {
            self.record_workspace_live_sync_notices_for_session(
                local_session_id,
                None,
                change,
                std::slice::from_ref(target_result),
            );
        }
    }

    fn persist_workspace_live_sync_journal_entry(
        &self,
        entry: &crate::git_observer::WorkspaceLiveSyncJournalEntry,
    ) {
        if let Err(error) = self.owned.durable_state_store.append_event(
            "workspace_live_sync.change_recorded",
            Some(entry.change.session_id.clone()),
            serde_json::json!({
                "entry": entry,
            }),
        ) {
            crate::logging::warn_with_fields(
                "daemon.workspace_live_sync",
                "failed to persist workspace live sync journal entry",
                serde_json::json!({
                    "session_id": entry.change.session_id,
                    "link_id": entry.link_id,
                    "sequence": entry.sequence,
                    "error": error.to_string(),
                }),
            );
        }
    }

    fn persist_workspace_live_sync_target_results(
        &self,
        target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
    ) {
        if target_results.is_empty() {
            return;
        }
        let session_id = target_results[0].session_id.clone();
        if let Err(error) = self.owned.durable_state_store.append_event(
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
        if self
            .forwarded_workspace_live_sync_target_attachment(context, &config.daemon_id)
            .is_some()
        {
            None
        } else {
            Some(format!(
                "target repo root `{}` is not attached to workspace live sync link `{}` on this kernel",
                context.target_repo_root, context.link_id
            ))
        }
    }

    fn forwarded_workspace_live_sync_target_attachment(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
        local_kernel_id: &str,
    ) -> Option<crate::session::WorkspaceLinkAttachment> {
        let target_root =
            crate::session::normalize_workspace_link_repo_root(context.target_repo_root.clone());
        self.owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .flat_map(|session| session.workspace_links().to_vec())
            .flat_map(|link| link.attachments().to_vec())
            .find(|attachment| {
                forwarded_workspace_live_sync_attachment_matches_context(
                    attachment,
                    context,
                    local_kernel_id,
                    &target_root,
                )
            })
    }

    fn forwarded_workspace_live_sync_target_session_id(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    ) -> Option<String> {
        let config = self.owned.config_projection.snapshot();
        let target_root =
            crate::session::normalize_workspace_link_repo_root(context.target_repo_root.clone());
        self.owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .find_map(|session| {
                session
                    .workspace_links()
                    .iter()
                    .flat_map(|link| link.attachments())
                    .any(|attachment| {
                        forwarded_workspace_live_sync_attachment_matches_context(
                            attachment,
                            context,
                            &config.daemon_id,
                            &target_root,
                        )
                    })
                    .then(|| session.id().to_string())
            })
    }

    fn forwarded_workspace_live_sync_identity_conflict(
        &self,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    ) -> Option<String> {
        let config = self.owned.config_projection.snapshot();
        let attachment =
            self.forwarded_workspace_live_sync_target_attachment(context, &config.daemon_id)?;
        crate::git_observer::workspace_live_sync_identity_conflict(
            std::path::Path::new(&context.target_repo_root),
            attachment.branch(),
            attachment.repo_fingerprint(),
        )
    }

    fn record_workspace_live_sync_notices(
        &self,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
    ) {
        self.record_workspace_live_sync_notices_for_session(
            &change.session_id,
            Some(&change.provider_run_id),
            change,
            target_results,
        );
    }

    fn record_workspace_live_sync_notices_for_session(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        change: &crate::git_observer::WorkspaceLiveSyncChange,
        target_results: &[crate::git_observer::WorkspaceLiveSyncTargetResult],
    ) {
        for message in crate::workspace_live_sync_journal::workspace_live_sync_notice_messages(
            change,
            target_results,
        ) {
            self.owned
                .record_notice(session_id, provider_run_id, Vec::new(), message);
        }
    }
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

fn workspace_live_sync_remote_conflict_result(
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    change: &crate::git_observer::WorkspaceLiveSyncChange,
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
        path_results: workspace_live_sync_identity_conflict_path_results(change, message),
    }
}

fn workspace_live_sync_identity_conflict_result(
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    link: &crate::session::WorkspaceLinkDefinition,
    attachment: &crate::session::WorkspaceLinkAttachment,
    message: String,
) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
    crate::git_observer::WorkspaceLiveSyncTargetResult {
        session_id: change.session_id.clone(),
        link_id: link.link_id().to_string(),
        link_name: link.name().to_string(),
        source_agent_id: change.agent_id.clone(),
        source_worktree_path: change.worktree_path.clone(),
        target_user_id: attachment.user_id().to_string(),
        target_machine_id: attachment.machine_id().to_string(),
        target_kernel_id: attachment.kernel_id().to_string(),
        target_repo_root: attachment.repo_root().to_string(),
        path_results: workspace_live_sync_identity_conflict_path_results(change, message),
    }
}

fn workspace_live_sync_identity_conflict_path_results(
    change: &crate::git_observer::WorkspaceLiveSyncChange,
    message: String,
) -> Vec<crate::git_observer::WorkspaceLiveSyncPathApplyResult> {
    let paths = if change.changed_paths.is_empty() {
        vec!["*".to_string()]
    } else {
        change.changed_paths.clone()
    };
    paths
        .into_iter()
        .map(
            |path| crate::git_observer::WorkspaceLiveSyncPathApplyResult {
                path,
                status: crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
                message: message.clone(),
            },
        )
        .collect()
}

fn workspace_live_sync_should_skip_source_attachment(
    attachment: &crate::session::WorkspaceLinkAttachment,
    source_repo_root: &str,
    source_kernel_id: &str,
    source_machine_id: &str,
) -> bool {
    attachment.repo_root() == source_repo_root
        && (attachment.kernel_id() == source_kernel_id
            || attachment.machine_id() == source_machine_id)
}

fn forwarded_workspace_live_sync_attachment_matches_context(
    attachment: &crate::session::WorkspaceLinkAttachment,
    context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext,
    local_kernel_id: &str,
    normalized_target_root: &str,
) -> bool {
    attachment.link_id() == context.link_id
        && attachment.kernel_id() == local_kernel_id
        && attachment.repo_root() == normalized_target_root
}

#[cfg(test)]
mod tests;
