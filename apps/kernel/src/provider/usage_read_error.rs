use std::time::Duration;

use crate::error::DaemonError;

/// Why a provider's account usage could not be read. The kernel keeps the last
/// reading and waits at least `retry_after` when the provider named a wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderUsageReadError {
    pub(crate) message: String,
    pub(crate) retry_after: Option<Duration>,
}

impl ProviderUsageReadError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_after: None,
        }
    }

    pub(crate) fn retry_after(mut self, wait: Option<Duration>) -> Self {
        self.retry_after = wait;
        self
    }
}

impl From<DaemonError> for ProviderUsageReadError {
    fn from(error: DaemonError) -> Self {
        Self::new(error.to_string())
    }
}
