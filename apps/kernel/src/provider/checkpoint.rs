//! Native provider state is a private artifact, separate from portable accounts.
//! A checkpoint prepares continuity; it never grants an execution lease.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ProviderClientInterface, RuntimeProviderRun};

mod codex;
#[cfg(unix)]
mod files;
#[cfg(test)]
mod tests;

pub const MAX_NATIVE_CHECKPOINT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NativeCheckpointFailure {
    #[error("This provider session cannot move to this target yet: native checkpoint support is unavailable for this provider.")]
    UnsupportedProvider,
    #[error("This provider session cannot move to this target yet: only the Chariox Codex interface has been verified.")]
    UnsupportedInterface,
    #[error("This provider session cannot move to this target yet: this operating-system pair has not been verified.")]
    UnsupportedPlatform,
    #[error("This provider session cannot move to this target yet: use the verified Codex CLI version 0.159.3 on both kernels.")]
    UnsupportedVersion,
    #[error("Finish the current turn and confirm provider teardown before exporting its native session.")]
    NotQuiescent,
    #[error("The provider has no durable native session to resume. Finish a provider turn before retrying.")]
    MissingSession,
    #[error("The selected account cannot be identified by the official provider. Refresh the account before retrying.")]
    AccountUnavailable,
    #[error("The target uses a different authenticated account. Transfer or select the original account before retrying.")]
    AccountMismatch,
    #[error("The checkpoint's provider, model or effort differs from the requested launch. Keep the checkpoint's configuration or make an explicit context handoff.")]
    ConfigurationMismatch,
    #[error("The native checkpoint is invalid or incomplete. Export it again from the stopped source provider.")]
    InvalidArtifact,
    #[error("The native checkpoint exceeds the 128 MiB limit. This session cannot move to this target yet.")]
    TooLarge,
    #[error("The native session changed during export. Finish writes to that session and retry.")]
    SourceChanged,
    #[error("The target already has different state for this native session. Preserve that state and choose a fresh destination profile.")]
    TargetConflict,
    #[error("The target provider profile cannot stage the checkpoint. Check its free space and directory permissions, then retry.")]
    StorageUnavailable,
    #[error("Native resume failed. The required session was retained; restore its checkpoint or retry on the source kernel.")]
    ResumeFailed,
    #[error("The provider returned a different native session. No prompt was submitted; restore the checkpoint before retrying.")]
    SessionChanged,
}

/// Safe persisted launch constraint. The native payload never enters an agent
/// projection, history entry, generic launch request or account transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredNativeSession {
    pub(crate) provider: String,
    pub(crate) session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<NativeCheckpointBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointBinding {
    provider: String,
    session_id: String,
    account_sha256: String,
    model: String,
    variant: Option<String>,
    cli_version: String,
    source_os: String,
    artifact_sha256: String,
}

/// The selected native file, not an account root or a reconstructed transcript.
/// Serialization is reserved for protected artifact transport, never snapshots.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderNativeCheckpoint {
    format: String,
    binding: NativeCheckpointBinding,
    relative_path: String,
    contents: Vec<u8>,
}

impl fmt::Debug for ProviderNativeCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderNativeCheckpoint")
            .field("format", &self.format)
            .field("bytes", &self.contents.len())
            .field("sha256", &self.binding.artifact_sha256)
            .finish_non_exhaustive()
    }
}

impl ProviderNativeCheckpoint {
    pub fn export(run: &RuntimeProviderRun) -> Result<Self, NativeCheckpointFailure> {
        supported_interface(run.adapter_key(), run.client_interface())?;
        if run.state() != super::ProviderRunState::Ended {
            return Err(NativeCheckpointFailure::NotQuiescent);
        }
        codex::export(run)
    }

    /// Stage and verify the private native file without submitting user work.
    /// The caller supplies a target launch planned by the ordinary adapter, so
    /// profile resolution and provider startup retain their existing authority.
    pub(crate) fn import(
        &self,
        request: &super::LaunchProviderRequest,
        launch: &super::ProviderLaunchResult,
    ) -> Result<super::ProviderResumeState, NativeCheckpointFailure> {
        supported_interface(&request.adapter_key, request.client_interface)?;
        self.validate()?;
        codex::import(self, request, launch)
    }

    pub fn provider_session_id(&self) -> &str {
        &self.binding.session_id
    }

    pub fn binding(&self) -> &NativeCheckpointBinding {
        &self.binding
    }

    pub fn sha256(&self) -> &str {
        &self.binding.artifact_sha256
    }

