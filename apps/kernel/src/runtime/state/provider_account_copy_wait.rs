//! Short-lived state for an explicit account copy waiting for the receiving Vault.
use super::*;
use crate::provider_account_copy::{
    failure, ProviderAccountCopyBinding, ProviderAccountCopyOutcome, ProviderAccountCopyProgress,
    ProviderAccountCopyReceipt, ProviderAccountCopyUnlockRequest, ProviderAccountCopyWait,
};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::Path;
use std::sync::Mutex as SyncMutex;
use tokio::time::Instant as Timer;

pub(super) const COPY_UNLOCK_WAIT: Duration = Duration::from_secs(600);
const COPY_RETRY_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Default)]
pub(super) struct AccountCopyWaits {
    incoming: Arc<SyncMutex<BTreeMap<String, Incoming>>>,
    outgoing: Arc<SyncMutex<BTreeMap<String, Outgoing>>>,
}

struct Incoming {
    owner: String,
    binding: ProviderAccountCopyBinding,
    deadline: Timer,
    expires_at_ms: u64,
    status: IncomingStatus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IncomingStatus {
    Waiting,
    Unlocked,
    Cancelled,
}

struct Outgoing {
    owner: String,
    binding: ProviderAccountCopyBinding,
    remove_at: Timer,
    progress: ProviderAccountCopyProgress,
}

impl KernelRuntimeState {
    pub(crate) fn account_copy_unlock_requests(
        &self,
        caller: &str,
    ) -> Vec<ProviderAccountCopyUnlockRequest> {
        let owner = self.provider_account_authority_owner_user_id(caller);
        self.account_copy_waits
            .incoming
            .lock()
            .expect("copy waits poisoned")
            .iter()
            .filter(|(_, copy)| {
                copy.owner == owner
                    && copy.status == IncomingStatus::Waiting
                    && copy.deadline > Timer::now()
            })
            .map(|(interaction_id, copy)| ProviderAccountCopyUnlockRequest {
                interaction_id: interaction_id.clone(),
                expires_at_ms: copy.expires_at_ms,
            })
            .collect()
    }

    pub(crate) fn provider_account_copy_progress(
        &self,
        owner: &str,
    ) -> Vec<ProviderAccountCopyProgress> {
        self.account_copy_waits
            .outgoing
            .lock()
            .expect("copy waits poisoned")
            .values()
            .filter(|copy| copy.owner == owner && copy.remove_at > Timer::now())
            .map(|copy| copy.progress.clone())
            .collect()
    }

    pub(super) fn existing_account_copy(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
    ) -> Result<Option<ProviderAccountCopyOutcome>, DaemonError> {
        let mut copies = self
            .account_copy_waits
            .outgoing
            .lock()
            .expect("copy waits poisoned");
        copies.retain(|_, copy| copy.remove_at > Timer::now());
        let Some(copy) = copies.get(&binding.operation_id) else {
            return Ok(None);
        };
        if copy.owner != owner || &copy.binding != binding {
            return Err(failure("provider_account_copy_conflict"));
        }
        match &copy.progress {
            ProviderAccountCopyProgress::WaitingForUnlock { wait } => {
                Ok(Some(ProviderAccountCopyOutcome::Waiting(wait.clone())))
            }
            ProviderAccountCopyProgress::Installed { receipt } => {
                Ok(Some(ProviderAccountCopyOutcome::Installed(receipt.clone())))
            }
            ProviderAccountCopyProgress::Failed { code, .. } => {
                Err(super::provider_account_copy_offer::copy_peer_failure(code))
            }
        }
    }

    pub(super) fn wait_for_account_copy<F, Fut>(
        &self,
        owner: String,
        binding: ProviderAccountCopyBinding,
        wait: ProviderAccountCopyWait,
        mut retry: F,
    ) where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<RelayPeerResponse, DaemonError>> + Send,
    {
        let remaining = wait
            .expires_at_ms
            .saturating_sub(crate::session::unix_epoch_ms());
        let deadline = Timer::now() + Duration::from_millis(remaining).min(COPY_UNLOCK_WAIT);
        {
            let mut copies = self
                .account_copy_waits
                .outgoing
                .lock()
                .expect("copy waits poisoned");
            if copies.contains_key(&binding.operation_id) {
                return;
            }
            copies.insert(
                binding.operation_id.clone(),
                Outgoing {
                    owner,
                    binding: binding.clone(),
                    remove_at: deadline + COPY_UNLOCK_WAIT,
                    progress: ProviderAccountCopyProgress::WaitingForUnlock { wait },
                },
            );
        }
        let state = self.clone();
        tokio::spawn(async move {
            let expected = ProviderAccountCopyReceipt::installed(&binding);
            let settled = loop {
                tokio::time::sleep_until((Timer::now() + COPY_RETRY_INTERVAL).min(deadline)).await;
                if Timer::now() >= deadline {
                    break Err(failure("provider_account_copy_cancelled"));
                }
                match retry().await {
                    Ok(RelayPeerResponse::ProviderAccountCopyTargetVaultLocked { .. }) => continue,
                    Ok(RelayPeerResponse::ProviderAccountCopyInstalled { receipt })
                        if receipt == expected =>
                    {
                        break Ok(receipt)
                    }
                    Ok(RelayPeerResponse::ProviderAccountCopyFailed { code }) => {
                        break Err(super::provider_account_copy::install_peer_failure(&code))
                    }
                    Ok(_) => break Err(failure("provider_account_copy_unsupported")),
                    Err(error) => break Err(error),
                }
            };
            let progress = match settled {
                Ok(receipt) => ProviderAccountCopyProgress::Installed { receipt },
                Err(DaemonError::ProviderAccountCopy { code, .. }) => {
                    ProviderAccountCopyProgress::Failed {
                        operation_id: binding.operation_id.clone(),
                        code: code.into(),
                    }
                }
                Err(_) => ProviderAccountCopyProgress::Failed {
                    operation_id: binding.operation_id.clone(),
                    code: "provider_account_copy_install_failed".into(),
                },
            };
            {
                let mut copies = state
                    .account_copy_waits
                    .outgoing
                    .lock()
                    .expect("copy waits poisoned");
                if let Some(copy) = copies.get_mut(&binding.operation_id) {
                    copy.progress = progress;
                }
            }
            state.notify_account_copy_progress();
        });
    }

