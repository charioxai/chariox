//! The owner binds one of their Vault entries to a credentialed destination an
//! installation's signed manifest declares. Only the owner can: there is no
//! agent tool for it, and the App sees an account's label, never its value.
use super::KernelRuntimeState;
use crate::durable_state::app_accounts::{AccountBinding, AccountCommand};
use crate::durable_state::app_active_release::ActiveReleaseError;
use crate::local::{
    AppAccountSummary, AppCredentialSummary, AppRequestErrorCode, LocalDaemonRequest,
    LocalDaemonResponse,
};
use chariox_app_package::NetworkDestination;

fn account_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl KernelRuntimeState {
    pub(super) async fn app_account_request(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        // An owner can still list and revoke while the release cannot be read.
        let destinations = self
            .app_credential_destinations(&owner, &installation)
            .await;
        let command = match request {
            LocalDaemonRequest::GrantAppAccount(request) => {
                let destination = destinations
                    .as_ref()
                    .map_err(Clone::clone)?
                    .iter()
                    .find(|destination| {
                        destination.credential.as_deref() == Some(request.credential.as_str())
                    })
                    .ok_or(AppRequestErrorCode::InvalidRequest)?;
                if !account_name(&request.account) {
                    return Err(AppRequestErrorCode::InvalidRequest);
                }
                // Refuses an entry that was not made for HTTP use at this
                // origin; the value is neither read nor unlocked here.
                self.home_runtime_secret_service()
                    .and_then(|service| {
                        service
                            .validate_app_account(&request.vault_credential_id, &destination.origin)
                            .map(|_| ())
                    })
                    .map_err(|_| AppRequestErrorCode::InvalidRequest)?;
                Some(AccountCommand::Bind {
                    owner: owner.clone(),
                    installation: installation.clone(),
                    binding: AccountBinding {
                        account: request.account,
                        credential: request.credential,
                        vault_credential_id: request.vault_credential_id,
                        allow: request.allow,
                        bound_at_ms: crate::session::unix_epoch_ms(),
                    },
                })
            }
            LocalDaemonRequest::RevokeAppAccount(request) => Some(AccountCommand::Revoke {
                owner: owner.clone(),
                installation: installation.clone(),
                account: request.account,
            }),
            LocalDaemonRequest::ListAppAccounts(_) => None,
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        };
        let store = self.owned.durable_state_store.clone();
        let permit = self.app_control().try_admit()?;
        let (list_owner, list_installation) = (owner.clone(), installation.clone());
        let accounts = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(command) = command {
                store.app_account_command(command)?;
            }
            store.app_accounts(&list_owner, &list_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|code| match code {
            "NOT_FOUND" => AppRequestErrorCode::NotFound,
            "LIMIT_EXCEEDED" => AppRequestErrorCode::LimitExceeded,
            _ => AppRequestErrorCode::StorageUnavailable,
        })?;
        Ok(LocalDaemonResponse::AppAccounts {
            installation_id: installation,
            credentials: destinations
                .unwrap_or_default()
                .iter()
                .filter_map(|destination| {
                    Some(AppCredentialSummary {
                        credential: destination.credential.clone()?,
                        origin: destination.origin.clone(),
                        routes: destination.routes.len() as u32,
                        safety: destination.routes.iter().map(|route| route.safety).max()?,
                    })
                })
                .collect(),
            accounts: accounts
                .into_iter()
                .map(|binding| AppAccountSummary {
                    account: binding.account,
                    credential: binding.credential,
                    vault_credential_id: binding.vault_credential_id,
                    allow: binding.allow,
                    bound_at_ms: binding.bound_at_ms,
                })
                .collect(),
        })
    }

    async fn app_credential_destinations(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<NetworkDestination>, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.active_app_credentials(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            ActiveReleaseError::NotActive => AppRequestErrorCode::NotFound,
            ActiveReleaseError::Untrusted | ActiveReleaseError::Invalid => {
                AppRequestErrorCode::Conflict
            }
            ActiveReleaseError::Unavailable | ActiveReleaseError::Storage => {
                AppRequestErrorCode::StorageUnavailable
            }
        })
    }
}
