//! Native Move checks that can run without stopping or resuming a provider session.
use super::{codex, NativeCheckpointBinding, NativeCheckpointFailure as Failure};
use crate::provider::{CodexClient, ProviderClientInterface, RuntimeProviderRun};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMoveCompatibility {
    provider: String,
    interface: ProviderClientInterface,
    source_os: String,
    cli_version: String,
    account_sha256: String,
}

impl NativeMoveCompatibility {
    pub(crate) fn from_run(run: &RuntimeProviderRun) -> Result<Self, Failure> {
        super::supported_interface(run.provider(), run.client_interface())?;
        let root = super::profile_root(run.pty_env())?;
        let endpoint = crate::provider::ensure_codex_account_endpoint(
            run.owner_user_id(),
            run.account_profile(),
            BTreeMap::from([("CODEX_HOME".into(), root.display().to_string())]),
        )
        .map_err(|_| Failure::AccountUnavailable)?;
        let client =
            CodexClient::new(run.id(), endpoint).map_err(|_| Failure::AccountUnavailable)?;
        let auth = codex::auth(&client, run.account_profile())?;
        Ok(Self {
            provider: run.provider().into(),
            interface: run.client_interface(),
            source_os: std::env::consts::OS.into(),
            cli_version: auth
                .detected_version
                .clone()
                .ok_or(Failure::UnsupportedVersion)?,
            account_sha256: codex::account_digest(&auth)?,
        })
    }

    pub(crate) fn validate_target(&self) -> Result<(), Failure> {
        super::supported_interface(&self.provider, self.interface)?;
        super::supported_os(&self.source_os)?;
        codex::supported_version(&self.cli_version)?;
        let version = crate::provider::codex_client::codex_version()
            .map_err(|_| Failure::UnsupportedVersion)?;
        codex::supported_version(&version)
    }

    pub(crate) fn verify_account(
        &self,
        owner: &str,
        profile: &str,
        environment: BTreeMap<String, String>,
    ) -> Result<(), Failure> {
        let endpoint = crate::provider::ensure_codex_account_endpoint(owner, profile, environment)
            .map_err(|_| Failure::AccountUnavailable)?;
        let client = CodexClient::new("move-account-preflight", endpoint)
            .map_err(|_| Failure::AccountUnavailable)?;
        let auth = codex::auth(&client, profile)?;
        if codex::account_digest(&auth)? != self.account_sha256 {
            return Err(Failure::AccountMismatch);
        }
        Ok(())
    }
}

impl NativeCheckpointBinding {
    pub(crate) fn move_compatibility(&self) -> NativeMoveCompatibility {
        NativeMoveCompatibility {
            provider: self.provider.clone(),
            interface: ProviderClientInterface::Chariox,
            source_os: self.source_os.clone(),
            cli_version: self.cli_version.clone(),
            account_sha256: self.account_sha256.clone(),
        }
    }
}
