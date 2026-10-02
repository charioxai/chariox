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

    pub(super) fn authorize_current_external_command(&self) -> Result<(), DaemonError> {
        self.authorize_prompt_command(
            self.external_command_authority
                .as_ref()
                .map(ExternalCommandAuthority::as_request),
        )
    }
}
