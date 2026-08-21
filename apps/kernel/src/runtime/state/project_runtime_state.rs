use super::*;

impl KernelRuntimeState {
    pub(crate) fn ensure_managed_context_project(
        &self,
        target: &crate::local::ManagedContextLaunchTarget,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(project) = managed_context_project(target, caller_user_id)? else {
            return Ok(());
        };
        let existing = self
            .owned
            .session_store
            .durable_projects()
            .into_iter()
            .find(|existing| existing.id() == project.id());
        install_managed_context_project(
            existing.as_ref(),
            project,
            |project| self.append_project_durable_event("project.created", project),
            |project| {
                self.owned.session_store.restore_projects(vec![project]);
                self.owned.runtime_projection_changes.record_change();
            },
        )?;
        Ok(())
    }

    pub(crate) async fn list_projects(
        &self,
        caller_user_id: &str,
        include_archived: bool,
    ) -> Vec<crate::session::RuntimeProject> {
        self.owned
            .session_store
            .list_projects(caller_user_id, include_archived)
    }

    pub(crate) fn list_waiting_room_projects(
        &self,
        caller_user_id: &str,
    ) -> Vec<crate::session::RuntimeProject> {
        self.owned
            .session_store
            .list_visible_projects(caller_user_id, true)
    }

    pub(crate) async fn rename_project(
        &self,
        project_id: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeProject, DaemonError> {
        let project = self
            .owned
            .session_store
            .rename_project(project_id, name, caller_user_id)?;
        self.append_project_durable_event("project.updated", &project)?;
        self.owned.runtime_projection_changes.record_change();
        Ok(project)
    }

    pub(crate) async fn update_project_workspaces(
        &self,
        project_id: &str,
        workspace_ids: Vec<String>,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeProject, DaemonError> {
        let project = self.owned.session_store.update_project_workspaces(
            project_id,
            workspace_ids,
            caller_user_id,
        )?;
        self.append_project_durable_event("project.updated", &project)?;
        self.owned.runtime_projection_changes.record_change();
        Ok(project)
    }

    pub(crate) async fn archive_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<
        (
            crate::session::RuntimeProject,
            Vec<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let project = self.owned.session_store.read().ensure_project_owner(
            project_id,
            caller_user_id,
            "project.archive",
        )?;
        if project.status() == crate::session::RuntimeProjectStatus::Archived {
            return Ok((
                project,
                self.owned.session_store.sessions_in_project(project_id),
            ));
        }
        let sessions = self
            .project_sessions_if_idle(project_id, "archived")
            .await?;
        let mut archived_sessions = Vec::with_capacity(sessions.len());
        for session in sessions {
            let archived = if session.status() == crate::session::SessionStatus::Ended {
                session
            } else {
                self.end_session(session.id()).await?
            };
            let archived = self.owned.session_snapshot(archived.id())?;
            self.owned.terminal_stream.remove_session(archived.id());
            archived_sessions.push(archived);
        }
        let project = self
            .owned
            .session_store
            .archive_project(project_id, caller_user_id)?;
        self.append_project_durable_event("project.updated", &project)?;
        self.owned.runtime_projection_changes.record_change();
        Ok((project, archived_sessions))
    }

    pub(crate) async fn delete_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<
        (
            crate::session::RuntimeProject,
            Vec<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let project = self.owned.session_store.read().ensure_project_owner(
            project_id,
            caller_user_id,
            "project.delete",
        )?;
        let sessions = self.project_sessions_if_idle(project_id, "deleted").await?;
        let mut deleted_sessions = Vec::with_capacity(sessions.len());
        for session in sessions {
            let deleted = self.delete_session_ref(session.id(), None).await?;
            self.owned.terminal_stream.remove_session(deleted.id());
            deleted_sessions.push(deleted);
        }
        if self.owned.session_store.get_project(project_id).is_ok() {
            let deleted_project = self
                .owned
                .session_store
                .delete_project_record(project_id, caller_user_id)?;
            self.append_project_durable_event("project.deleted", &deleted_project)?;
        }
        self.owned.runtime_projection_changes.record_change();
        Ok((project, deleted_sessions))
    }

    pub(crate) async fn restore_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<
        (
            crate::session::RuntimeProject,
            Vec<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        self.owned.session_store.read().ensure_project_owner(
            project_id,
            caller_user_id,
            "project.restore",
        )?;
        let ended_session_ids = self
            .owned
            .session_store
            .sessions_in_project(project_id)
            .into_iter()
            .filter(|session| session.status() == crate::session::SessionStatus::Ended)
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut restored_sessions = Vec::with_capacity(ended_session_ids.len());
        for session_id in ended_session_ids {
            let session = self
                .owned
                .session_store
                .restore_ended_session(&session_id)?;
            let session = self.owned.session_snapshot(session.id())?;
            self.append_session_durable_event(
                "session.updated",
                &session,
                "runtime_restore_project",
            )
            .await?;
            restored_sessions.push(session);
        }
        let project = self
            .owned
            .session_store
            .restore_project_status(project_id, caller_user_id)?;
        self.append_project_durable_event("project.updated", &project)?;
        self.owned.runtime_projection_changes.record_change();
        Ok((project, restored_sessions))
    }

    async fn project_sessions_if_idle(
        &self,
        project_id: &str,
        action: &str,
    ) -> Result<Vec<crate::session::RuntimeSession>, DaemonError> {
        let session_ids = self
            .owned
            .session_store
            .sessions_in_project(project_id)
            .into_iter()
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut sessions = Vec::with_capacity(session_ids.len());
        let mut busy = Vec::new();
        for session_id in session_ids {
            let session = self.session_snapshot(&session_id).await?;
            if !project_session_is_idle(&session) {
                busy.push(
                    session
                        .alias()
                        .map(str::to_string)
                        .unwrap_or_else(|| session.id().to_string()),
                );
            }
            sessions.push(session);
        }
        if !busy.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: if action == "archived" {
                    "project.archive"
                } else {
                    "project.delete"
                },
                message: format!(
                    "Project `{project_id}` cannot be {action} until all sessions are idle. Active or queued work remains in: {}.",
                    busy.join(", ")
                ),
            });
        }
        Ok(sessions)
    }

    pub(super) fn append_project_durable_event(
        &self,
        kind: &str,
        project: &crate::session::RuntimeProject,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            kind,
            Some(project.id().to_string()),
            serde_json::json!({ "project": project }),
        )?;
        Ok(())
    }
}

