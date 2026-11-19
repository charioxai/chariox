//! MP-08 / MP-10 / MP-11: standalone named Project creation, independent of folders/sessions.
use super::core::{normalize_project_name, project_error};
use super::*;
impl SessionService {
    pub fn create_project(
        &mut self,
        name: String,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        if caller_user_id.is_empty() {
            return Err(project_error(
                "project.create",
                "a Project owner is required".into(),
            ));
        }
        let name = normalize_project_name(&name)?;
        self.ensure_project_name_available(caller_user_id, &name, None, "project.create")?;
        let number = self.next_named_project_number(caller_user_id);
        let id = self.next_named_project_id(caller_user_id, "", number);
        let project = RuntimeProject::new(
            id.clone(),
            caller_user_id,
            "",
            name,
            RuntimeProjectKind::Named,
        );
        self.projects.insert(id, project.clone());
        Ok(project)
    }
}
