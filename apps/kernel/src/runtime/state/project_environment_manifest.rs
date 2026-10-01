//! MP-08: Shared local/remote client query of the source kernel's manifest.
use super::*;

impl KernelRuntimeState {
    pub(crate) fn project_environment_for_shell(
        &self,
        session_id: &str,
        directory: &Path,
    ) -> Result<crate::provider::ProviderCredentialEnvironment, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        crate::project_environment::project_launch_environment(
            &self.owned.config_projection.snapshot(),
            session.project_id(),
            session.workspace_id(),
            directory,
        )
    }

    pub(crate) fn get_project_environment_manifest(
        &self,
        request: crate::local::GetProjectEnvironmentManifestRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(DaemonError::LocalTransport {
                operation: "get Project environment manifest",
                message: "caller does not own the selected Project".into(),
            });
        }
        let config = self.owned.config_projection.snapshot();
        let store = crate::project_environment::ProjectEnvironmentStore::new(
            &config.private_runtime_state_root(),
        );
        let manifest = store.load(&request.project_id)?.map(|state| state.manifest);
        Ok(LocalDaemonResponse::ProjectEnvironmentManifest { manifest })
    }
}
