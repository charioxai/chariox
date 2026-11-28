//! Short-lived state for an explicit account copy waiting for the receiving Vault.
use super::*;
use crate::provider_account_copy::{
    failure, ProviderAccountCopyBinding, ProviderAccountCopyOutcome, ProviderAccountCopyProgress,
    ProviderAccountCopyReceipt, ProviderAccountCopyUnlockRequest, ProviderAccountCopyWait,
};
use sha2::{Digest, Sha256};
use std::future::Future;
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
    claim: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<bool>>,
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

fn copy_unlock_id(binding: &ProviderAccountCopyBinding) -> Result<String, DaemonError> {
    Ok(format!(
        "account-copy-unlock:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&binding.source_kernel_id, &binding.operation_id))
                .map_err(|_| failure("invalid_request"))?,
        )
    ))
}

fn retryable_failure(code: &str) -> bool {
    matches!(
        super::provider_account_copy_offer::copy_peer_failure(code),
        DaemonError::ProviderAccountCopy {
            retryable: true,
            ..
        }
    )
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
        let Some(copy) = copies.get_mut(&binding.operation_id) else {
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
                if !retryable_failure(code) {
                    return Err(super::provider_account_copy_offer::copy_peer_failure(code));
                }
                if Timer::now() >= copy.remove_at - COPY_UNLOCK_WAIT {
                    copy.progress = ProviderAccountCopyProgress::Failed {
                        operation_id: binding.operation_id.clone(),
                        code: "provider_account_copy_cancelled".into(),
                    };
                    drop(copies);
                    self.notify_account_copy_progress();
                    return Err(failure("provider_account_copy_cancelled"));
                }
                // Keep the binding reserved while an explicit retry contacts the peer.
                Ok(None)
            }
        }
    }

    pub(super) fn remember_account_copy_retry(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        result: &Result<ProviderAccountCopyOutcome, DaemonError>,
    ) {
        let progress = match result {
            Ok(ProviderAccountCopyOutcome::Installed(receipt)) => {
                ProviderAccountCopyProgress::Installed {
                    receipt: receipt.clone(),
                }
            }
            Err(DaemonError::ProviderAccountCopy { code, .. }) => {
                ProviderAccountCopyProgress::Failed {
                    operation_id: binding.operation_id.clone(),
                    code: (*code).into(),
                }
            }
            _ => return,
        };
        {
            let mut copies = self
                .account_copy_waits
                .outgoing
                .lock()
                .expect("copy waits poisoned");
            let Some(copy) = copies.get_mut(&binding.operation_id) else {
                return;
            };
            if copy.owner != owner || &copy.binding != binding {
                return;
            }
            if !matches!(&copy.progress, ProviderAccountCopyProgress::Failed { code, .. } if retryable_failure(code))
            {
                return;
            }
            copy.progress = progress;
        }
        self.notify_account_copy_progress();
    }

    pub(super) fn wait_for_account_copy<F, Fut>(
        &self,
        owner: String,
        binding: ProviderAccountCopyBinding,
        wait: ProviderAccountCopyWait,
        mut retry: F,
    ) -> Result<ProviderAccountCopyOutcome, DaemonError>
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<RelayPeerResponse, DaemonError>> + Send,
    {
        let remaining = wait
            .expires_at_ms
            .saturating_sub(crate::session::unix_epoch_ms());
        let mut deadline = Timer::now() + Duration::from_millis(remaining).min(COPY_UNLOCK_WAIT);
        {
            let mut copies = self
                .account_copy_waits
                .outgoing
                .lock()
                .expect("copy waits poisoned");
            if let Some(copy) = copies.get_mut(&binding.operation_id) {
                if copy.owner != owner || copy.binding != binding {
                    return Err(failure("provider_account_copy_conflict"));
                }
                match &copy.progress {
                    ProviderAccountCopyProgress::WaitingForUnlock { wait } => {
                        return Ok(ProviderAccountCopyOutcome::Waiting(wait.clone()));
                    }
                    ProviderAccountCopyProgress::Installed { receipt } => {
                        return Ok(ProviderAccountCopyOutcome::Installed(receipt.clone()));
                    }
                    ProviderAccountCopyProgress::Failed { code, .. }
                        if !retryable_failure(code) =>
                    {
                        return Err(super::provider_account_copy_offer::copy_peer_failure(code));
                    }
                    ProviderAccountCopyProgress::Failed { .. } => {}
                }
                deadline = deadline.min(copy.remove_at - COPY_UNLOCK_WAIT);
                if Timer::now() >= deadline {
                    return Err(failure("provider_account_copy_cancelled"));
                }
                copy.progress =
                    ProviderAccountCopyProgress::WaitingForUnlock { wait: wait.clone() };
            } else {
                copies.insert(
                    binding.operation_id.clone(),
                    Outgoing {
                        owner,
                        binding: binding.clone(),
                        remove_at: deadline + COPY_UNLOCK_WAIT,
                        progress: ProviderAccountCopyProgress::WaitingForUnlock {
                            wait: wait.clone(),
                        },
                    },
                );
            }
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
        Ok(ProviderAccountCopyOutcome::Waiting(wait))
    }

    pub(super) fn notify_account_copy_progress(&self) {
        self.kernel_resource_changes.notify();
    }

    /// Claim only this operation's card after its normal Vault lease was acquired.
    /// The card task orders an already answered Cancel or expired wait before the claim.
    pub(super) async fn claim_account_copy_unlock(
        &self,
        binding: &ProviderAccountCopyBinding,
        resume: bool,
    ) -> Result<(), DaemonError> {
        let id = copy_unlock_id(binding)?;
        let claim = {
            let incoming = self
                .account_copy_waits
                .incoming
                .lock()
                .expect("copy waits poisoned");
            let Some(entry) = incoming.get(&id) else {
                return if resume {
                    Err(failure("provider_account_copy_cancelled"))
                } else {
                    Ok(())
                };
            };
            if &entry.binding != binding {
                return Err(failure("provider_account_copy_conflict"));
            }
            match entry.status {
                IncomingStatus::Cancelled => {
                    return Err(failure("provider_account_copy_cancelled"))
                }
                IncomingStatus::Unlocked => return Ok(()),
                IncomingStatus::Waiting => entry.claim.clone(),
            }
        };
        let (accepted, answer) = tokio::sync::oneshot::channel();
        claim
            .send(accepted)
            .map_err(|_| failure("provider_account_copy_cancelled"))?;
        match answer.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(failure("provider_account_copy_install_failed")),
            Err(_) => Err(failure("provider_account_copy_cancelled")),
        }
    }

    /// Called with the Vault's mutation lock to order admission against the owner's unlock.
    pub(super) async fn account_copy_unlock_gate(
        &self,
        owner: &str,
        binding: &ProviderAccountCopyBinding,
        locked: bool,
        resume: bool,
    ) -> Result<Option<u64>, DaemonError> {
        let id = copy_unlock_id(binding)?;
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
                    IncomingStatus::Waiting if !locked => Ok(None),
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
        let (claim, mut claims) = tokio::sync::mpsc::unbounded_channel();
        let super::runtime_interaction_owned_state::InteractionRequest(kind, interaction) =
            super::runtime_interaction_owned_state::InteractionRequest::account_copy_unlock(
                &id,
                COPY_UNLOCK_WAIT.as_secs(),
            );
        let answer = self
            .create_kernel_operation_interaction(kind, "", owner, interaction)
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
                    claim,
                },
            );
        let state = self.clone();
        tokio::spawn(async move {
            let mut answer = answer;
            let acquired: Option<tokio::sync::oneshot::Sender<bool>> = tokio::select! {
                biased;
                _ = &mut answer => None,
                _ = tokio::time::sleep_until(deadline) => None,
                claim = claims.recv() => claim,
            };
            let status = if acquired.is_some() {
                IncomingStatus::Unlocked
            } else {
                IncomingStatus::Cancelled
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
            let retired = state.owned.timeout_runtime_interaction("", &id).is_ok();
            if !retired {
                if let Some(entry) = state
                    .account_copy_waits
                    .incoming
                    .lock()
                    .expect("copy waits poisoned")
                    .get_mut(&id)
                {
                    entry.status = IncomingStatus::Cancelled;
                }
            }
            state.notify_account_copy_progress();
            if let Some(accepted) = acquired {
                let _ = accepted.send(retired);
            }
        });
        self.notify_account_copy_progress();
        Ok(Some(expires_at_ms))
    }
}