fn project_session_is_idle(session: &crate::session::RuntimeSession) -> bool {
    !session.has_any_prompt_work()
        && !session.has_active_session_task()
        && !session.has_pending_session_task()
        && session.active_interactions().is_empty()
}

fn managed_context_project(
    target: &crate::local::ManagedContextLaunchTarget,
    caller_user_id: &str,
) -> Result<Option<crate::session::RuntimeProject>, DaemonError> {
    let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
        project_id,
        repositories,
        ..
    } = &target.development
    else {
        return Ok(None);
    };
    let primary = repositories
        .iter()
        .find(|repository| {
            repository.role
                == crate::managed_context::development::DevelopmentRepositoryRole::Primary
        })
        .ok_or_else(|| {
            managed_context_project_error("managed context Project has no primary repository")
        })?;
    let mut workspace_ids = vec![primary.workspace_path.clone()];
    workspace_ids.extend(
        repositories
            .iter()
            .filter(|repository| {
                repository.role
                    == crate::managed_context::development::DevelopmentRepositoryRole::Supporting
            })
            .map(|repository| repository.workspace_path.clone()),
    );
    if workspace_ids
        .iter()
        .any(|workspace_id| workspace_id.is_empty())
        || workspace_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != workspace_ids.len()
    {
        return Err(managed_context_project_error(
            "managed context Project contains invalid Workspace paths",
        ));
    }
    let mut project = crate::session::RuntimeProject::new(
        project_id,
        caller_user_id,
        primary.workspace_path.clone(),
        "Managed Project",
        crate::session::RuntimeProjectKind::Named,
    );
    project.replace_workspace_ids(workspace_ids);
    Ok(Some(project))
}

fn managed_context_project_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed context project",
        message: message.into(),
    }
}

