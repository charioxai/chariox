//! Ephemeral external command authority for kernel commands and their continuations.
use super::*;

#[derive(Clone)]
pub(super) struct ExternalCommandAuthority {
    pub(super) grant_id: String,
    request: LocalDaemonRequest,
}

impl std::fmt::Debug for ExternalCommandAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalCommandAuthority")
            .field("grant_id", &self.grant_id)
            .finish_non_exhaustive()
    }
}

impl ExternalCommandAuthority {
    pub(super) fn as_request(&self) -> (&str, &LocalDaemonRequest) {
        (&self.grant_id, &self.request)
    }
}

impl KernelRuntimeState {
    pub(super) async fn refresh_remote_agent_binding_authorized(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_external_command()?;
        let state = self.clone();
        let agent_id = agent_id.to_owned();
        self.with_app_side_effect_blocking(move |app| {
            app.refresh_remote_agent_binding_authorized(&agent_id, &|| {
                state.authorize_current_external_command()
            })
        })
        .await
    }

    pub(super) async fn with_authorized_app_side_effect<R>(
        &self,
        operation: impl FnOnce(&mut crate::DaemonApp) -> Result<R, DaemonError>,
    ) -> Result<R, DaemonError> {
        self.authorize_current_external_command()?;
        self.with_app_side_effect(|app| {
            self.authorize_current_external_command()?;
            operation(app)
        })
        .await
    }

    pub(crate) fn with_external_command_authority(
        &self,
        authority: Option<(&str, &LocalDaemonRequest)>,
    ) -> Self {
        let mut state = self.clone();
        state.external_command_authority =
            authority.map(|(grant_id, request)| ExternalCommandAuthority {
                grant_id: grant_id.to_owned(),
                request: request.clone(),
            });
        state
    }

    pub(crate) fn authorize_current_external_command(&self) -> Result<(), DaemonError> {
        self.authorize_current_external_response()?;
        if let Some((actor, request)) = self.room_request_origin.as_ref() {
            self.authorize_room_agent_request(actor, request)?;
            self.owned
                .ensure_workflow_request_controlled_by_metaagent(request, actor)?;
        }
        Ok(())
    }

    /// MP-11 F5: retain epoch, peer and grant checks on delivery. A successful
    /// mutation may have renamed or deleted its original object reference.
    pub(crate) fn authorize_current_external_response(&self) -> Result<(), DaemonError> {
        self.authorize_current_forwarded_binding()?;
        if let Some((actor, run)) = self.room_provider_origin.as_ref() {
            self.authorize_room_provider_epoch(Some(actor), Some(run))?;
        }
        self.authorize_prompt_command(
            self.external_command_authority
                .as_ref()
                .map(ExternalCommandAuthority::as_request),
        )
    }
}
