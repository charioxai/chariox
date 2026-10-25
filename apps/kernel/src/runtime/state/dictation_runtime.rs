//! Kernel dictation requests; the service owns the model, uploads and transcription.
use super::*;
use crate::local::{DictationCommand, DictationResult};

impl KernelRuntimeState {
    pub(crate) async fn dictation_request(
        &self,
        owner: &str,
        command: DictationCommand,
    ) -> Result<DictationResult, DaemonError> {
        self.owned.dictation.handle(owner, command).await
    }
}
