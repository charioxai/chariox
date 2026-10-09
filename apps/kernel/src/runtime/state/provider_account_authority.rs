//! MP-08/MP-10/MP-11: account authority follows worker lease bindings.
use super::*;

impl KernelRuntimeState {
    pub(super) async fn resolve_provider_launch_account_authority(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if request.agent_id.is_none() {
            request.agent_id = session.focused_agent_id().map(str::to_string).or_else(|| {
                self.owned
                    .agent_store
                    .get_focused_agent(&request.session_id)
                    .map(|agent| agent.id().to_string())
            });
        }
        request.provider_account_owner_user_id = Some(
            self.with_app_side_effect(|app| {
                app.provider_account_owner_for_execution(
                    &request.session_id,
                    request.agent_id.as_deref(),
                )
            })
            .await?,
        );
        Ok(request)
    }

    pub(super) async fn provider_account_owner_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<String, DaemonError> {
        self.with_app_side_effect(|app| {
            app.provider_account_owner_for_execution(run.session_id(), run.agent_instance_id())
        })
        .await
    }
}
