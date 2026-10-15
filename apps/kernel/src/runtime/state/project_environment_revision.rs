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
        let mut draft = request.draft;
        let diff = tokio::task::spawn_blocking(move || {
            let _lock = _lock;
            let current = store.snapshot_locked(&project)?;
            // Validate before any path read; Preview measures the same selections as Save.
            environment_revision_diff(&current, &draft)?;
            verify_manual_files(&current, &mut draft)?;
            environment_revision_diff(&current, &draft)
        })
        .await
        .map_err(|_| environment_error("Environment preview task failed"))??;
        Ok(LocalDaemonResponse::ProjectEnvironmentDiff { diff })
    }
    pub(crate) async fn save_project_environment_revision(
        &self,
        request: crate::local::SaveProjectEnvironmentRevisionRequest,
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
        let user = user.to_owned();
        let response_store = store.clone();
        let (mut environment, diff) = tokio::task::spawn_blocking(move || {
            let _lock = _lock;
            let current = store.snapshot_locked(&project)?;
            store.save_revision_locked(&current, &request, &user)
        })
        .await
        .map_err(|_| environment_error("Environment Save task failed"))??;
        let config = self.owned.config_projection.snapshot();
        let environment = tokio::task::spawn_blocking(move || {
            response_store.attach_checks(
                &mut environment,
                &crate::project_environment::current_check_target(&config),
                crate::session::unix_epoch_ms(),
            )?;
            Ok::<_, DaemonError>(environment)
        })
        .await
        .map_err(|_| environment_error("Environment observation read task failed"))??;
        Ok(LocalDaemonResponse::ProjectEnvironmentSaved { environment, diff })
    }
}
