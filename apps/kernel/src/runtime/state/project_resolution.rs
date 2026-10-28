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
    #[tokio::test]
    async fn review0440_standalone_session_delete_keeps_project_journal_and_environment() {
        crate::test_support::isolated_env_test!();
        let workspace = crate::test_support::TestWorktree::new("standalone-delete");
        let config = crate::DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config.clone()).unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        )
        .runtime_state();
        let request = crate::local::ResolveProjectRequest {
            project_id: None,
            workspace_id: workspace.path().display().to_string(),
            create_if_missing: true,
        };
        let LocalDaemonResponse::ProjectResolved {
            project: Some(project),
        } = runtime.resolve_project(request, "owner").unwrap()
        else {
            panic!("Project expected")
        };
        let LocalDaemonResponse::SessionCreated { session, .. } = runtime
            .create_session_response(
                crate::session::CreateSessionRequest::new(
                    project.workspace_id(),
                    project.workspace_id(),
                )
                .with_owner_user_id("owner"),
            )
            .await
            .unwrap()
        else {
            panic!("Session expected")
        };
        runtime.delete_session_id(session.id()).await.unwrap();
        assert_eq!(
            runtime.list_projects("owner", true).await,
            vec![project.clone()]
        );
        let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        let deletions: i64 = connection
            .query_row(
                "SELECT count(*) FROM durable_state_events WHERE kind = 'project.deleted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deletions, 0);
        assert!(crate::project_environment::ProjectEnvironmentStore::new(
            &config.private_runtime_state_root()
        )
        .snapshot(&project)
        .is_ok());
    }
    // MP-08 / MP-10 / MP-11: both entry paths reuse a non-Git folder through aliases.
    #[cfg(unix)]
    #[tokio::test]
    async fn review0034_alias_and_trailing_slash_share_project_in_both_creation_orders() {
        crate::test_support::isolated_env_test!();
        for standalone_first in [false, true] {
            for symlink in [false, true] {
                let root = crate::test_support::TestWorktree::new("project-alias");
                let folder = root.path().join("source");
                std::fs::create_dir(&folder).unwrap();
                let alias = if symlink {
                    let alias = root.path().join("source-alias");
                    std::os::unix::fs::symlink(&folder, &alias).unwrap();
                    alias.to_string_lossy().into_owned()
                } else {
                    format!("{}/", folder.display())
                };
                let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
                let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
                    Arc::new(Mutex::new(app)),
                    1,
                )
                .runtime_state();
                let resolve = |path: &str| match runtime
                    .resolve_project(
                        crate::local::ResolveProjectRequest {
                            project_id: None,
                            workspace_id: path.into(),
                            create_if_missing: true,
                        },
                        "owner",
                    )
                    .unwrap()
                {
                    LocalDaemonResponse::ProjectResolved {
                        project: Some(project),
                    } => project,
                    _ => panic!("Project expected"),
                };
                let create = || {
                    runtime.create_session_response(
                        crate::session::CreateSessionRequest::new(alias.clone(), alias.clone())
                            .with_owner_user_id("owner"),
                    )
                };
                let before = standalone_first.then(|| resolve(folder.to_str().unwrap()));
                let LocalDaemonResponse::SessionCreated { session, .. } = create().await.unwrap()
                else {
                    panic!("session expected")
                };
                let project = resolve(folder.to_str().unwrap());
                assert_eq!(
                    session.project_id(),
                    project.id(),
                    "standalone_first={standalone_first}, symlink={symlink}"
                );
                if let Some(before) = before {
                    assert_eq!(before.id(), project.id());
                }
                assert_eq!(runtime.list_projects("owner", true).await.len(), 1);
            }
        }
    }
}
