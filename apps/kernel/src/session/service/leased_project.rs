//! MP-08 / MP-10 / MP-11: A leased hidden session may use its imported Project scope.
use super::*;

impl SessionService {
    pub(crate) fn prepare_leased_project(
        &self,
        session_id: &str,
        project_id: &str,
        workspace_ids: Vec<String>,
    ) -> Result<RuntimeProject, DaemonError> {
        let session = self.get_session(session_id)?;
        let workspace_ids = normalize_project_workspace_ids(workspace_ids)?;
        if !session.is_hidden()
            || project_id.is_empty()
            || (!session.project_id().is_empty() && session.project_id() != project_id)
            || !workspace_ids.iter().any(|id| id == session.workspace_id())
        {
            return Err(project_error(
                "leased project",
                "invalid leased Project scope".into(),
            ));
        }
        if let Some(project) = self.projects.get(project_id) {
            if project.owner_user_id() != session.owner_user_id()
                || project.status() != RuntimeProjectStatus::Active
                || workspace_ids
                    .iter()
                    .any(|id| !project.contains_workspace(id))
            {
                return Err(project_error(
                    "leased project",
                    "imported Project ownership or workspaces changed".into(),
                ));
            }
            return Ok(project.clone());
        }
        let name = self.unique_project_name(
            session.owner_user_id(),
            &default_project_name(session.workspace_id()),
            None,
        );
        let mut project = RuntimeProject::new(
            project_id,
            session.owner_user_id(),
            session.workspace_id(),
            name,
            RuntimeProjectKind::Named,
        );
        project.replace_workspace_ids(workspace_ids);
        Ok(project)
    }

    pub(crate) fn bind_leased_project(
        &mut self,
        session_id: &str,
        project: RuntimeProject,
    ) -> Result<RuntimeSession, DaemonError> {
        // Revalidate before mutating either scope. Hidden sessions remain ephemeral.
        self.prepare_leased_project(session_id, project.id(), project.workspace_ids().to_vec())?;
        self.projects
            .entry(project.id().into())
            .or_insert(project.clone());
        let session = self
            .store
            .get_mut(session_id)
            .expect("validated hidden session");
        session.assign_project_id(project.id());
        Ok(session.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp10_leased_hidden_session_owns_all_imported_workspaces() {
        let mut service = SessionService::new(&DaemonConfig::for_tests());
        let hidden = service
            .create_ephemeral_session(
                CreateSessionRequest::new("primary", "primary").with_hidden(true),
            )
            .unwrap();
        assert!(hidden.project_id().is_empty());
        let project = service
            .prepare_leased_project(
                hidden.id(),
                "imported-project",
                vec!["primary".into(), "supporting".into()],
            )
            .unwrap();
        let bound = service.bind_leased_project(hidden.id(), project).unwrap();
        assert_eq!(bound.project_id(), "imported-project");
        assert!(bound.is_hidden() && service.is_ephemeral_session(bound.id()));
        assert!(service
            .get_project(bound.project_id())
            .unwrap()
            .contains_workspace("supporting"));
        assert!(service
            .prepare_leased_project(bound.id(), "other-project", vec!["primary".into()])
            .is_err());
        assert!(service
            .prepare_leased_project(bound.id(), "imported-project", vec!["foreign".into()])
            .is_err());
    }
}
