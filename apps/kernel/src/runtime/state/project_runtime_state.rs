use super::*;

impl KernelRuntimeState {
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

#[cfg(test)]
mod tests {
    use super::project_session_is_idle;
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
}
