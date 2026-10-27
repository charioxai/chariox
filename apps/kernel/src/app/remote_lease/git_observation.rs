use std::path::PathBuf;

use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::transport::relay_peer::{RemoteGitObservation, RemoteGitTurnContext};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(super) fn observe_leased_git_before(
        &mut self,
        leased_agent: &LeasedAgent,
        provider_run_id: &str,
        git_context: RemoteGitTurnContext,
    ) {
        let Some(lease) = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
        else {
            return;
        };
        let Ok(provider_run) = self.app.providers.get_run(provider_run_id) else {
            return;
        };
        let worktree_path = provider_run.working_directory().cloned().or_else(|| {
            self.app
                .sessions
                .get_session(&leased_agent.backing_session_id)
                .ok()
                .map(|session| PathBuf::from(session.worktree_id()))
        });
        let Some(worktree_path) = worktree_path else {
            return;
        };
        let workspace_live_sync_tracked = git_context
            .workspace_live_sync_mode
            .is_some_and(|mode| mode == crate::config::WorkspaceLiveSyncMode::Tracked)
            || provider_run.tracks_workspace_live_sync();
        let context = crate::git_observer::GitTurnContext {
            session_id: git_context.home_session_id,
            agent_id: git_context.home_agent_id,
            provider: leased_agent.provider.clone(),
            model: leased_agent
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            provider_run_id: provider_run_id.to_string(),
            provider_session_id: provider_run.provider_session_id().map(str::to_string),
            prompt_id: git_context.home_prompt_id,
            turn_id: git_context.home_turn_id,
            source_attachment_id: git_context.source_attachment_id,
            prompt_origin: git_context.prompt_origin,
            external_provider: git_context.external_provider,
            external_provider_session_id: git_context.external_provider_session_id,
            external_provider_turn_id: git_context.external_provider_turn_id,
            started_at_ms: None,
            worktree_path,
            workspace_live_sync_tracked,
            machine_id: Some(lease.machine_id),
            prompt_summary: git_context.prompt_summary,
        };
        if let Some(snapshot) = crate::git_observer::capture_turn_snapshot(context) {
            self.app.remote_git_turn_snapshots.insert(snapshot);
        }
    }

    pub(crate) fn observe_leased_git_after(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<
        (
            Vec<RemoteGitObservation>,
            Option<crate::git_observer::WorkspaceLiveSyncChange>,
        ),
        DaemonError,
    > {
        let _leased_agent = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            })?;
        let Some(before) = self
            .app
            .remote_git_turn_snapshots
            .remove_for_provider_run(provider_run_id)
        else {
            return Ok((Vec::new(), None));
        };
        let candidates = self.app.remote_git_turn_snapshots.candidates_for(&before);
        let after_context = crate::git_observer::GitTurnContext {
            session_id: before.session_id.clone(),
            agent_id: before.agent_id.clone(),
            provider: before.provider.clone(),
            model: before.model.clone(),
            provider_run_id: before.provider_run_id.clone(),
            provider_session_id: before.provider_session_id.clone(),
            prompt_id: before.prompt_id.clone(),
            turn_id: before.turn_id.clone(),
            source_attachment_id: before.source_attachment_id.clone(),
            prompt_origin: before.prompt_origin,
            external_provider: before.external_provider.clone(),
            external_provider_session_id: before.external_provider_session_id.clone(),
            external_provider_turn_id: before.external_provider_turn_id.clone(),
            started_at_ms: before.started_at_ms,
            worktree_path: PathBuf::from(before.worktree_path.clone()),
            workspace_live_sync_tracked: before.workspace_live_sync_tracked,
            machine_id: before.machine_id.clone(),
            prompt_summary: before.prompt_summary.clone(),
        };
        let retry_delays_ms: &[u64] = if before.workspace_live_sync_tracked {
            &[50, 150, 300, 500]
        } else {
            &[]
        };
        let mut attempts = 0usize;
        let (after, tracked_change) = loop {
            let Some(after) = crate::git_observer::capture_turn_snapshot(after_context.clone())
            else {
                if attempts >= retry_delays_ms.len() {
                    if before.workspace_live_sync_tracked {
                        self.app.remote_git_turn_snapshots.insert(before);
                    }
                    return Ok((Vec::new(), None));
                }
                std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
                attempts += 1;
                continue;
            };
            let tracked_change = if before.workspace_live_sync_tracked {
                crate::git_observer::tracked_workspace_live_sync_change_after_turn(&before, &after)
            } else {
                None
            };
            let should_retry = before.workspace_live_sync_tracked && tracked_change.is_none();
            if !should_retry || attempts >= retry_delays_ms.len() {
                break (after, tracked_change);
            }
            std::thread::sleep(std::time::Duration::from_millis(retry_delays_ms[attempts]));
            attempts += 1;
        };
        if let Some(change) = tracked_change.as_ref() {
            crate::logging::info_with_fields(
                "daemon.workspace_live_sync",
                "recorded remote tracked workspace live sync turn change",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "changed_path_count": change.changed_paths.len(),
                    "retry_attempts": attempts,
                }),
            );
        } else if before.workspace_live_sync_tracked {
            crate::logging::info_with_fields(
                "daemon.workspace_live_sync",
                "remote tracked workspace live sync turn had no changed paths",
                serde_json::json!({
                    "provider_run_id": provider_run_id,
                    "retry_attempts": attempts,
                    "before_status_fingerprint": before.status_fingerprint.as_str(),
                    "after_status_fingerprint": after.status_fingerprint.as_str(),
                }),
            );
            self.app.remote_git_turn_snapshots.insert(before);
            return Ok((Vec::new(), None));
        }
        Ok((
            crate::git_observer::observations_after_turn(before, after, candidates),
            tracked_change,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn run_test_git(cwd: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo_with_file(root: &std::path::Path, path: &str, contents: &str) {
        std::fs::create_dir_all(root).expect("repo root should exist");
        run_test_git(root, &["init", "-b", "main"]);
        run_test_git(root, &["config", "user.email", "chariox@example.test"]);
        run_test_git(root, &["config", "user.name", "Chariox Test"]);
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("parent dir should exist");
        }
        std::fs::write(&file, contents).expect("seed file should write");
        run_test_git(root, &["add", "."]);
        run_test_git(root, &["commit", "-m", "init"]);
    }

    #[test]
    fn tracked_leased_git_observation_keeps_snapshot_until_change_exists() {
        let root = std::env::temp_dir().join(format!(
            "chariox-remote-tracked-git-observation-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        init_repo_with_file(&root, "target-origin.txt", "target-origin-a\n");

        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            crate::app::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease(
                "home-kernel",
                "home-session",
                "home-agent",
                false,
                "home-user",
            )
            .expect("execution lease should be created");
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".to_string()),
                None,
                None,
                None,
                Some(crate::config::WorkspaceLiveSyncMode::Tracked),
                Some(root.display().to_string()),
                None,
            )
            .expect("leased agent should be created");
        let (provider_run_id, _) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, "edit the file", Vec::new())
            .expect("leased prompt should submit");
        let git_context = RemoteGitTurnContext {
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            home_prompt_id: "home-prompt".to_string(),
            home_turn_id: "home-turn".to_string(),
            source_attachment_id: Some("home-attachment".to_string()),
            workspace_live_sync_mode: Some(crate::config::WorkspaceLiveSyncMode::Tracked),
            prompt_origin: Some(crate::session::PromptOrigin::External),
            external_provider: Some("codex".to_string()),
            external_provider_session_id: Some("codex-thread-1".to_string()),
            external_provider_turn_id: Some("codex-turn-1".to_string()),
            prompt_summary: "edit the file".to_string(),
        };
        RemoteLeaseRuntime::new(&mut app).observe_leased_git_before(
            &leased_agent,
            &provider_run_id,
            git_context,
        );
        let snapshot = app
            .remote_git_turn_snapshots
            .get_for_provider_run(&provider_run_id)
            .expect("remote git snapshot should be recorded");
        assert_eq!(
            snapshot.prompt_origin,
            Some(crate::session::PromptOrigin::External)
        );
        assert_eq!(snapshot.external_provider.as_deref(), Some("codex"));
        assert_eq!(
            snapshot.external_provider_session_id.as_deref(),
            Some("codex-thread-1")
        );
        assert_eq!(
            snapshot.external_provider_turn_id.as_deref(),
            Some("codex-turn-1")
        );

        let (_observations, no_change) = RemoteLeaseRuntime::new(&mut app)
            .observe_leased_git_after(&leased_agent.id, &provider_run_id)
            .expect("no-op observation should succeed");
        assert!(
            no_change.is_none(),
            "first observation should not invent a tracked change"
        );

        std::fs::write(
            root.join("target-origin.txt"),
            "target-origin-a\nagent change\n",
        )
        .expect("agent edit should write");
        let (_observations, tracked_change) = RemoteLeaseRuntime::new(&mut app)
            .observe_leased_git_after(&leased_agent.id, &provider_run_id)
            .expect("second observation should still have baseline");
        let tracked_change = tracked_change.expect("tracked change should be detected");
        assert_eq!(tracked_change.changed_paths, vec!["target-origin.txt"]);

        let _ = std::fs::remove_dir_all(root);
    }
}
