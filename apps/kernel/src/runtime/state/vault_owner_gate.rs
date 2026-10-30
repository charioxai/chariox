//! Who may change the Vault and the credentials stored in it. One gate for
//! every such request, checked where the request enters the router.
use super::vault_unlock_request::refusal;
use super::*;
use crate::local::{KernelConnectionClass, LocalDaemonRequest};
use crate::runtime::command::KernelCommand;

mod wording {
    pub(super) const LOCK: &str = "Only the owner of this kernel can lock its Vault.";
    pub(super) const ENROLL: &str =
        "Only a person at a Chariox client can sign in a provider account. Ask the owner to do it.";
    pub(super) const CHANGE: &str =
        "Only the owner of this kernel or its agents can change what the Vault stores.";
}

/// What the request changes, and so who may ask.
enum Who {
    /// The owner's own client.
    Owner,
    /// A person at a client: the sign-in of a provider account.
    Person,
    /// The owner's client or an agent working for the owner: an agent's own
    /// secret requests still wait for the owner's unlock when the Vault is
    /// locked.
    OwnerOrAgent,
}

fn who_may(request: &LocalDaemonRequest) -> Option<(Who, &'static str, &'static str)> {
    Some(match request {
        LocalDaemonRequest::LockCredentialVault(_) => {
            (Who::Owner, "vault_lock_owner_only", wording::LOCK)
        }
        LocalDaemonRequest::SetProviderAccountCredential(_) => (
            Who::Person,
            "provider_account_sign_in_person_only",
            wording::ENROLL,
        ),
        LocalDaemonRequest::SetCredentialSecret(_)
        | LocalDaemonRequest::DeleteCredentialSecret(_)
        | LocalDaemonRequest::RegisterCredential(_)
        | LocalDaemonRequest::UpsertCredential(_)
        | LocalDaemonRequest::RemoveCredential(_) => (
            Who::OwnerOrAgent,
            "vault_change_owner_only",
            wording::CHANGE,
        ),
        _ => return None,
    })
}

impl KernelRuntimeState {
    /// `Ok` for every request that is not a Vault or credential change.
    pub(crate) fn ensure_vault_change_allowed(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<(), DaemonError> {
        let Some((who, code, message)) = who_may(request) else {
            return Ok(());
        };
        let class = command.caller.connection_class;
        let host = class == Some(KernelConnectionClass::Host);
        let allowed = host
            || match who {
                Who::Owner => self.owner_client_user_id(command).is_ok(),
                Who::Person => command.is_terminal_caller(),
                Who::OwnerOrAgent => {
                    self.owner_client_user_id(command).is_ok()
                        || command.caller.metaagent_id.is_some()
                        || matches!(
                            class,
                            Some(
                                KernelConnectionClass::KernelAgent
                                    | KernelConnectionClass::ExternalAgent
                            )
                        )
                }
            };
        if allowed {
            Ok(())
        } else {
            Err(refusal(code, message, false))
        }
    }
}
