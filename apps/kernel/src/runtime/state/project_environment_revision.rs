//! MP-08 / MP-10 / MP-11: One owner-authorized revision path for Web and TUI.
use super::*;
use crate::project_environment::*;
impl KernelRuntimeState {
    pub(crate) async fn preview_environment_diff(
        &self,
        request: crate::local::PreviewEnvironmentDiffRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let store = ProjectEnvironmentStore::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        );
        let _lock = store.lock_briefly_async(project.id()).await?;
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let current = store.snapshot_locked(&project)?;
        Ok(LocalDaemonResponse::ProjectEnvironmentDiff {
            diff: environment_revision_diff(&current, &request.draft)?,
        })
    }
    pub(crate) async fn save_project_environment_revision(
        &self,
        mut request: crate::local::SaveProjectEnvironmentRevisionRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let store = ProjectEnvironmentStore::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        );
        let _lock = store.lock_briefly_async(project.id()).await?;
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let mut current = store.snapshot_locked(&project)?;
        let attached = store.snapshot_live_bindings_locked(&project)?;
        review_new_proposal_folders(&mut current, &mut request, &attached.folders)?;
        let (environment, diff) = store.save_revision_locked(&current, &request, user)?;
        Ok(LocalDaemonResponse::ProjectEnvironmentSaved { environment, diff })
    }
}
