//! Git turn observation around prompt dispatch and completion.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn observe_git_before_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) {
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
        let Some(before) = self
            .owned
            .git_turn_snapshots
            .remove(provider_run_id, completed_prompt.id())
            .or_else(|| {
                self.owned
                    .git_turn_snapshots
                    .remove_for_provider_run(provider_run_id)
            })
        else {
            crate::logging::warn_with_fields(
                "daemon.git_observer",
                "missing pre-turn git snapshot for completed prompt",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": completed_prompt.id(),
                }),
            );
            return;
        };
        self.observe_git_after_turn_snapshot(provider_run_id, completed_prompt.id(), before, true)
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
        let prompt_is_active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some();
        if prompt_is_active {
            return;
        }
        let Some(before) = self
            .owned
            .git_turn_snapshots
            .remove_for_provider_run(provider_run_id)
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
            worktree_path: std::path::PathBuf::from(before.worktree_path.clone()),
            workspace_live_sync_tracked: before.workspace_live_sync_tracked,
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let history = self.owned.operational_history_store.clone();
        let before_workspace_live_sync_tracked = before.workspace_live_sync_tracked;
        let before_status_fingerprint = before.status_fingerprint.clone();
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
                let tracked_change = if before.workspace_live_sync_tracked {
                    crate::git_observer::tracked_workspace_live_sync_change_after_turn(
                        &before, &after,
                    )
                } else {
                    None
                };
                let should_retry = before.workspace_live_sync_tracked && tracked_change.is_none();
                if !should_retry || attempts >= retry_delays_ms.len() {
                    let after_status_fingerprint = after.status_fingerprint.clone();
                    let history_events = if record_history {
                        crate::git_observer::observe_after_turn(before, after, candidates, &history)
                    } else {
                        Ok(Vec::new())
                    };
                    return Some(history_events.map(|events| {
                        (events, tracked_change, attempts, after_status_fingerprint)
                    }));
                }
                std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
                attempts += 1;
            }
        })
        .await;
        match observation {
            Ok(Some(Ok((events, tracked_change, retry_attempts, after_status_fingerprint)))) => {
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
            Ok(Some(Err(error))) => crate::logging::warn_with_fields(
                "daemon.git_observer",
                "failed to record git history events after agent turn",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "prompt_id": prompt_id,
                    "error": error.to_string(),
                }),
            ),
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
mod tests {
    use super::*;
    use crate::workspace_live_sync_journal::workspace_live_sync_notice_messages;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<crate::DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.workspace_coordinator(),
        )
    }

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

    #[test]
    fn workspace_live_sync_source_attachment_skip_matches_root_and_origin_machine_or_kernel() {
        let source = attachment("/tmp/source", "kernel-source");
        let same_path_same_machine = crate::session::WorkspaceLinkAttachment::new(
            "link-1",
            "local",
            "machine-source",
            "kernel-remote",
            "/tmp/source/",
            Some("main".to_string()),
            Some("fingerprint".to_string()),
        );
        let same_path_remote_machine = crate::session::WorkspaceLinkAttachment::new(
            "link-1",
            "local",
            "machine-remote",
            "kernel-remote",
            "/tmp/source/",
            Some("main".to_string()),
            Some("fingerprint".to_string()),
        );
        let same_kernel_other_path = attachment("/tmp/target", "kernel-source");
        let source_root = crate::session::normalize_workspace_link_repo_root("/tmp/source/");

        assert!(workspace_live_sync_should_skip_source_attachment(
            &source,
            &source_root,
            "kernel-source",
            "machine-source"
        ));
        assert!(workspace_live_sync_should_skip_source_attachment(
            &same_path_same_machine,
            &source_root,
            "kernel-source",
            "machine-source"
        ));
        assert!(!workspace_live_sync_should_skip_source_attachment(
            &same_path_remote_machine,
            &source_root,
            "kernel-source",
            "machine-source"
        ));
        assert!(!workspace_live_sync_should_skip_source_attachment(
            &same_kernel_other_path,
            &source_root,
            "kernel-source",
            "machine-source"
        ));
    }

    #[test]
    fn forwarded_workspace_live_sync_apply_requires_matching_link_attachment() {
        let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
            home_session_id: "session-1".to_string(),
            link_id: "link-1".to_string(),
            link_name: "pair".to_string(),
            source_agent_id: "agent-1".to_string(),
            source_worktree_path: "/tmp/source".to_string(),
            target_user_id: "user-2".to_string(),
            target_machine_id: "machine-2".to_string(),
            target_kernel_id: "kernel-target".to_string(),
            target_repo_root: "/tmp/target/".to_string(),
        };
        let normalized_target_root =
            crate::session::normalize_workspace_link_repo_root("/tmp/target/");
        let linked_target = crate::session::WorkspaceLinkAttachment::new(
            "link-1",
            "user-2",
            "machine-2",
            "kernel-target",
            "/tmp/target",
            Some("main".to_string()),
            None,
        );
        let wrong_link_same_root = crate::session::WorkspaceLinkAttachment::new(
            "link-2",
            "user-2",
            "machine-2",
            "kernel-target",
            "/tmp/target",
            Some("main".to_string()),
            None,
        );
        let wrong_kernel_same_link = crate::session::WorkspaceLinkAttachment::new(
            "link-1",
            "user-2",
            "machine-2",
            "other-kernel",
            "/tmp/target",
            Some("main".to_string()),
            None,
        );

        assert!(forwarded_workspace_live_sync_attachment_matches_context(
            &linked_target,
            &context,
            "kernel-target",
            &normalized_target_root,
        ));
        assert!(!forwarded_workspace_live_sync_attachment_matches_context(
            &wrong_link_same_root,
            &context,
            "kernel-target",
            &normalized_target_root,
        ));
        assert!(!forwarded_workspace_live_sync_attachment_matches_context(
            &wrong_kernel_same_link,
            &context,
            "kernel-target",
            &normalized_target_root,
        ));
    }

    #[tokio::test]
    async fn workspace_live_sync_fans_out_between_second_user_attached_forks() {
        let root = std::env::temp_dir().join(format!(
            "arroba-workspace-live-sync-collab-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let source = root.join("source");
        let target = root.join("target-fork");
        init_repo_with_file(&source, "src/lib.rs", "seed\n");
        init_repo_with_file(&target, "src/lib.rs", "seed\n");

        let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                source.to_string_lossy(),
                source.to_string_lossy(),
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "terminal-client-1",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("terminal attachment should attach");
        let session_id = session.id().to_string();
        let attachment_id = attachment.id().to_string();
        let daemon_id = app.config_projection_store().snapshot().daemon_id;
        let machine_id = app.config_projection_store().snapshot().host_machine_id;
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let (_, invite) = runtime
            .create_session_invite(
                &session_id,
                "invite-user-2".to_string(),
                "local".to_string(),
                None,
                None,
                crate::session::CollaborationLevel::Full,
            )
            .expect("invite should be created");
        runtime
            .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
            .expect("user-2 should join");
        let (_, link) = runtime
            .create_workspace_link(&session_id, "shared-fork".to_string(), "local".to_string())
            .expect("workspace link should be created");
        let source_branch = crate::git_observer::workspace_live_sync_git_branch(&source);
        let target_branch = crate::git_observer::workspace_live_sync_git_branch(&target);
        let source_fingerprint = crate::git_observer::workspace_live_sync_repo_fingerprint(&source);
        let target_fingerprint = crate::git_observer::workspace_live_sync_repo_fingerprint(&target);
        runtime
            .attach_workspace_link(
                &session_id,
                link.link_id(),
                "local".to_string(),
                machine_id.clone(),
                daemon_id.clone(),
                source.to_string_lossy().to_string(),
                source_branch,
                source_fingerprint,
            )
            .expect("source worktree should attach");
        runtime
            .attach_workspace_link(
                &session_id,
                link.link_id(),
                "user-2".to_string(),
                machine_id,
                daemon_id,
                target.to_string_lossy().to_string(),
                target_branch,
                target_fingerprint,
            )
            .expect("second user target worktree should attach");

        std::fs::write(source.join("src/lib.rs"), "seed\nagent change\n")
            .expect("source should change");
        let change = workspace_live_sync_text_change(
            &session_id,
            "agent-source",
            "provider-run-1",
            "prompt-1",
            &source,
            "seed\n",
            "seed\nagent change\n",
        );

        runtime
            .record_and_fanout_workspace_live_sync_change(change, None, None)
            .await;

        let notices = runtime
            .drain_notice_records(&session_id, &attachment_id)
            .await;
        assert!(
            notices.iter().any(|notice| {
                notice.session_id == session_id
                    && notice.provider_run_id.as_deref() == Some("provider-run-1")
                    && notice
                        .message
                        .contains("Workspace live sync tracked turn summary")
                    && notice.message.contains("source agent `agent-source`")
                    && notice.message.contains("target user `user-2`")
                    && notice.message.contains("Next action: none")
            }),
            "workspace live sync fanout should deliver a runtime notice: {notices:?}"
        );

        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs"))
                .expect("target file should be readable"),
            "seed\nagent change\n"
        );
        let results = runtime.workspace_live_sync_target_results(&session_id);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_user_id, "user-2");
        assert_eq!(results[0].source_agent_id, "agent-source");
        assert_eq!(results[0].path_results[0].path, "src/lib.rs");
        assert_eq!(
            results[0].path_results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
        );

        std::fs::write(
            target.join("src/lib.rs"),
            "seed\nagent change\npeer change\n",
        )
        .expect("target should change");
        let peer_change = workspace_live_sync_text_change(
            &session_id,
            "agent-peer",
            "provider-run-2",
            "prompt-2",
            &target,
            "seed\nagent change\n",
            "seed\nagent change\npeer change\n",
        );

        runtime
            .record_and_fanout_workspace_live_sync_change(peer_change, None, None)
            .await;

        assert_eq!(
            std::fs::read_to_string(source.join("src/lib.rs"))
                .expect("source file should be readable"),
            "seed\nagent change\npeer change\n"
        );
        let results = runtime.workspace_live_sync_target_results(&session_id);
        assert_eq!(results.len(), 2);
        let reverse_result = results
            .iter()
            .find(|result| result.source_agent_id == "agent-peer")
            .expect("peer change should produce a reverse target result");
        assert_eq!(reverse_result.target_user_id, "local");
        assert_eq!(reverse_result.path_results[0].path, "src/lib.rs");
        assert_eq!(
            reverse_result.path_results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn forwarded_workspace_live_sync_apply_notifies_target_session() {
        let root = std::env::temp_dir().join(format!(
            "arroba-forwarded-workspace-live-sync-notice-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let target = root.join("target");
        init_repo_with_file(&target, "src/lib.rs", "seed\n");

        let mut config = crate::config::DaemonConfig::for_tests();
        config.daemon_id = "target-kernel".to_string();
        let mut app = crate::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                target.to_string_lossy(),
                target.to_string_lossy(),
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "target-terminal-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("terminal attachment should attach");
        let target_session_id = session.id().to_string();
        let target_attachment_id = attachment.id().to_string();
        let machine_id = app.config_projection_store().snapshot().host_machine_id;
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let (_, link) = runtime
            .create_workspace_link(
                &target_session_id,
                "shared-remote".to_string(),
                "target-user".to_string(),
            )
            .expect("workspace link should be created");
        runtime
            .attach_workspace_link(
                &target_session_id,
                link.link_id(),
                "target-user".to_string(),
                machine_id,
                "target-kernel".to_string(),
                target.to_string_lossy().to_string(),
                crate::git_observer::workspace_live_sync_git_branch(&target),
                crate::git_observer::workspace_live_sync_repo_fingerprint(&target),
            )
            .expect("target worktree should attach");

        let change = workspace_live_sync_text_change(
            "home-session",
            "source-agent",
            "source-provider-run",
            "source-prompt",
            std::path::Path::new("/tmp/source-worktree"),
            "seed\n",
            "seed\nfrom source\n",
        );
        let result = runtime.apply_forwarded_workspace_live_sync_change(
            crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
                home_session_id: "home-session".to_string(),
                link_id: link.link_id().to_string(),
                link_name: link.name().to_string(),
                source_agent_id: "source-agent".to_string(),
                source_worktree_path: "/tmp/source-worktree".to_string(),
                target_user_id: "target-user".to_string(),
                target_machine_id: "target-machine".to_string(),
                target_kernel_id: "target-kernel".to_string(),
                target_repo_root: target.to_string_lossy().to_string(),
            },
            change,
        );

        assert_eq!(
            result.path_results[0].status,
            crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
        );
        assert_eq!(
            std::fs::read_to_string(target.join("src/lib.rs"))
                .expect("target file should be readable"),
            "seed\nfrom source\n"
        );
        let notices = runtime
            .drain_notice_records(&target_session_id, &target_attachment_id)
            .await;
        assert!(
            notices.iter().any(|notice| {
                notice.session_id == target_session_id
                    && notice.provider_run_id.is_none()
                    && notice
                        .message
                        .contains("Workspace live sync tracked turn summary")
                    && notice.message.contains("source agent `source-agent`")
                    && notice.message.contains("target user `target-user`")
                    && notice.message.contains("Next action: none")
            }),
            "forwarded workspace live sync apply should notify target session: {notices:?}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn workspace_live_sync_text_change(
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        worktree: &std::path::Path,
        before: &str,
        after: &str,
    ) -> crate::git_observer::WorkspaceLiveSyncChange {
        use base64::Engine as _;

        crate::git_observer::WorkspaceLiveSyncChange {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            prompt_id: prompt_id.to_string(),
            repo_root: worktree.to_string_lossy().to_string(),
            worktree_path: worktree.to_string_lossy().to_string(),
            branch: crate::git_observer::workspace_live_sync_git_branch(worktree),
            changed_paths: vec!["src/lib.rs".to_string()],
            file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
                before_content_base64: Some(
                    base64::engine::general_purpose::STANDARD.encode(before),
                ),
                after_content_base64: Some(base64::engine::general_purpose::STANDARD.encode(after)),
                binary: false,
            }],
            status_fingerprint: "tracked_workspace_live_sync".to_string(),
        }
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

    fn attachment(repo_root: &str, kernel_id: &str) -> crate::session::WorkspaceLinkAttachment {
        crate::session::WorkspaceLinkAttachment::new(
            "link-1",
            "user-1",
            "machine-1",
            kernel_id,
            repo_root,
            Some("main".to_string()),
            Some("repo-fingerprint".to_string()),
        )
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

    fn init_repo_with_file(root: &std::path::Path, relative_path: &str, content: &str) {
        std::fs::create_dir_all(root.join(std::path::Path::new(relative_path).parent().unwrap()))
            .expect("fixture directory should be created");
        run_git(root, &["init", "-b", "main"]);
        run_git(
            root,
            &["config", "user.email", "workspace-live-sync@example.com"],
        );
        run_git(root, &["config", "user.name", "Workspace Live Sync"]);
        std::fs::write(root.join(relative_path), content).expect("fixture file should be written");
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "seed"]);
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        std::fs::create_dir_all(cwd).expect("git cwd should exist");
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
