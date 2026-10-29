//! MP-08 / MP-10 / MP-11: one Project identity for session and standalone entry.
use super::*;

impl SessionService {
    pub(super) fn prepare_default_project(
        &self,
        owner: &str,
        workspace: &str,
        name_hint: Option<&str>,
    ) -> RuntimeProject {
        if let Some(project) = self.projects.values().find(|project| {
            project.owner_user_id() == owner
                && RuntimeProject::workspace_paths_match(project.workspace_id(), workspace)
                && project.kind() == RuntimeProjectKind::Default
        }) {
            return project.clone();
        }
        self.prepare_new_default_project(owner, workspace, name_hint)
    }

    fn prepare_new_default_project(
        &self,
        owner: &str,
        workspace: &str,
        name_hint: Option<&str>,
    ) -> RuntimeProject {
        let name = name_hint
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| core::default_project_name(workspace));
        RuntimeProject::new(
            core::default_project_id(owner, workspace),
            owner,
            workspace,
            self.unique_project_name(owner, &name, None),
            RuntimeProjectKind::Default,
        )
    }

    pub(crate) fn prepare_project_resolution(
        &self,
        project_id: Option<&str>,
        workspace: &str,
        owner: &str,
        create: bool,
        name_hint: Option<&str>,
    ) -> Result<Option<RuntimeProject>, DaemonError> {
        let existing = project_id.and_then(|id| self.projects.get(id)).or_else(|| {
            self.projects.values().find(|project| {
                project.owner_user_id() == owner
                    && project.workspace_id() == workspace
                    && project.kind() == RuntimeProjectKind::Default
            })
        });
        let mut project = match existing {
            Some(project) => {
                if project.owner_user_id() != owner
                    || !project.workspace_ids().iter().any(|path| path == workspace)
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "project.resolve",
                        message: "caller does not own the selected Project and workspace".into(),
                    });
                }
                project.clone()
            }
            None if create => self.prepare_new_default_project(owner, workspace, name_hint),
            None => return Ok(None),
        };
        if project.status() != RuntimeProjectStatus::Active {
            return Err(DaemonError::LocalTransport {
                operation: "project.resolve",
                message: "This Project is archived. Restore it before setting up its Environment."
                    .into(),
            });
        }
        if create {
            project.retain_without_sessions();
        }
        Ok(Some(project))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_project_resolution_is_explicit_idempotent_and_session_free() {
        let mut service = SessionService::new(&DaemonConfig::for_tests());
        assert!(service
            .prepare_project_resolution(None, "/src", "owner", false, None)
            .unwrap()
            .is_none());
        assert!(service.durable_projects().is_empty());
        let created = service
            .prepare_project_resolution(Some("missing"), "/src", "owner", true, None)
            .unwrap()
            .unwrap();
        assert!(created.is_standalone());
        assert!(
            service.durable_projects().is_empty(),
            "prepare must not publish before durable append"
        );
        service.restore_projects(vec![created.clone()]);
        let retry = service
            .prepare_project_resolution(Some("missing"), "/src", "owner", true, None)
            .unwrap()
            .unwrap();
        assert_eq!(created, retry);
        assert!(service.sessions_in_project(created.id()).is_empty());
        assert!(service
            .prepare_project_resolution(Some(created.id()), "/src", "other", true, None)
            .is_err());
        assert!(service
            .prepare_project_resolution(Some(created.id()), "/foreign", "owner", true, None)
            .is_err());
    }
    #[test]
    fn standalone_project_uses_the_ordinary_session_default_identity() {
        let mut service = SessionService::new(&DaemonConfig::for_tests());
        let project = service
            .prepare_project_resolution(None, "/src", "owner", true, None)
            .unwrap()
            .unwrap();
        service.restore_projects(vec![project.clone()]);
        assert!(service.sessions_in_project(project.id()).is_empty());
        let session = service
            .create_session(CreateSessionRequest::new("/src", "/src").with_owner_user_id("owner"))
            .unwrap();
        assert_eq!(session.project_id(), project.id());
        assert_eq!(service.durable_projects().len(), 1);
    }

    #[test]
    fn standalone_project_resolution_refuses_archived_projects() {
        let mut service = SessionService::new(&DaemonConfig::for_tests());
        let mut project = service
            .prepare_project_resolution(None, "/src", "owner", true, None)
            .unwrap()
            .unwrap();
        project.archive();
        service.restore_projects(vec![project.clone()]);
        for create in [false, true] {
            assert!(service
                .prepare_project_resolution(Some(project.id()), "/src", "owner", create, None)
                .is_err());
            assert_eq!(service.get_project(project.id()).unwrap(), project);
        }
    }
}
