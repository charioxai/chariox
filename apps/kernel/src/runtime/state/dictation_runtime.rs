//! Kernel dictation requests; the service owns the model, uploads and transcription.
use super::*;
use crate::local::{DictationCommand, DictationResult};

impl KernelRuntimeState {
    pub(crate) async fn dictation_request(
        &self,
        command_context: &crate::runtime::command::KernelCommand,
        owner: &str,
        command: DictationCommand,
    ) -> Result<DictationResult, DaemonError> {
        let remote =
            self.owned.dictation.selected_service() != crate::local::DictationProvider::Local;
        let _vault_unlock = if let DictationCommand::Transcribe { context, .. } = &command {
            if remote {
                if let Some(context) = context {
                    let session = self.owned.session_store.get_session(&context.session_id)?;
                    if session.owner_user_id() != owner {
                        return Err(DaemonError::RelayTransport {
                            operation: "dictation",
                            code: "dictation_context_invalid".into(),
                            message: "Dictation can only open a Vault prompt in your own Room."
                                .into(),
                            retryable: false,
                        });
                    }
                }
                Some(
                    self.ensure_vault_unlocked_for_command_context(
                        command_context,
                        context.as_ref().map(|context| context.session_id.as_str()),
                        context.as_ref().map(|context| context.agent_id.as_str()),
                        "dictation",
                    )
                    .await
                    .map_err(|_| DaemonError::RelayTransport {
                        operation: "dictation",
                        code: "dictation_vault_locked".into(),
                        message: "Unlock your kernel's Vault, then try dictation again.".into(),
                        retryable: false,
                    })?,
                )
            } else {
                None
            }
        } else {
            None
        };
        // Unlock can wait for human input; resolve the Vault configuration only after that wait.
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        self.owned
            .dictation
            .handle_with_vault(owner, command, if remote { Some(config) } else { None })
            .await
    }
}
