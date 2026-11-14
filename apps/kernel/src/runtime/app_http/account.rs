//! An owner's account for a credentialed destination: the Vault entry's
//! `Authorization` value is read here, inside the kernel, when the request is
//! admitted. The App never receives it, and it is scrubbed from the answer.
use super::{HttpError, Result};
use crate::runtime::app_lifecycle::EventConfig;
use zeroize::Zeroizing;

pub(crate) trait AccountVault: Send + Sync {
    /// The `Authorization` header value of the Vault entry `credential_id`,
    /// which must be bound to HTTP use at exactly `origin`.
    fn authorization(&self, credential_id: &str, origin: &str) -> Result<Zeroizing<String>>;
}

/// Reads the kernel's own Vault through its configuration.
pub(crate) struct KernelAccountVault(pub(crate) EventConfig);

impl AccountVault for KernelAccountVault {
    fn authorization(&self, credential_id: &str, origin: &str) -> Result<Zeroizing<String>> {
        let config = self.0.get().ok_or(HttpError::AccountUnavailable)?;
        let service = crate::secret::RuntimeSecretService::with_vault_config(
            crate::credential::load_user_credentials()
                .map_err(|_| HttpError::AccountUnavailable)?,
            &config.snapshot().user_config.credential_vault,
        )
        .map_err(|_| HttpError::AccountUnavailable)?;
        service
            .app_account_authorization(credential_id, origin)
            .map_err(|_| HttpError::AccountUnavailable)
    }
}

/// The owner's accounts for one installation, read when a request is admitted.
pub(super) struct Accounts<'a> {
    pub(super) store: &'a crate::durable_state::DurableKernelStateStore,
    pub(super) owner: &'a str,
    pub(super) installation: &'a str,
    pub(super) vault: &'a dyn AccountVault,
}

impl Accounts<'_> {
    /// A call as `account:<label>`: the signed route table decides what it is,
    /// the owner's binding decides whether it may be made, and only then is
    /// the Vault value read and attached.
    pub(super) fn target(
        &self,
        policy: &super::policy::AppHttpPolicy,
        connection: &str,
        url: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<super::policy::ApprovedTarget> {
        let label = connection
            .strip_prefix("account:")
            .filter(|label| !label.is_empty())
            .ok_or(HttpError::ConnectionAuthority)?;
        let call = policy.account_target(url, method, headers)?;
        let binding = self
            .store
            .app_account(self.owner, self.installation, label)
            .map_err(|_| HttpError::Busy)?
            .filter(|binding| binding.credential == call.credential)
            .ok_or(HttpError::AccountUnknown)?;
        if binding.allow < call.safety {
            return Err(HttpError::AccountNotAllowed);
        }
        let authorization = self
            .vault
            .authorization(&binding.vault_credential_id, &call.origin)?;
        Ok(call.target.with_authorization(authorization))
    }
}

/// A Vault holding at most one entry's value, for tests.
#[cfg(test)]
pub(crate) struct FixtureVault(pub(crate) Option<String>);
#[cfg(test)]
impl AccountVault for FixtureVault {
    fn authorization(&self, _: &str, _: &str) -> Result<Zeroizing<String>> {
        self.0
            .clone()
            .map(Zeroizing::new)
            .ok_or(HttpError::AccountUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::app_accounts::{AccountBinding, AccountCommand};
    use crate::durable_state::DurableKernelStateStore;
    use chariox_app_package::RouteSafety;

    const PULL: &str = "https://api.github.com/repos/o/r/pulls/7";
    const MERGE: &str = "https://api.github.com/repos/o/r/pulls/7/merge";

    #[test]
    fn a_call_needs_a_route_the_owners_binding_and_a_readable_entry_in_that_order() {
        let root = std::env::temp_dir().join(format!("chariox-accounts-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let policy = super::super::policy::fixture_account_policy(
            crate::durable_state::app_state::fixture_http_catalog(&store)
                .app_catalog()
                .clone(),
        );
        let bind = |account: &str, credential: &str, allow| {
            store
                .app_account_command(AccountCommand::Bind {
                    owner: "alice".into(),
                    installation: "github".into(),
                    binding: AccountBinding {
                        account: account.into(),
                        credential: credential.into(),
                        vault_credential_id: format!("entry-{account}"),
                        allow,
                        bound_at_ms: 1,
                    },
                })
                .unwrap()
        };
        let call = |vault: &FixtureVault, connection: &str, url: &str, method: &str| {
            Accounts { store: &store, owner: "alice", installation: "github", vault }
                .target(&policy, connection, url, method, &[])
                .map(|target| target.authorization().map(str::to_owned))
        };
        let vault = FixtureVault(Some("Bearer ghp_X".into()));

        // No binding yet, then one for another credential label: unknown.
        assert_eq!(call(&vault, "account:work", PULL, "GET"), Err(HttpError::AccountUnknown));
        bind("work", "gitlab", RouteSafety::Destructive);
        assert_eq!(call(&vault, "account:work", PULL, "GET"), Err(HttpError::AccountUnknown));
        // A read binding reads, and is refused the destructive route.
        bind("work", "github", RouteSafety::Read);
        assert_eq!(call(&vault, "account:work", PULL, "GET"), Ok(Some("Bearer ghp_X".into())));
        assert_eq!(call(&vault, "account:work", MERGE, "PUT"), Err(HttpError::AccountNotAllowed));
        bind("work", "github", RouteSafety::Destructive);
        assert!(call(&vault, "account:work", MERGE, "PUT").is_ok());
        // Another account of the same installation is its own binding.
        assert_eq!(call(&vault, "account:home", PULL, "GET"), Err(HttpError::AccountUnknown));
        // The route table is checked before anything is read from the Vault.
        let empty = FixtureVault(None);
        assert_eq!(call(&empty, "account:work", "https://api.github.com/user", "GET"), Err(HttpError::RouteNotDeclared));
        assert_eq!(call(&empty, "account:work", PULL, "GET"), Err(HttpError::AccountUnavailable));
        // Only `account:<label>` names an account.
        for connection in ["work", "slack-1", "account:", "Account:work"] {
            assert_eq!(call(&vault, connection, PULL, "GET"), Err(HttpError::ConnectionAuthority), "{connection}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
