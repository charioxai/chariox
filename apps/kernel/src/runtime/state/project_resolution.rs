//! MP-08 / MP-10 / MP-11: durable, session-free Project entry on the selected kernel.
use super::*;

impl KernelRuntimeState {
    pub(crate) fn resolve_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let selected = request
            .project_id
            .as_deref()
            .and_then(|id| self.owned.session_store.get_project(id).ok());
        if let Some(project) = &selected {
            if project.owner_user_id() != caller
                || !project.contains_workspace(&request.workspace_id)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "project.resolve",
                    message: "caller does not own the selected Project and workspace".into(),
                });
            }
        }
        // Existing saved Projects remain readable with detached folders. Creating
        // a binding uses the same directory and Git scope as ordinary sessions.
        let workspace = if request.create_if_missing || selected.is_none() {
            let admitted = crate::git_worktree_placement::preflight_existing_absolute_directory(
                &request.workspace_id,
                "workspace_id",
                "project.resolve",
            )?;
            let path = admitted.canonical_path.to_string_lossy().into_owned();
            let canonical =
                crate::runtime::workspace_git_common::canonical_workspace_path(&path, &path)
                    .unwrap_or(path);
            crate::git_worktree_placement::preflight_existing_absolute_directory(
                &canonical,
                "workspace_id",
                "project.resolve",
            )?;
            if selected.is_some() {
                request.workspace_id.clone()
            } else {
                canonical
            }
        } else {
            request.workspace_id
        };
        let name = if selected.is_none() {
            crate::runtime::workspace_git_common::workspace_display_label(&workspace)
        } else {
            None
        };
        let mut store = self.owned.session_store.write();
        let project = store.prepare_project_resolution(
            request.project_id.as_deref(),
            &workspace,
            caller,
            request.create_if_missing,
            name.as_deref(),
        )?;
        if let Some(project) = &project {
            let old = store.get_project(project.id()).ok();
            if old.as_ref() != Some(project) {
                // Persist before publishing; a failed journal write leaves the
                // in-memory Project map unchanged, and retry reuses its identity.
                self.append_project_durable_event(
                    if old.is_some() {
                        "project.updated"
                    } else {
                        "project.created"
                    },
                    project,
                )?;
                store.restore_projects(vec![project.clone()]);
                self.owned.runtime_projection_changes.record_change();
            }
        }
        Ok(LocalDaemonResponse::ProjectResolved { project })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn standalone_project_journal_failure_does_not_publish_a_phantom_project() {
        let workspace = std::env::temp_dir().join(format!(
            "standalone-project-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir(&workspace).unwrap();
        let config = crate::DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_standalone_project BEFORE INSERT ON durable_state_events WHEN NEW.kind = 'project.created' BEGIN SELECT RAISE(ABORT, 'owned journal failure test'); END;").unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        )
        .runtime_state();
        let request = crate::local::ResolveProjectRequest {
            project_id: None,
            workspace_id: workspace.display().to_string(),
            create_if_missing: true,
        };
        assert!(runtime.resolve_project(request.clone(), "owner").is_err());
        assert!(runtime.list_projects("owner", true).await.is_empty());
        connection
            .execute_batch("DROP TRIGGER reject_standalone_project;")
            .unwrap();
        let response = runtime.resolve_project(request, "owner").unwrap();
        let LocalDaemonResponse::ProjectResolved {
            project: Some(project),
        } = response
        else {
            panic!("expected a durable Project")
        };
        assert!(project.is_standalone());
        assert_eq!(runtime.list_projects("owner", true).await, vec![project]);
        std::fs::remove_dir(&workspace).unwrap();
    }
}