    fn validate(&self) -> Result<(), NativeCheckpointFailure> {
        if self.contents.len() > MAX_NATIVE_CHECKPOINT_BYTES {
            return Err(NativeCheckpointFailure::TooLarge);
        }
        if self.format != "codex-jsonl-v1"
            || self.binding.provider != "codex"
            || self.binding.model.is_empty()
            || self.binding.model == "default"
            || self.binding.account_sha256.len() != 64
            || !self
                .binding
                .account_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || self.contents.is_empty()
            || digest(&self.contents) != self.binding.artifact_sha256
            || !codex::valid_relative_path(&self.relative_path, &self.binding.session_id)
        {
            return Err(NativeCheckpointFailure::InvalidArtifact);
        }
        codex::supported_version(&self.binding.cli_version)?;
        supported_os(&self.binding.source_os)
    }
}

pub(crate) fn supported_interface(
    provider: &str,
    interface: ProviderClientInterface,
) -> Result<(), NativeCheckpointFailure> {
    if provider != "codex" {
        return Err(NativeCheckpointFailure::UnsupportedProvider);
    }
    if interface != ProviderClientInterface::Chariox {
        return Err(NativeCheckpointFailure::UnsupportedInterface);
    }
    supported_os(std::env::consts::OS)
}

fn supported_os(source_os: &str) -> Result<(), NativeCheckpointFailure> {
    if source_os != "linux" || std::env::consts::OS != "linux" {
        return Err(NativeCheckpointFailure::UnsupportedPlatform);
    }
    Ok(())
}

fn digest(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn profile_root(
    environment: &std::collections::BTreeMap<String, String>,
) -> Result<&Path, NativeCheckpointFailure> {
    environment
        .get("CODEX_HOME")
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .ok_or(NativeCheckpointFailure::AccountUnavailable)
}

impl RequiredNativeSession {
    pub(crate) fn validate_request(
        &self,
        request: &super::LaunchProviderRequest,
    ) -> Result<(), NativeCheckpointFailure> {
        supported_interface(&request.adapter_key, request.client_interface)?;
        if self.provider != request.adapter_key || self.session_id.is_empty() {
            return Err(NativeCheckpointFailure::MissingSession);
        }
        if request
            .resume_state
            .as_ref()
            .and_then(|state| state.provider_session_id(&self.provider))
            != Some(self.session_id.as_str())
        {
            return Err(NativeCheckpointFailure::SessionChanged);
        }
        if let Some(binding) = &self.checkpoint {
            self.validate_configuration(&request.model, request.variant.as_deref(), binding)?;
        }
        Ok(())
    }

    pub(crate) fn verify_run(
        &self,
        run: &RuntimeProviderRun,
        client: &super::CodexClient,
    ) -> Result<(), NativeCheckpointFailure> {
        supported_interface(run.adapter_key(), run.client_interface())?;
        if self.provider != run.adapter_key()
            || run.resume_state().provider_session_id(run.adapter_key())
                != Some(self.session_id.as_str())
        {
            return Err(NativeCheckpointFailure::SessionChanged);
        }
        if let Some(binding) = &self.checkpoint {
            self.validate_configuration(run.model(), run.variant(), binding)?;
            codex::verify_required_account(client, run.account_profile(), binding)?;
        }
        Ok(())
    }

    fn validate_configuration(
        &self,
        model: &str,
        variant: Option<&str>,
        binding: &NativeCheckpointBinding,
    ) -> Result<(), NativeCheckpointFailure> {
        if binding.provider != self.provider || binding.session_id != self.session_id {
            return Err(NativeCheckpointFailure::InvalidArtifact);
        }
        codex::supported_version(&binding.cli_version)?;
        supported_os(&binding.source_os)?;
        let model = model.strip_prefix("codex/").unwrap_or(model);
        let variant = variant
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "default");
        if model != binding.model || variant != binding.variant.as_deref() {
            return Err(NativeCheckpointFailure::ConfigurationMismatch);
        }
        Ok(())
    }

    pub(crate) fn confirm_session(&self, id: &str) -> Result<(), NativeCheckpointFailure> {
        if id != self.session_id {
            return Err(NativeCheckpointFailure::SessionChanged);
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod live;

impl NativeCheckpointBinding {
    pub(crate) fn requirement(&self) -> RequiredNativeSession {
        RequiredNativeSession {
            provider: self.provider.clone(),
            session_id: self.session_id.clone(),
            checkpoint: Some(self.clone()),
        }
    }
}

impl From<NativeCheckpointFailure> for crate::DaemonError {
    fn from(reason: NativeCheckpointFailure) -> Self {
        Self::NativeProviderCheckpoint { reason }
    }
}
