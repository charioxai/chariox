//! MP-08 / MP-09 / MP-10 / MP-11: preparation stays outside the shared Project commit lane.
use super::*;
impl SessionRuntimeStore {
    pub(in crate::runtime::session_actor) async fn prepare_project_resolution(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &LocalDaemonRequest,
        actor: Option<&str>,
        caller: &str,
    ) -> Result<Option<crate::runtime::state::PreparedProjectResolution>, DaemonError> {
        let LocalDaemonRequest::ResolveProject(resolution) = request else {
            return Ok(None);
        };
        let state = self
            .state
            .with_kernel_command_authority(command, request)
            .with_room_provider_origin(actor, command.provider_run_id.as_deref())
            .with_room_request_origin(actor, request);
        state.authorize_current_external_command()?;
        state.authorize_room_provider_epoch(actor, command.provider_run_id.as_deref())?;
        if let Some(actor) = actor {
            state.authorize_room_agent_request(actor, request)?;
        }
        state
            .prepare_project_resolution(resolution.clone(), caller)
            .await
            .map(Some)
    }
    pub(in crate::runtime::session_actor) fn resolve_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
        prepared: Option<crate::runtime::state::PreparedProjectResolution>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = prepared.ok_or_else(|| DaemonError::LocalTransport {
            operation: "project.resolve",
            message: "Project folder was not prepared; retry".into(),
        })?;
        self.state
            .resolve_prepared_project(request, caller, prepared)
    }
}
