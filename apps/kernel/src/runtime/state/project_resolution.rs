//! MP-08 / MP-09 / MP-10 / MP-11: durable, session-free Project entry on the selected kernel.
use super::*;
use crate::runtime::project_resolution_preparation::{Candidate, Input, Metadata};
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
            create: request.create_if_missing,
            selected: selected.as_ref().map(|project| Candidate {
                id: project.id().into(),
                workspaces: project.workspace_ids().to_vec(),
            }),
            defaults: defaults.clone(),
        };
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
        let Metadata {
            workspace,
            name,
            project_id: matched,
        } = metadata.map_err(|failure| failure.error())?;
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

    // MP-08 / MP-10 / MP-11: interpose realpath to model an unavailable saved mount.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r6_environment_create_session_ignores_unrelated_stalled_saved_workspace() {
        if std::env::var_os("CHARIOX_R6_REALPATH_CHILD").is_none() {
            let root = crate::test_support::TestWorktree::new("r6-saved-mount");
            let source = root.path().join("stall.c");
            let library = root.path().join("stall.so");
            std::fs::write(
                &source,
                r#"
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
char *realpath(const char *path, char *resolved) {
    const char *blocked=getenv("CHARIOX_R6_SAVED_WORKSPACE");
    if (blocked && !strcmp(path,blocked)) usleep(2000000);
    char *(*next)(const char *, char *)=dlsym(RTLD_NEXT,"realpath");
    return next(path,resolved);
}
"#,
            )
            .unwrap();
            assert!(std::process::Command::new("cc")
                .args(["-shared", "-fPIC", "-o"])
                .arg(&library)
                .arg(source)
                .arg("-ldl")
                .status()
                .unwrap()
                .success());
            let test_name = std::thread::current().name().unwrap().to_owned();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    test_name.as_str(),
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("CHARIOX_R6_REALPATH_CHILD", "1")
                .env(
                    "CHARIOX_R6_SAVED_WORKSPACE",
                    root.path().join("unavailable"),
                )
                .env("LD_PRELOAD", library)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "ordinary CreateSession traversed unrelated saved storage: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert!(
                String::from_utf8_lossy(&result.stdout).contains("1 passed;"),
                "the prepared-workspace regression must actually run"
            );
            return;
        }
        let root = crate::test_support::TestWorktree::new("r6-healthy-session");
        let blocked = std::env::var("CHARIOX_R6_SAVED_WORKSPACE").unwrap();
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
                "saved-unavailable",
                crate::session::DEFAULT_LOCAL_USER_ID,
                blocked,
                "Unavailable",
                crate::session::RuntimeProjectKind::Default,
            )]);
        let began = std::time::Instant::now();
        let response = r4_dispatch(
            &router,
            "r6-create-healthy-folder",
            crate::local::LocalDaemonRequest::CreateSession(root.session_request()),
        )
        .await
        .unwrap();
        let elapsed = began.elapsed();
        if let Ok(LocalDaemonResponse::SessionCreated { session, .. }) = &response {
            state.delete_session_id(session.id()).await.unwrap();
        }
        assert!(
            matches!(response, Ok(LocalDaemonResponse::SessionCreated { .. })),
            "healthy local session must be created: {response:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "unrelated saved workspace blocked ordinary CreateSession for {elapsed:?}"
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
}
