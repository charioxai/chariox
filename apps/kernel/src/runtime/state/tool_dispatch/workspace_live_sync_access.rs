use super::workspace_live_sync_permission::{
    workspace_live_sync_permission_interaction, workspace_live_sync_tool_requires_popup,
};
use super::*;

impl KernelRuntimeState {
    pub(super) async fn workspace_live_sync_workspace_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<WorkspaceLiveSyncWorkspaceContext, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let working_directory = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
        let workspace_root = workspace_live_sync_root_for_working_directory(&working_directory);
        let identity = workspace_identity_for_root_off_thread(workspace_root.clone()).await?;
        let identity = workspace_live_sync_identity_for_session_workspace_link(
            identity,
            &session,
            &workspace_root,
        );
        let snapshot = self.owned.workspace_identity_monitor.observe_provider_run(
            provider_run.id(),
            workspace_root.clone(),
            identity,
        );
        Ok(WorkspaceLiveSyncWorkspaceContext {
            root: workspace_root,
            identity: snapshot.current_identity,
            generation: snapshot.generation,
            identity_changed: snapshot.identity_changed,
            valid: snapshot.valid,
        })
    }

    pub(super) async fn maybe_gate_workspace_live_sync_mutation(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        permission_level: crate::provider::AgentPermissionLevel,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        if permission_level != crate::provider::AgentPermissionLevel::Required {
            return Ok(None);
        }
        let Some(agent_id) = agent_id else {
            return Ok(None);
        };
        if !workspace_live_sync_tool_requires_popup(tool_name) {
            return Ok(None);
        }
        let interaction =
            workspace_live_sync_permission_interaction(agent_id, tool_name, arguments)?;
        let interaction_id = interaction.id().to_string();
        let resolution = self
            .create_runtime_interaction(session_id, interaction)
            .await?
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_live_sync_permission_popup",
                message: format!("workspace live sync approval dropped before resolution: {error}"),
            })?;
        if resolution.choice_id.as_deref() == Some("allow") {
            return Ok(None);
        }
        Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "applied": false,
                "interaction_id": interaction_id,
                "reason": {
                    "kind": "permission_denied",
                    "message": "The workspace live sync operation was not approved."
                },
                "next_action": "Retry after approving the workspace live sync request, or switch the session/agent permissions to yolo.",
            }),
        }))
    }

    pub(super) async fn effective_permission_level_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::provider::AgentPermissionLevel, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(crate::session::effective_agent_permission_level(
            &session,
            Some(&agent),
        ))
    }
}

fn workspace_live_sync_root_for_working_directory(working_directory: &Path) -> PathBuf {
    workspace_live_sync_git_toplevel(working_directory)
        .unwrap_or_else(|| working_directory.to_path_buf())
}

fn workspace_live_sync_git_toplevel(path: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_live_sync_root_uses_git_toplevel_for_subdirectories() {
        let base = std::env::temp_dir().join(format!(
            "chariox-live-sync-tool-root-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let repo = base.join("selected");
        let subdir = repo.join("src").join("feature");
        std::fs::create_dir_all(&subdir).expect("repo subdir fixture should exist");
        run_git_init(&repo);
        let expected = repo
            .canonicalize()
            .expect("repo root should canonicalize for comparison");
        let root = workspace_live_sync_root_for_working_directory(&subdir);

        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(root, expected);
    }

    #[test]
    fn workspace_live_sync_root_keeps_non_git_working_directory() {
        let base = std::env::temp_dir().join(format!(
            "chariox-live-sync-tool-root-non-git-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let selected = base.join("selected").join("subdir");
        std::fs::create_dir_all(&selected).expect("non-git fixture should exist");
        let root = workspace_live_sync_root_for_working_directory(&selected);

        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(root, selected);
    }

    fn run_git_init(path: &Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .arg("-b")
            .arg("main")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should run");
        assert!(
            status.success(),
            "git init should succeed in {}",
            path.display()
        );
    }
}