fn install_managed_context_project(
    existing: Option<&crate::session::RuntimeProject>,
    project: crate::session::RuntimeProject,
    persist: impl FnOnce(&crate::session::RuntimeProject) -> Result<(), DaemonError>,
    publish: impl FnOnce(crate::session::RuntimeProject),
) -> Result<bool, DaemonError> {
    if let Some(existing) = existing {
        if existing.owner_user_id() != project.owner_user_id()
            || existing.kind() != project.kind()
            || existing.status() != crate::session::RuntimeProjectStatus::Active
            || existing.workspace_ids() != project.workspace_ids()
        {
            return Err(managed_context_project_error(format!(
                "managed context Project `{}` conflicts with existing kernel state",
                project.id()
            )));
        }
        return Ok(false);
    }
    persist(&project)?;
    publish(project);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        install_managed_context_project, managed_context_project, managed_context_project_error,
        project_session_is_idle,
    };
    use crate::session::{
        PromptQueueItem, PromptStatus, RuntimeInteraction, RuntimeInteractionKind,
        RuntimeInteractionLevel, RuntimeSession,
    };
    use std::collections::VecDeque;

    #[test]
    fn idle_guard_ignores_attached_native_process_but_rejects_authoritative_work() {
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        session.set_active_provider_run(Some("native-provider-run-starting".to_string()));
        assert!(project_session_is_idle(&session));

        let active = PromptQueueItem::new(
            "prompt-active",
            "attachment",
            "agent-1",
            "work",
            PromptStatus::Running,
        );
        session.mirror_agent_prompt_state("agent-1", Some(active), VecDeque::new());
        assert!(!project_session_is_idle(&session));

        let queued = PromptQueueItem::new(
            "prompt-queued",
            "attachment",
            "agent-1",
            "later",
            PromptStatus::Queued,
        );
        session.mirror_agent_prompt_state("agent-1", None, VecDeque::from([queued]));
        assert!(!project_session_is_idle(&session));

        session.mirror_agent_prompt_state("agent-1", None, VecDeque::new());
        session.add_active_interaction(RuntimeInteraction::new(
            "interaction-1",
            "agent-1",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            None,
            "Approve?",
            Vec::new(),
            None,
            None,
            None,
        ));
        assert!(!project_session_is_idle(&session));
    }

    #[test]
    fn managed_context_project_uses_materialized_target_workspace_paths() {
        let target = managed_context_target();

        let project = managed_context_project(&target, "user-1")
            .expect("build managed Project")
            .expect("source Project");

        assert_eq!(project.id(), "project-1");
        assert_eq!(project.owner_user_id(), "user-1");
        assert_eq!(project.kind(), crate::session::RuntimeProjectKind::Named);
        assert_eq!(
            project.workspace_ids(),
            [
                "/managed/context-1/primary",
                "/managed/context-1/supporting"
            ]
        );
    }

    #[test]
    fn managed_context_project_retries_persistence_before_publish_and_restart() {
        let project = managed_context_project(&managed_context_target(), "user-1")
            .expect("build managed Project")
            .expect("source Project");
        let mut durable_project = None;
        let mut published_project = None;

        install_managed_context_project(
            None,
            project.clone(),
            |_project| {
                Err(managed_context_project_error(
                    "injected durable write failure",
                ))
            },
            |project| published_project = Some(project),
        )
        .expect_err("first durable append should fail");
        assert!(durable_project.is_none());
        assert!(published_project.is_none());

        assert!(install_managed_context_project(
            None,
            project,
            |project| {
                durable_project = Some(project.clone());
                Ok(())
            },
            |project| published_project = Some(project),
        )
        .expect("retry should persist before publish"));
        assert_eq!(published_project, durable_project);

        let restarted_projects = durable_project.into_iter().collect::<Vec<_>>();
        assert_eq!(restarted_projects.len(), 1);
        assert_eq!(restarted_projects[0].id(), "project-1");
        assert_eq!(
            restarted_projects[0].workspace_ids(),
            [
                "/managed/context-1/primary",
                "/managed/context-1/supporting"
            ]
        );
    }

    fn managed_context_target() -> crate::local::ManagedContextLaunchTarget {
        crate::local::ManagedContextLaunchTarget {
            environment_id: "environment-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            context_id: "context-1".to_string(),
            plan_digest: "sha256:plan".to_string(),
            development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
                project_id: "project-1".to_string(),
                destination_root: "/managed/context-1".to_string(),
                primary_repository_id: "repository-primary".to_string(),
                repositories: vec![
                    crate::local::ManagedContextRepositoryLaunchTarget {
                        repository_id: "repository-supporting".to_string(),
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Supporting,
                        target_directory: "supporting".to_string(),
                        workspace_path: "/managed/context-1/supporting".to_string(),
                        head_sha: "b".repeat(40),
                    },
                    crate::local::ManagedContextRepositoryLaunchTarget {
                        repository_id: "repository-primary".to_string(),
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        workspace_path: "/managed/context-1/primary".to_string(),
                        head_sha: "a".repeat(40),
                    },
                ],
            },
        }
    }
}