    pub(super) fn notify_account_copy_progress(&self) {
        self.kernel_resource_changes.notify();
    }

    /// Called with the Vault's mutation lock to order admission against the owner's unlock.
    pub(super) async fn account_copy_unlock_gate(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        path: &Path,
        locked: bool,
        resume: bool,
    ) -> Result<Option<u64>, DaemonError> {
        let id = format!(
            "account-copy-unlock:{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&binding.source_kernel_id, &binding.operation_id))
                    .map_err(|_| failure("invalid_request"))?,
            )
        );
        {
            let mut incoming = self
                .account_copy_waits
                .incoming
                .lock()
                .expect("copy waits poisoned");
            incoming.retain(|_, entry| entry.deadline + COPY_UNLOCK_WAIT > Timer::now());
            if let Some(entry) = incoming.get(&id) {
                if &entry.binding != binding {
                    return Err(failure("provider_account_copy_conflict"));
                }
                return match entry.status {
                    _ if entry.deadline <= Timer::now() => {
                        Err(failure("provider_account_copy_cancelled"))
                    }
                    IncomingStatus::Cancelled => Err(failure("provider_account_copy_cancelled")),
                    IncomingStatus::Unlocked if !locked => Ok(None),
                    IncomingStatus::Unlocked => Err(failure("provider_account_copy_vault_locked")),
                    IncomingStatus::Waiting => Ok(Some(entry.expires_at_ms)),
                };
            }
        }
        if resume {
            return Err(failure("provider_account_copy_cancelled"));
        }
        if !locked {
            return Ok(None);
        }
        let deadline = Timer::now() + COPY_UNLOCK_WAIT;
        let expires_at_ms = crate::session::unix_epoch_ms() + COPY_UNLOCK_WAIT.as_millis() as u64;
        let mut unlocks = super::vault_unlock_requests::vault_unlock_notifications(path);
        let answer = self
            .create_kernel_operation_interaction(
                "",
                owner,
                RuntimeInteraction::for_kernel_operation(
                    &id,
                    &id,
                    "Vault unlock requested",
                    "Unlock this kernel’s Vault to receive the copied login.",
                    vec![RuntimeInteractionChoice::new(
                        "cancel", "Cancel", "cancel", None,
                    )],
                )
                .with_timeout_sec(COPY_UNLOCK_WAIT.as_secs()),
            )
            .await?;
        self.account_copy_waits
            .incoming
            .lock()
            .expect("copy waits poisoned")
            .insert(
                id.clone(),
                Incoming {
                    owner: self.provider_account_authority_owner_user_id(owner),
                    binding: binding.clone(),
                    deadline,
                    expires_at_ms,
                    status: IncomingStatus::Waiting,
                },
            );
        let state = self.clone();
        let path = path.to_owned();
        tokio::spawn(async move {
            let mut answer = answer;
            let status = loop {
                tokio::select! {
                    biased;
                    _ = &mut answer => break IncomingStatus::Cancelled,
                    _ = tokio::time::sleep_until(deadline) => break IncomingStatus::Cancelled,
                    Ok(()) = unlocks.changed() => {
                        if crate::secret::chariox_encrypted_vault_status(&path).is_ok_and(|status| status.unlocked) {
                            break IncomingStatus::Unlocked;
                        }
                    }
                }
            };
            if let Some(entry) = state
                .account_copy_waits
                .incoming
                .lock()
                .expect("copy waits poisoned")
                .get_mut(&id)
            {
                entry.status = status;
            }
            let _ = state.owned.timeout_runtime_interaction("", &id);
            state.notify_account_copy_progress();
        });
        self.notify_account_copy_progress();
        Ok(Some(expires_at_ms))
    }
}
