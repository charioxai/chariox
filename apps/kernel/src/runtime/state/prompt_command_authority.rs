//! Ephemeral external command authority for prompt policy work and its continuations.
use super::*;

#[derive(Clone)]
pub(super) struct PromptCommandAuthority {
    pub(super) grant_id: String,
    request: LocalDaemonRequest,
}

impl std::fmt::Debug for PromptCommandAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptCommandAuthority")
            .field("grant_id", &self.grant_id)
            .finish_non_exhaustive()
    }
}

impl PromptCommandAuthority {
    pub(super) fn as_request(&self) -> (&str, &LocalDaemonRequest) {
        (&self.grant_id, &self.request)
    }
}

impl KernelRuntimeState {
    pub(crate) fn with_prompt_command_authority(
        &self,
        authority: Option<(&str, &LocalDaemonRequest)>,
    ) -> Self {
        let mut state = self.clone();
        state.prompt_command_authority =
            authority.map(|(grant_id, request)| PromptCommandAuthority {
                grant_id: grant_id.to_owned(),
                request: request.clone(),
            });
        state
    }

    pub(super) fn authorize_current_prompt_command(&self) -> Result<(), DaemonError> {
        self.authorize_prompt_command(
            self.prompt_command_authority
                .as_ref()
                .map(PromptCommandAuthority::as_request),
        )
    }
}
