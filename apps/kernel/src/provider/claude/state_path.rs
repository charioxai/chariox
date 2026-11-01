//! MP-08 / MP-09 / MP-10 / MP-11: match the selected launch's native state scope.
use std::path::PathBuf;

use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;

pub(super) fn for_launch(request: Option<&LaunchProviderRequest>) -> Result<PathBuf, DaemonError> {
    let configured = request
        .and_then(|request| {
            request
                .provider_account_env
                .get(super::CLAUDE_CONFIG_DIR_ENV)
        })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            // Explicit account values replace inherited values; a removal only
            // suppresses the inherited profile, just as at process launch.
            if request.is_some_and(|request| {
                request
                    .provider_env_remove
                    .iter()
                    .any(|key| key == super::CLAUDE_CONFIG_DIR_ENV)
            }) {
                None
            } else {
                std::env::var_os(super::CLAUDE_CONFIG_DIR_ENV)
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            }
        });
    let directory = configured
        .or_else(|| {
            request.and_then(|request| {
                request
                    .provider_account_env
                    .get("HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| {
                        request
                            .provider_credential_env
                            .iter()
                            .find(|(name, value)| *name == "HOME" && !value.is_empty())
                            .map(|(_, value)| PathBuf::from(value))
                    })
            })
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "initialize Claude headless onboarding state",
            message: "HOME is unavailable".into(),
        })?;
    Ok(directory.join(super::CLAUDE_HEADLESS_STATE_FILE))
}
