//! MP-08 / MP-09 / MP-10 / MP-11: preparation stays outside the shared Project commit lane.
use super::*;
use crate::runtime::state::PreparedProjectEntry;
impl SessionRuntimeStore {
    pub(in crate::runtime::session_actor) async fn prepare_project_resolution(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: &LocalDaemonRequest,
        actor: Option<&str>,
        caller: &str,
    ) -> Result<Option<PreparedProjectEntry>, DaemonError> {
        if !matches!(
            request,
            LocalDaemonRequest::ResolveProject(_) | LocalDaemonRequest::CreateSession(_)
        ) {
            return Ok(None);
        }
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
        match request {
            LocalDaemonRequest::ResolveProject(resolution) => state
                .prepare_project_resolution(resolution.clone(), caller)
                .await
                .map(|prepared| Some(PreparedProjectEntry::Resolution(Box::new(prepared)))),
            LocalDaemonRequest::CreateSession(creation) => Ok(state
                .prepare_session_workspace(creation, caller)
                .await?
                .map(PreparedProjectEntry::SessionWorkspace)),
            _ => Ok(None),
        }
    }
    pub(in crate::runtime::session_actor) fn resolve_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
        prepared: Option<PreparedProjectEntry>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let Some(PreparedProjectEntry::Resolution(prepared)) = prepared else {
            return Err(DaemonError::LocalTransport {
                operation: "project.resolve",
                message: "Project folder was not prepared; retry".into(),
            });
        };
        self.state
            .resolve_prepared_project(request, caller, *prepared)
    }
}
