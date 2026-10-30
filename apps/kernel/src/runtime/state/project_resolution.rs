//! MP-08 / MP-09 / MP-10 / MP-11: durable, session-free Project entry on the selected kernel.
use super::*;
use crate::runtime::project_resolution_preparation::{Candidate, Failure, Input, Metadata};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct PreparedProjectResolution {
    request: crate::local::ResolveProjectRequest,
    caller: String,
    selected: Option<crate::session::RuntimeProject>,
    defaults: Vec<Candidate>,
    workspace: String,
    name: Option<String>,
    matched: Option<String>,
    deadline: Instant,
}
/// The saved Default Project spelling of the requested session folder, measured before lane
/// admission and valid only while the caller's saved Default Projects are unchanged.
#[derive(Debug)]
pub(crate) struct PreparedSessionWorkspace {
    caller: String,
    saved: Vec<Candidate>,
    workspace: String,
    identity: String,
}
#[derive(Debug)]
pub(crate) enum PreparedProjectEntry {
    Resolution(Box<PreparedProjectResolution>),
    SessionWorkspace(PreparedSessionWorkspace),
}
fn defaults(projects: &[crate::session::RuntimeProject]) -> Vec<Candidate> {
    let mut candidates: Vec<_> = projects
        .iter()
        .filter(|project| project.kind() == crate::session::RuntimeProjectKind::Default)
        .map(|project| Candidate {
            id: project.id().into(),
            workspaces: vec![project.workspace_id().into()],
        })
        .collect();
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    candidates
}
fn resolution_error(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "project.resolve",
        message: message.into(),
    }
}
impl KernelRuntimeState {
    pub(crate) async fn prepare_project_resolution(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
    ) -> Result<PreparedProjectResolution, DaemonError> {
        self.authorize_current_external_command()?;
        let selected = request
            .project_id
            .as_deref()
            .and_then(|id| self.owned.session_store.get_project(id).ok());
        if selected
            .as_ref()
            .is_some_and(|project| project.owner_user_id() != caller)
        {
            return Err(resolution_error(
                "caller does not own the selected Project and workspace",
            ));
        }
        let projects = self.owned.session_store.list_projects(caller, true);
        let defaults = defaults(&projects);
        let input = Input {
            workspace: request.workspace_id.clone(),
            worktree: None,
            create: request.create_if_missing,
            selected: selected.as_ref().map(|project| Candidate {
                id: project.id().into(),
                workspaces: project.workspace_ids().to_vec(),
            }),
            defaults: defaults.clone(),
        };
        let (metadata, deadline) = self.measure_project_workspace(input).await?;
        let Metadata {
            workspace,
            name,
            project_id: matched,
            ..
        } = metadata.map_err(Failure::error)?;
        Ok(PreparedProjectResolution {
            request,
            caller: caller.into(),
            selected,
            defaults,
            workspace,
            name,
            matched,
            deadline,
        })
    }
    async fn measure_project_workspace(
        &self,
        input: Input,
    ) -> Result<(Result<Metadata, Failure>, Instant), DaemonError> {
        let budget = Duration::from_secs(10);
        #[cfg(test)]
        let budget = std::env::var("CHARIOX_PROJECT_RESOLUTION_BUDGET_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0 && *value <= 10000)
            .map(Duration::from_millis)
            .unwrap_or(budget);
        let deadline = Instant::now() + budget;
        let root = self
            .owned
            .config_projection
            .snapshot()
            .private_runtime_state_root();
        let metadata = tokio::task::spawn_blocking(move || {
            crate::project_environment::inspect_workspace_metadata(
                &root, crate::runtime::workspace_inspection::Inspection::ProjectResolution(input), deadline,
            )
        }).await.map_err(|_| resolution_error("Project folder preparation failed; retry"))?
            .map_err(|_| resolution_error("Project folder preparation did not finish within its budget; check storage and Git, then retry"))?;
        let crate::runtime::workspace_inspection::Metadata::ProjectResolution(metadata) = metadata
        else {
            return Err(resolution_error(
                "Project folder preparation returned invalid metadata; retry",
            ));
        };
        Ok((metadata, deadline))
    }
    // Ordinary session creation reuses the saved Default Project's folder spelling, so a Project
    // restored under a symlink keeps its identity and Environment. The folder check runs before
    // lane admission, like ResolveProject, and is revalidated when the session is committed.
    pub(crate) async fn prepare_session_workspace(
        &self,
        request: &crate::session::CreateSessionRequest,
        caller: &str,
    ) -> Result<Option<PreparedSessionWorkspace>, DaemonError> {
        let local_placement = request.slice_ref.is_none()
            && request.kernel_ref.as_deref().is_none_or(|kernel_ref| {
                super::session_lifecycle_runtime_state::kernel_ref_matches_local_config(
                    &self.owned.config_projection.snapshot(),
                    kernel_ref,
                )
            });
        // The lane rewrites the worktree of a placement request, so it cannot be measured here.
        if request.hidden
            || request.worktree_placement.is_some()
            || !local_placement
            || !matches!(
                request.project_selection,
                crate::session::SessionProjectSelection::Default
            )
        {
            return Ok(None);
        }
        let saved = defaults(&self.owned.session_store.list_projects(caller, true));
        if saved.is_empty() {
            return Ok(None);
        }
        let (metadata, _) = self
            .measure_project_workspace(Input {
                workspace: request.workspace_id.clone(),
                worktree: Some(request.worktree_id.clone()),
                create: true,
                selected: None,
                defaults: saved.clone(),
            })
            .await?;
        // An unusable requested folder is reported by session creation itself.
        let Ok(Metadata {
            workspace,
            identity,
            project_id: Some(_),
            ..
        }) = metadata
        else {
            return Ok(None);
        };
        Ok(Some(PreparedSessionWorkspace {
            caller: caller.into(),
            saved,
            workspace,
            identity,
        }))
    }
    pub(super) fn apply_prepared_session_workspace(
        &self,
        mut request: crate::session::CreateSessionRequest,
        prepared: Option<PreparedSessionWorkspace>,
    ) -> Result<crate::session::CreateSessionRequest, DaemonError> {
        let Some(prepared) = prepared else {
            return Ok(request);
        };
        if prepared.caller != request.owner_user_id
            || defaults(
                &self
                    .owned
                    .session_store
                    .list_projects(&request.owner_user_id, true),
            ) != prepared.saved
        {
            return Err(resolution_error(
                "Project changed while its folder was checked; refresh and retry",
            ));
        }
        // The lane normalizes the folder against the worktree; the saved spelling belongs to
        // the same folder only if both reach one identity.
        if request.workspace_id != prepared.identity {
            return Err(resolution_error(
                "Project folder changed while it was checked; retry",
            ));
        }
        request.workspace_id = prepared.workspace;
        Ok(request)
    }
    pub(crate) fn resolve_prepared_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
        prepared: PreparedProjectResolution,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        if prepared.request.project_id != request.project_id
            || prepared.request.workspace_id != request.workspace_id
            || prepared.request.create_if_missing != request.create_if_missing
            || prepared.caller != caller
            || Instant::now() >= prepared.deadline
        {
            return Err(resolution_error(
                "Project folder preparation expired or changed; retry",
            ));
        }
        let mut store = self.owned.session_store.write();
        let selected_now = request
            .project_id
            .as_deref()
            .and_then(|id| store.get_project(id).ok());
        if selected_now != prepared.selected
            || (prepared.selected.is_none()
                && defaults(&store.list_projects(caller, true)) != prepared.defaults)
        {
            return Err(resolution_error(
                "Project changed while its folder was checked; refresh and retry",
            ));
        }
        let project = store.prepare_project_resolution(
            prepared.matched.as_deref(),
            &prepared.workspace,
            caller,
            request.create_if_missing,
            prepared.name.as_deref(),
        )?;
        if let Some(project) = &project {
            let old = store.get_project(project.id()).ok();
            if old.as_ref() != Some(project) {
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
    #[cfg(test)]
    async fn resolve_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = self
            .prepare_project_resolution(request.clone(), caller)
            .await?;
        self.resolve_prepared_project(request, caller, prepared)
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
        assert!(runtime
            .resolve_project(request.clone(), "owner")
            .await
            .is_err());
        assert!(runtime.list_projects("owner", true).await.is_empty());
        connection
            .execute_batch("DROP TRIGGER reject_standalone_project;")
            .unwrap();
        let response = runtime.resolve_project(request, "owner").await.unwrap();
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
        } = runtime.resolve_project(request, "owner").await.unwrap()
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
                let resolve = async |path: &str| match runtime
                    .resolve_project(
                        crate::local::ResolveProjectRequest {
                            project_id: None,
                            workspace_id: path.into(),
                            create_if_missing: true,
                        },
                        "owner",
                    )
                    .await
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
                let before = if standalone_first {
                    Some(resolve(folder.to_str().unwrap()).await)
                } else {
                    None
                };
                let LocalDaemonResponse::SessionCreated { session, .. } = create().await.unwrap()
                else {
                    panic!("session expected")
                };
                let project = resolve(folder.to_str().unwrap()).await;
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

    fn r4_dispatch(
        router: &crate::runtime::router::CommandRouter,
        command_id: &str,
        request: crate::local::LocalDaemonRequest,
    ) -> tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>> {
        let router = router.clone();
        let command = crate::runtime::command::KernelCommand::from_local_request(
            command_id, None, None, &request,
        );
        tokio::spawn(async move { router.dispatch(command, request).await })
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r4_project_resolution_dispatcher_stalled_git_keeps_project_and_session_lane_live() {
        use std::os::unix::fs::PermissionsExt;
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let original_path = std::env::var_os("PATH").unwrap();
        let git = std::env::split_paths(&original_path)
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .unwrap()
            .canonicalize()
            .unwrap();
        for release_before_deadline in [false, true] {
            let root = crate::test_support::TestWorktree::new("r4-resolution-worker");
            let blocked = root.path().join("blocked");
            let bin = root.path().join("metadata-tools");
            std::fs::create_dir(&blocked).unwrap();
            std::fs::create_dir(&bin).unwrap();
            let marker = root.path().join("git.pid");
            let release = root.path().join("release-git");
            std::fs::write(bin.join("git"), format!("#!/bin/sh\nif [ \"$PWD\" = '{}' ]; then\n echo $$ > '{}'\n while [ ! -f '{}' ]; do /bin/sleep 0.02; done\n exit 1\nfi\nexec '{}' \"$@\"\n", blocked.display(), marker.display(), release.display(), git.display())).unwrap();
            std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            std::env::set_var(
                "PATH",
                std::env::join_paths(
                    std::iter::once(bin).chain(std::env::split_paths(&original_path)),
                )
                .unwrap(),
            );
            std::env::set_var(
                "CHARIOX_PROJECT_RESOLUTION_BUDGET_MS",
                if release_before_deadline {
                    "5000"
                } else {
                    "1000"
                },
            );
            let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
            let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
                Arc::new(Mutex::new(app)),
                4,
            );
            let state = router.runtime_state();
            let mut resolution = r4_dispatch(
                &router,
                "r4-stalled-resolution",
                crate::local::LocalDaemonRequest::ResolveProject(
                    crate::local::ResolveProjectRequest {
                        project_id: None,
                        workspace_id: blocked.to_string_lossy().into_owned(),
                        create_if_missing: true,
                    },
                ),
            );
            let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !marker.exists() && std::time::Instant::now() < end {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let entered = marker.exists();
            let mut listing = r4_dispatch(
                &router,
                "r4-projects-during-resolution",
                crate::local::LocalDaemonRequest::ListProjects(crate::local::ListProjectsRequest {
                    include_archived: true,
                }),
            );
            let listed = tokio::time::timeout(std::time::Duration::from_secs(1), &mut listing)
                .await
                .ok();
            let other = crate::test_support::TestWorktree::new("r4-create-during-resolution");
            let mut creation = r4_dispatch(
                &router,
                "r4-session-during-resolution",
                crate::local::LocalDaemonRequest::CreateSession(other.session_request()),
            );
            let created = tokio::time::timeout(std::time::Duration::from_secs(2), &mut creation)
                .await
                .ok();
            let listed_live = listed.is_some();
            let created_live = created.is_some();
            if release_before_deadline {
                std::fs::write(&release, "release").unwrap();
            }
            let bounded = tokio::time::timeout(std::time::Duration::from_secs(5), &mut resolution)
                .await
                .ok();
            let settled_before_release = bounded.is_some();
            std::fs::write(&release, "release").unwrap();
            let resolution = match bounded {
                Some(result) => result.unwrap(),
                None => resolution.await.unwrap(),
            };
            let listed = match listed {
                Some(result) => result.unwrap(),
                None => listing.await.unwrap(),
            };
            let created = match created {
                Some(result) => result.unwrap(),
                None => creation.await.unwrap(),
            };
            let no_late_project = !state
                .list_projects(crate::session::DEFAULT_LOCAL_USER_ID, true)
                .await
                .iter()
                .any(|project| {
                    project
                        .workspace_ids()
                        .iter()
                        .any(|path| std::path::Path::new(path) == blocked)
                });
            let session = match created {
                Ok(LocalDaemonResponse::SessionCreated { session, .. }) => Some(session),
                _ => None,
            };
            if let Some(session) = &session {
                state.delete_session_id(session.id()).await.unwrap();
            }
            assert!(
                entered,
                "the real Git subprocess must reach the stalled folder"
            );
            assert!(
                listed_live && listed.is_ok(),
                "Project listing was blocked behind filesystem/Git preparation"
            );
            assert!(
                created_live && session.is_some(),
                "session creation was blocked behind filesystem/Git preparation"
            );
            assert!(
                settled_before_release,
                "Project resolution did not settle its owned deadline"
            );
            assert!(
                resolution.is_err(),
                "expired metadata or changed Project topology cannot commit after preparation"
            );
            assert!(
                no_late_project,
                "late preparation must not publish a Project"
            );
            let pid = std::fs::read_to_string(&marker)
                .unwrap()
                .trim()
                .parse::<i32>()
                .unwrap();
            assert!(pid > 1 && pid != std::process::id() as i32);
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                -1,
                "owned Git subprocess was not reaped"
            );
        }
        std::env::set_var("PATH", original_path);
        std::env::remove_var("CHARIOX_PROJECT_RESOLUTION_BUDGET_MS");
    }

    #[tokio::test]
    async fn r4_project_resolution_worker_keeps_ordinary_protected_directory_admission() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let protected = crate::test_support::TestWorktree::new("r4-resolution-protected");
        std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", protected.path());
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let result = r4_dispatch(
            &router,
            "r4-protected-resolution",
            crate::local::LocalDaemonRequest::ResolveProject(crate::local::ResolveProjectRequest {
                project_id: None,
                workspace_id: protected.path().to_string_lossy().into_owned(),
                create_if_missing: true,
            }),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().to_string().contains("protected"));
        assert!(router
            .runtime_state()
            .list_projects(crate::session::DEFAULT_LOCAL_USER_ID, true)
            .await
            .is_empty());
        std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    }
    // MP-08 / MP-10 / MP-11: restored non-Git spelling participates in automatic aliases.
    #[tokio::test]
    async fn r6_environment_session_alias_allocation_uses_prepared_workspace_equivalence() {
        let root = crate::test_support::TestWorktree::new("r6-restored-session");
        let folder = root.path().join("source");
        std::fs::create_dir(&folder).unwrap();
        let old = format!("{}/", folder.display());
        let config = crate::DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let existing = state
            .owned
            .session_store
            .write()
            .create_session(crate::session::CreateSessionRequest::new(&old, &old))
            .unwrap();
        assert_eq!(existing.alias(), Some("source-1"));
        let LocalDaemonResponse::SessionCreated { session, .. } = r4_dispatch(
            &router,
            "r6-restored-auto-alias",
            crate::local::LocalDaemonRequest::CreateSession(
                crate::session::CreateSessionRequest::new(
                    folder.to_str().unwrap(),
                    folder.to_str().unwrap(),
                ),
            ),
        )
        .await
        .unwrap()
        .expect("automatic alias allocation must include restored workspace spellings") else {
            panic!("Session expected")
        };
        assert_eq!(session.alias(), Some("source-2"));
        assert_eq!(session.project_id(), existing.project_id());
        state.delete_session_id(session.id()).await.unwrap();
        state.delete_session_id(existing.id()).await.unwrap();
    }

    // MP-08 / MP-10 / MP-11: an existing saved folder whose metadata lookup never answers
    // (an unavailable mount) is stalled inside the bounded worker.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r10_environment_create_session_ignores_unresponsive_saved_folder_and_keeps_lane_live()
    {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = crate::test_support::TestWorktree::new("r10-healthy-session");
        let other = crate::test_support::TestWorktree::new("r10-other-session");
        let saved = crate::test_support::TestWorktree::new("r10-unresponsive-saved");
        std::env::set_var("CHARIOX_PROJECT_PREPARATION_STALL_PATH", saved.path());
        std::env::set_var("CHARIOX_PROJECT_RESOLUTION_BUDGET_MS", "3000");
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        state
            .owned
            .session_store
            .restore_projects(vec![crate::session::RuntimeProject::new(
                "saved-unresponsive",
                crate::session::DEFAULT_LOCAL_USER_ID,
                saved.path().to_string_lossy(),
                "Unresponsive",
                crate::session::RuntimeProjectKind::Default,
            )]);
        let began = std::time::Instant::now();
        let healthy = r4_dispatch(
            &router,
            "r10-create-healthy-folder",
            crate::local::LocalDaemonRequest::CreateSession(root.session_request()),
        );
        let second = r4_dispatch(
            &router,
            "r10-create-other-folder",
            crate::local::LocalDaemonRequest::CreateSession(other.session_request()),
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let listed = std::time::Instant::now();
        let projects = r4_dispatch(
            &router,
            "r10-list-projects",
            crate::local::LocalDaemonRequest::ListProjects(crate::local::ListProjectsRequest {
                include_archived: true,
            }),
        )
        .await
        .unwrap();
        let listing = listed.elapsed();
        let healthy = healthy.await.unwrap();
        let second = second.await.unwrap();
        let elapsed = began.elapsed();
        for response in [&healthy, &second] {
            if let Ok(LocalDaemonResponse::SessionCreated { session, .. }) = response {
                state.delete_session_id(session.id()).await.unwrap();
            }
        }
        std::env::remove_var("CHARIOX_PROJECT_PREPARATION_STALL_PATH");
        std::env::remove_var("CHARIOX_PROJECT_RESOLUTION_BUDGET_MS");
        assert!(
            matches!(healthy, Ok(LocalDaemonResponse::SessionCreated { .. }))
                && matches!(second, Ok(LocalDaemonResponse::SessionCreated { .. })),
            "healthy sessions must be created despite an unresponsive saved folder: {healthy:?} {second:?}"
        );
        assert!(projects.is_ok());
        assert!(
            listing < std::time::Duration::from_millis(500),
            "ListProjects waited {listing:?} behind session creation"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(2500),
            "unresponsive saved folder delayed healthy creation for {elapsed:?}"
        );
    }

    // MP-08 / MP-10 / MP-11: the prepared folder identity is valid only for the Projects it saw.
    #[cfg(unix)]
    #[tokio::test]
    async fn r10_environment_prepared_session_folder_is_revalidated_at_commit() {
        let root = crate::test_support::TestWorktree::new("r10-revalidate-session");
        let folder = root.path().join("source");
        std::fs::create_dir(&folder).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let owner = crate::session::DEFAULT_LOCAL_USER_ID;
        let saved = crate::session::RuntimeProject::new(
            "restored-alias",
            owner,
            alias.to_str().unwrap(),
            "Saved",
            crate::session::RuntimeProjectKind::Default,
        );
        state
            .owned
            .session_store
            .restore_projects(vec![saved.clone()]);
        let request = crate::session::CreateSessionRequest::new(
            folder.to_str().unwrap(),
            folder.to_str().unwrap(),
        )
        .with_owner_user_id(owner);
        let prepared = state
            .prepare_session_workspace(&request, owner)
            .await
            .unwrap()
            .expect("the saved symlink spelling must be prepared");
        state
            .owned
            .session_store
            .restore_projects(vec![crate::session::RuntimeProject::new(
                "added-meanwhile",
                owner,
                root.path().join("elsewhere").to_str().unwrap(),
                "Added",
                crate::session::RuntimeProjectKind::Default,
            )]);
        let error = state
            .create_prepared_session_response(request, Some(prepared))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("Project changed"),
            "a stale prepared identity must not commit: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn r6_environment_restored_alias_lookup_runs_through_preparation() {
        let root = crate::test_support::TestWorktree::new("r6-prepared-project-alias");
        let folder = root.path().join("source");
        std::fs::create_dir(&folder).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let saved = crate::session::RuntimeProject::new(
            "restored-alias",
            crate::session::DEFAULT_LOCAL_USER_ID,
            alias.to_str().unwrap(),
            "Saved",
            crate::session::RuntimeProjectKind::Default,
        );
        state
            .owned
            .session_store
            .restore_projects(vec![saved.clone()]);
        let response = r4_dispatch(
            &router,
            "r6-lookup-restored-alias",
            crate::local::LocalDaemonRequest::ResolveProject(crate::local::ResolveProjectRequest {
                project_id: None,
                workspace_id: folder.to_str().unwrap().into(),
                create_if_missing: false,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        let LocalDaemonResponse::ProjectResolved {
            project: Some(resolved),
        } = response
        else {
            panic!("saved Project expected")
        };
        assert_eq!(resolved.id(), saved.id());
        assert_eq!(resolved.workspace_id(), saved.workspace_id());
        assert!(state
            .owned
            .session_store
            .write()
            .sessions_in_project(saved.id())
            .is_empty());
    }

    // MP-08 / MP-09 / MP-10 / MP-11: ordinary session admission shares Project folder identity.
    #[cfg(unix)]
    #[tokio::test]
    async fn r9_environment_create_session_keeps_project_restored_under_a_symlink() {
        let root = crate::test_support::TestWorktree::new("r9-restored-symlink-session");
        let folder = root.path().join("source");
        std::fs::create_dir(&folder).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let state = router.runtime_state();
        let mut saved = crate::session::RuntimeProject::new(
            "restored-symlink",
            crate::session::DEFAULT_LOCAL_USER_ID,
            alias.to_str().unwrap(),
            "Saved",
            crate::session::RuntimeProjectKind::Default,
        );
        let definition = crate::session::ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: crate::session::ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: crate::session::ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".into(),
            source_path: None,
            inputs: vec![],
            path_entries: vec![],
            setup_steps: vec![],
            validation_commands: vec![],
        };
        saved.set_environment_definition(definition.clone());
        state
            .owned
            .session_store
            .restore_projects(vec![saved.clone()]);
        let mut created = Vec::new();
        for (index, requested) in [&folder, &alias].into_iter().enumerate() {
            let LocalDaemonResponse::SessionCreated { session, .. } = r4_dispatch(
                &router,
                &format!("r9-create-restored-symlink-{index}"),
                crate::local::LocalDaemonRequest::CreateSession(
                    crate::session::CreateSessionRequest::new(
                        requested.to_str().unwrap(),
                        requested.to_str().unwrap(),
                    ),
                ),
            )
            .await
            .unwrap()
            .expect("a session in a restored Project folder must be created") else {
                panic!("Session expected")
            };
            assert_eq!(
                session.project_id(),
                saved.id(),
                "a session requested as {requested:?} must join the saved Project, not allocate another"
            );
            assert_eq!(session.workspace_id(), saved.workspace_id());
            created.push(session.id().to_string());
        }
        let projects = state
            .list_projects(crate::session::DEFAULT_LOCAL_USER_ID, true)
            .await;
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id(), saved.id());
        assert_eq!(projects[0].environment_definition(), Some(&definition));
        for id in created {
            state.delete_session_id(&id).await.unwrap();
        }
    }
    struct LinkedWorktreeFixture {
        _root: crate::test_support::TestWorktree,
        plain: std::path::PathBuf,
        linked: std::path::PathBuf,
        plain_project: crate::session::RuntimeProject,
        repository_project: crate::session::RuntimeProject,
        router: crate::runtime::router::CommandRouter,
    }
    // Saved Default Projects (each with its own Environment) for a non-Git folder A and for a
    // repository B, plus a linked worktree of B.
    fn linked_worktree_fixture(label: &str) -> LinkedWorktreeFixture {
        let root = crate::test_support::TestWorktree::new(label);
        let plain = root.path().join("plain");
        let repository = root.path().join("repository");
        let linked = root.path().join("linked");
        std::fs::create_dir(&plain).unwrap();
        std::fs::create_dir(&repository).unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.email", "tests@example.invalid"],
            vec!["config", "user.name", "Chariox Tests"],
            vec!["commit", "--allow-empty", "-m", "initial"],
            vec![
                "worktree",
                "add",
                "-b",
                "feature/r11",
                linked.to_str().unwrap(),
            ],
        ] {
            crate::runtime::workspace_git_common::run_git(&repository, &args).unwrap();
        }
        let app = DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            1,
        );
        let saved = |id: &str, folder: &std::path::Path, platform: &str| {
            let mut project = crate::session::RuntimeProject::new(
                id,
                crate::session::DEFAULT_LOCAL_USER_ID,
                folder.to_str().unwrap(),
                id,
                crate::session::RuntimeProjectKind::Default,
            );
            project.set_environment_definition(crate::session::ProjectEnvironmentDefinition {
                schema_version: 1,
                origin: crate::session::ProjectEnvironmentDefinitionOrigin::UserAuthored,
                source: crate::session::ProjectEnvironmentDefinitionSource::Commands,
                target_platform: platform.into(),
                source_path: None,
                inputs: vec![],
                path_entries: vec![],
                setup_steps: vec![],
                validation_commands: vec![],
            });
            project
        };
        let plain_project = saved("saved-plain", &plain, "linux-x86_64");
        let repository_project = saved("saved-repository", &repository, "linux-aarch64");
        router
            .runtime_state()
            .owned
            .session_store
            .restore_projects(vec![plain_project.clone(), repository_project.clone()]);
        LinkedWorktreeFixture {
            _root: root,
            plain,
            linked,
            plain_project,
            repository_project,
            router,
        }
    }

    // MP-08 / MP-09 / MP-10 / MP-11: the saved Project follows the folder the session really
    // runs in: a linked worktree of repository B outranks the requested non-Git folder A.
    #[cfg(unix)]
    #[tokio::test]
    async fn r11_environment_create_session_prepares_the_effective_linked_worktree_workspace() {
        let fixture = linked_worktree_fixture("r11-effective-workspace");
        let state = fixture.router.runtime_state();
        let LocalDaemonResponse::SessionCreated { session, .. } = r4_dispatch(
            &fixture.router,
            "r11-create-plain-and-linked",
            crate::local::LocalDaemonRequest::CreateSession(
                crate::session::CreateSessionRequest::new(
                    fixture.plain.to_str().unwrap(),
                    fixture.linked.to_str().unwrap(),
                ),
            ),
        )
        .await
        .unwrap()
        .expect("the session must be created") else {
            panic!("Session expected")
        };
        let joined = state
            .owned
            .session_store
            .get_project(session.project_id())
            .unwrap();
        state.delete_session_id(session.id()).await.unwrap();
        assert_eq!(
            (session.workspace_id(), joined.id()),
            (
                fixture.repository_project.workspace_id(),
                fixture.repository_project.id()
            ),
            "the session runs in repository B's worktree and must keep B's Project"
        );
        assert_eq!(
            joined.environment_definition(),
            fixture.repository_project.environment_definition()
        );
        assert_ne!(joined.id(), fixture.plain_project.id());
    }

    // MP-08 / MP-10 / MP-11: a prepared identity applies only to the folder it was measured for.
    #[cfg(unix)]
    #[tokio::test]
    async fn r11_environment_prepared_session_folder_must_match_the_lane_normalization() {
        let fixture = linked_worktree_fixture("r11-revalidate-effective");
        let state = fixture.router.runtime_state();
        let owner = crate::session::DEFAULT_LOCAL_USER_ID;
        let measured = crate::session::CreateSessionRequest::new(
            fixture.plain.to_str().unwrap(),
            fixture.linked.to_str().unwrap(),
        )
        .with_owner_user_id(owner);
        let prepared = state
            .prepare_session_workspace(&measured, owner)
            .await
            .unwrap()
            .expect("repository B's saved folder must be prepared");
        let committed = crate::session::CreateSessionRequest::new(
            fixture.plain.to_str().unwrap(),
            fixture.plain.to_str().unwrap(),
        )
        .with_owner_user_id(owner);
        let error = state
            .create_prepared_session_response(committed, Some(prepared))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("Project folder changed"),
            "an identity measured for another worktree must not commit: {error}"
        );
    }
}
