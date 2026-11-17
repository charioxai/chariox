//! The launch credential of a leased provider process, held in memory for as long as the process
//! runs. A turn that goes on in a new process after its settings changed is relaunched by the
//! worker with it; the home is not asked again in the middle of a turn.

use std::collections::BTreeMap;

use crate::transport::relay_peer::RemoteProviderLaunchCredential;

/// One credential per backing agent, with the run that was launched with it.
#[derive(Default)]
pub(crate) struct LeasedLaunchCredentials {
    inner: std::sync::Mutex<BTreeMap<String, (String, RemoteProviderLaunchCredential)>>,
}

impl LeasedLaunchCredentials {
    fn held(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, (String, RemoteProviderLaunchCredential)>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn keep(
        &self,
        backing_agent_id: &str,
        provider_run_id: &str,
        credential: RemoteProviderLaunchCredential,
    ) {
        self.held().insert(
            backing_agent_id.to_string(),
            (provider_run_id.to_string(), credential),
        );
    }

    /// The credential the agent's process was launched with, when it is for `provider` and
    /// `account_profile`: a new account needs the home's credential, which the worker does not hold.
    pub(crate) fn for_profile(
        &self,
        backing_agent_id: &str,
        provider: &str,
        account_profile: &str,
    ) -> Option<RemoteProviderLaunchCredential> {
        self.held()
            .get(backing_agent_id)
            .map(|(_, credential)| credential)
            .filter(|credential| {
                crate::provider::canonical_provider_family(provider)
                    == crate::provider::canonical_provider_family(&credential.provider)
                    && credential.account_profile == account_profile
            })
            .cloned()
    }

    pub(crate) fn forget(&self, backing_agent_id: &str) {
        self.held().remove(backing_agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::relay_peer::RemoteCredentialSecretInput;

    fn credential(account: &str) -> RemoteProviderLaunchCredential {
        RemoteProviderLaunchCredential {
            provider: "claude".into(),
            account_profile: account.into(),
            secret_input: RemoteCredentialSecretInput::new("token".into()),
        }
    }

    #[test]
    fn a_credential_serves_only_the_account_it_was_launched_with() {
        let held = LeasedLaunchCredentials::default();
        held.keep("agent", "run-1", credential("work"));
        assert!(held.for_profile("agent", "claude", "work").is_some());
        assert!(held.for_profile("agent", "claude", "personal").is_none());
        assert!(held.for_profile("other-agent", "claude", "work").is_none());
        held.forget("agent");
        assert!(held.for_profile("agent", "claude", "work").is_none());
    }
}
