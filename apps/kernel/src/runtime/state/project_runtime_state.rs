use super::*;

impl KernelRuntimeState {
    pub(crate) fn ensure_managed_context_project(
        &self,
        target: &crate::local::ManagedContextLaunchTarget,
        caller_user_id: &str,
    ) -> Result<(), ManagedContextRegistrationFailure> {
        let durable = self.owned.session_store.durable_projects();
        register_managed_context_projects(
            managed_context_projects(target, caller_user_id)
                .map_err(ManagedContextRegistrationFailure::RolledBack)?,
            &durable,
            |project| self.append_project_durable_event("project.created", project),
            |project| {
                self.owned.session_store.restore_projects(vec![project]);
                self.owned.runtime_projection_changes.record_change();
            },
            |project| {
                self.append_project_durable_event("project.deleted", project)?;
                self.owned
                    .session_store
                    .delete_project_record(project.id(), caller_user_id)?;
                self.owned.runtime_projection_changes.record_change();
                Ok(())
            },
        )
    }

    pub(crate) fn remove_managed_context_project(
        &self,
        target: &crate::local::ManagedContextLaunchTarget,
        caller_user_id: &str,
    ) -> Result<bool, DaemonError> {
        let mut removed = false;
        for expected in managed_context_projects(target, caller_user_id)?
            .into_iter()
            .rev()
        {
            let Some(existing) = self
                .owned
                .session_store
                .durable_projects()
                .into_iter()
                .find(|project| project.id() == expected.id())
            else {
                continue;
            };
            install_managed_context_project(Some(&existing), expected, |_| Ok(()), |_| {})?;
            if !self
                .owned
                .session_store
                .sessions_in_project(existing.id())?
                .is_empty()
            {
                return Err(managed_context_project_error(format!(
                    "managed context Project `{}` has sessions and cannot be rolled back",
                    existing.id()
                )));
            }
            self.append_project_durable_event("project.deleted", &existing)?;
            self.owned
                .session_store
                .delete_project_record(existing.id(), caller_user_id)?;
            self.owned.runtime_projection_changes.record_change();
            removed = true;
        }
        Ok(removed)
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

    pub(crate) fn list_room_list_projects(
        &self,
        caller_user_id: &str,
    ) -> Vec<crate::session::RuntimeProject> {
        self.owned
            .session_store
            .list_visible_projects(caller_user_id, true, |session| {
                self.owned.is_room_member(session, caller_user_id)
            })
    }

    pub(crate) async fn set_project_pinned(
        &self,
        project_id: &str,
        pinned: bool,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeProject, DaemonError> {
        let project =
            self.owned
                .session_store
                .set_project_pinned(project_id, pinned, caller_user_id)?;
        self.append_project_durable_event("project.updated", &project)?;
        self.owned.runtime_projection_changes.record_change();
        Ok(project)
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
                self.owned.session_store.sessions_in_project(project_id)?,
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
        let config = self.owned.config_projection.snapshot();
        let vault = crate::secret::project_environment_vault(&config)?;
        crate::project_environment::ProjectEnvironmentStore::new(
            &config.private_runtime_state_root(),
        )
        .remove_with_secrets(project_id, vault.as_ref())?;
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
            .sessions_in_project(project_id)?
            .into_iter()
            .filter(|session| session.status() == crate::session::SessionStatus::Ended)
            .map(|session| session.id().to_string())
            .collect::<Vec<_>>();
        let mut restored_sessions = Vec::with_capacity(ended_session_ids.len());
        for session_id in ended_session_ids {
            let ended = self
                .owned
                .session_store
                .restore_ended_session(&session_id)?;
            // Archiving drops the live agents; the ended snapshot still holds
            // them, so the restored Room gets its own agents back, not new ones.
            let agents = ended.agents().to_vec();
            for agent in &agents {
                self.owned.agent_store.restore_agent(agent.clone());
            }
            let session = self.owned.session_snapshot(ended.id())?;
            self.append_session_durable_event(
                "session.updated",
                &session,
                "runtime_restore_project",
            )
            .await?;
            for agent in &agents {
                self.owned.durable_state_store.append_event(
                    "agent.updated",
                    Some(agent.id().to_string()),
                    serde_json::json!({"agent": agent, "source": "runtime_restore_project"}),
                )?;
            }
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
            .sessions_in_project(project_id)?
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

fn managed_context_projects(
    target: &crate::local::ManagedContextLaunchTarget,
    caller_user_id: &str,
) -> Result<Vec<crate::session::RuntimeProject>, DaemonError> {
    let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { projects } =
        &target.development
    else {
        return Ok(Vec::new());
    };
    projects
        .iter()
        .map(|project| managed_context_project(project, caller_user_id))
        .collect()
}

fn managed_context_project(
    target: &crate::local::ManagedContextProjectLaunchTarget,
    caller_user_id: &str,
) -> Result<crate::session::RuntimeProject, DaemonError> {
    let crate::local::ManagedContextProjectLaunchTarget {
        project_id,
        repositories,
        ..
    } = target;
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
    Ok(project)
}

fn managed_context_project_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed context project",
        message: message.into(),
    }
}

/// Why a copy's Projects did not all register, and whether the copied files
/// may still go: the bridge deletes them only when no Project points at them.
#[derive(Debug)]
pub(crate) enum ManagedContextRegistrationFailure {
    /// Every Project this call installed has been taken back.
    RolledBack(DaemonError),
    /// Projects stay registered with their files, because an earlier attempt left them or they could not be taken
    /// back. Boxed to keep the `Result` this travels in small.
    ProjectsRetained(Box<RetainedProjects>),
}

#[derive(Debug)]
pub(crate) struct RetainedProjects {
    pub(crate) error: DaemonError,
    /// Positions in the selection of the Projects that stay registered.
    pub(crate) projects: Vec<usize>,
}

impl ManagedContextRegistrationFailure {
    pub(crate) fn into_error(self) -> DaemonError {
        match self {
            Self::RolledBack(error) => error,
            Self::ProjectsRetained(retained) => retained.error,
        }
    }
}

/// A copy registers every Project or none: a later failure must not leave an
/// earlier Project registered while the package rollback deletes its files.
/// The whole selection is checked first; only a failed write rolls back, and
/// only the records this call installed. Every one of them is taken back even
/// when another cannot be. The copied files go only when no Project of the
/// selection stays registered: that counts the Projects an earlier attempt
/// left behind (matched from durable state) as well as the ones that cannot
/// be taken back now, and names them. A retry finds them already registered.
fn register_managed_context_projects(
    projects: Vec<crate::session::RuntimeProject>,
    durable: &[crate::session::RuntimeProject],
    mut persist: impl FnMut(&crate::session::RuntimeProject) -> Result<(), DaemonError>,
    mut publish: impl FnMut(crate::session::RuntimeProject),
    mut retract: impl FnMut(&crate::session::RuntimeProject) -> Result<(), DaemonError>,
) -> Result<(), ManagedContextRegistrationFailure> {
    // A Project already registered keeps its files whatever goes wrong before this call writes anything.
    let registered_before = projects
        .iter()
        .enumerate()
        .filter(|(_, project)| durable.iter().any(|existing| existing.id() == project.id()))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let rejected = |error: DaemonError| {
        if registered_before.is_empty() {
            ManagedContextRegistrationFailure::RolledBack(error)
        } else {
            ManagedContextRegistrationFailure::ProjectsRetained(Box::new(RetainedProjects {
                error,
                projects: registered_before.clone(),
            }))
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut fresh = Vec::new();
    let mut registered_earlier = Vec::new();
    for (index, project) in projects.into_iter().enumerate() {
        if !seen.insert(project.id().to_string()) {
            return Err(rejected(managed_context_project_error(format!(
                "managed context names Project `{}` twice",
                project.id()
            ))));
        }
        let existing = durable
            .iter()
            .find(|existing| existing.id() == project.id());
        if install_managed_context_project(existing, project.clone(), |_| Ok(()), |_| {})
            .map_err(rejected)?
        {
            fresh.push((index, project));
        } else {
            registered_earlier.push((index, project));
        }
    }
    let mut installed: Vec<(usize, crate::session::RuntimeProject)> = Vec::new();
    for (index, project) in fresh {
        if let Err(error) = persist(&project) {
            let mut retained = registered_earlier
                .iter()
                .map(|(index, project)| {
                    (
                        *index,
                        format!("`{}` (registered by an earlier attempt)", project.id()),
                    )
                })
                .collect::<Vec<_>>();
            retained.extend(installed.iter().rev().filter_map(|(done_index, done)| {
                retract(done)
                    .err()
                    .map(|rollback| (*done_index, format!("`{}` ({rollback})", done.id())))
            }));
            if retained.is_empty() {
                return Err(ManagedContextRegistrationFailure::RolledBack(error));
            }
            let descriptions = retained
                .iter()
                .map(|(_, description)| description.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ManagedContextRegistrationFailure::ProjectsRetained(Box::new(RetainedProjects {
                error: DaemonError::ManagedContext {
                    code: "managed_context_import_unavailable",
                    operation: "managed context relay import",
                    message: format!(
                        "{error}; Projects stay registered with their files until the import is retried: {descriptions}"
                    ),
                    retryable: true,
                },
                projects: retained.into_iter().map(|(index, _)| index).collect(),
            })));
        }
        publish(project.clone());
        installed.push((index, project));
    }
    Ok(())
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
        install_managed_context_project, managed_context_project_error, managed_context_projects,
        project_session_is_idle, register_managed_context_projects,
        ManagedContextRegistrationFailure, RetainedProjects,
    };
    use crate::error::DaemonError;
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

        let project = managed_context_projects(&target, "user-1")
            .expect("build managed Project")
            .remove(0);

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

    fn projects_named(ids: &[&str]) -> Vec<crate::session::RuntimeProject> {
        let mut target = managed_context_target();
        let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { projects } =
            &mut target.development
        else {
            unreachable!("the fixture imports Projects")
        };
        let template = projects[0].clone();
        *projects = ids
            .iter()
            .map(|id| crate::local::ManagedContextProjectLaunchTarget {
                project_id: (*id).to_string(),
                ..template.clone()
            })
            .collect();
        managed_context_projects(&target, "user-1").expect("build managed Projects")
    }

    #[test]
    fn a_failed_second_registration_rolls_back_only_the_projects_it_installed() {
        let mut durable = Vec::new();
        let mut published = Vec::new();
        let mut retracted = Vec::new();
        let failure = register_managed_context_projects(
            projects_named(&["project-1", "project-2"]),
            &[],
            |project| {
                if project.id() == "project-2" {
                    return Err(managed_context_project_error(
                        "injected durable write failure",
                    ));
                }
                durable.push(project.id().to_string());
                Ok(())
            },
            |project| published.push(project.id().to_string()),
            |project| {
                retracted.push(project.id().to_string());
                Ok(())
            },
        )
        .expect_err("the second write fails");
        let ManagedContextRegistrationFailure::RolledBack(error) = failure else {
            panic!("every installed Project was taken back, so nothing stays registered")
        };
        assert!(error.to_string().contains("injected durable write failure"));
        assert_eq!(durable, ["project-1"]);
        assert_eq!(published, ["project-1"]);
        assert_eq!(retracted, ["project-1"]);
    }

    #[test]
    fn a_failed_compensation_keeps_the_projects_that_stay_registered() {
        let mut retracted = Vec::new();
        let failure = register_managed_context_projects(
            projects_named(&["project-1", "project-2", "project-3"]),
            &[],
            |project| {
                if project.id() == "project-3" {
                    return Err(managed_context_project_error(
                        "injected durable write failure",
                    ));
                }
                Ok(())
            },
            |_| {},
            |project| {
                retracted.push(project.id().to_string());
                if project.id() == "project-2" {
                    return Err(managed_context_project_error("injected delete failure"));
                }
                Ok(())
            },
        )
        .expect_err("the third write fails and project-2 cannot be taken back");
        // One stuck Project does not stop the others from being taken back.
        assert_eq!(retracted, ["project-2", "project-1"]);
        let ManagedContextRegistrationFailure::ProjectsRetained(retained) = failure else {
            panic!("project-2 is still registered, so the copied files must stay")
        };
        let RetainedProjects { error, projects } = *retained;
        assert_eq!(projects, [1], "project-2 is the second of the selection");
        let message = error.to_string();
        assert!(
            message.contains("injected durable write failure"),
            "{message}"
        );
        assert!(message.contains("project-2"), "{message}");
        assert!(!message.contains("project-1"), "{message}");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn a_retried_copy_registers_only_the_projects_a_retained_attempt_left_out() {
        let projects = projects_named(&["project-1", "project-2"]);
        let kept = vec![projects[0].clone()];
        let mut persisted = Vec::new();
        register_managed_context_projects(
            projects,
            &kept,
            |project| {
                persisted.push(project.id().to_string());
                Ok(())
            },
            |_| {},
            |_| Ok(()),
        )
        .expect("the retry finishes the registration");
        assert_eq!(persisted, ["project-2"]);
    }

    #[test]
    fn a_failed_retry_keeps_the_projects_an_earlier_attempt_left_registered() {
        let projects = projects_named(&["project-1", "project-2"]);
        // First attempt: project-1 registers, project-2's write fails and
        // project-1 cannot be taken back.
        let mut durable = Vec::new();
        let first = register_managed_context_projects(
            projects.clone(),
            &[],
            |project| {
                if project.id() == "project-2" {
                    return Err(managed_context_project_error(
                        "injected durable write failure",
                    ));
                }
                durable.push(project.clone());
                Ok(())
            },
            |_| {},
            |_| Err(managed_context_project_error("injected delete failure")),
        )
        .expect_err("project-2 fails and project-1 cannot be taken back");
        assert!(matches!(
            first,
            ManagedContextRegistrationFailure::ProjectsRetained(_)
        ));

        // After a restart only the durable record is left. The retry finds
        // project-1 registered and installs nothing itself before project-2
        // fails again: project-1 still points at the copied files.
        let mut retracted = Vec::new();
        let retry = register_managed_context_projects(
            projects,
            &durable,
            |_| {
                Err(managed_context_project_error(
                    "injected durable write failure",
                ))
            },
            |_| {},
            |project| {
                retracted.push(project.id().to_string());
                Ok(())
            },
        )
        .expect_err("project-2 fails again");
        assert!(
            retracted.is_empty(),
            "project-1 is not this call's to retract"
        );
        let ManagedContextRegistrationFailure::ProjectsRetained(retained) = retry else {
            panic!("project-1 is still registered, so the copied files must stay")
        };
        let RetainedProjects { error, projects } = *retained;
        assert_eq!(projects, [0], "project-1 is the first of the selection");
        let message = error.to_string();
        assert!(
            message.contains("injected durable write failure"),
            "{message}"
        );
        assert!(
            message.contains("`project-1` (registered by an earlier attempt)"),
            "{message}"
        );
        assert!(!message.contains("`project-2`"), "{message}");
    }

    #[test]
    fn managed_context_project_retries_persistence_before_publish_and_restart() {
        let project = managed_context_projects(&managed_context_target(), "user-1")
            .expect("build managed Project")
            .remove(0);
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
            destination: None,
            environment_id: "environment-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            context_id: "context-1".to_string(),
            plan_digest: "sha256:plan".to_string(),
            development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
                projects: vec![crate::local::ManagedContextProjectLaunchTarget {
                project_id: "project-1".to_string(),
                destination_root: "/managed/context-1".to_string(),
                primary_repository_id: "repository-primary".to_string(),
                repositories: vec![
                    crate::local::ManagedContextRepositoryLaunchTarget {
                        workspace_kind:
                            crate::managed_context::development::DevelopmentWorkspaceKind::Git,
                        repository_id: "repository-supporting".to_string(),
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Supporting,
                        target_directory: "supporting".to_string(),
                        workspace_path: "/managed/context-1/supporting".to_string(),
                        head_sha: "b".repeat(40),
                    },
                    crate::local::ManagedContextRepositoryLaunchTarget {
                        workspace_kind:
                            crate::managed_context::development::DevelopmentWorkspaceKind::Git,
                        repository_id: "repository-primary".to_string(),
                        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        workspace_path: "/managed/context-1/primary".to_string(),
                        head_sha: "a".repeat(40),
                    },
                ],
                }],
            },
        }
    }
}
