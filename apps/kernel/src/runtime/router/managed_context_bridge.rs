use std::time::Duration;

use chariox_relay::protocol::RelayCallerIdentity;

use crate::error::DaemonError;
use crate::managed_context::development::{
    import_development_context_with_publication, recover_development_context_publication,
    DevelopmentContextImportRequest, DevelopmentContextPublicationReceipt,
};
use crate::managed_context::transfer::{
    ArmManagedContextTransfer, ManagedContextImportClaim, ManagedContextTransferCaller,
    ManagedContextTransferPhase, ManagedContextTransferStatus, MAX_TRANSFER_CHUNK_BYTES,
};
use crate::runtime::terminal_pairings::public_key_thumbprint;
use crate::transport::relay_peer::{
    RelayManagedContextCapability, RelayManagedContextImportReceipt,
    RelayManagedContextImportedRepository, RelayManagedContextTransferPhase,
    RelayManagedContextTransferStatus, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::CommandRouter;

const TRANSFER_TTL: Duration = Duration::from_secs(30 * 60);

impl CommandRouter {
    pub(crate) async fn relay_arm_managed_context_import(
        &self,
        identity: RelayCallerIdentity,
        target_environment_id: String,
        target_kernel_id: String,
        target_key_thumbprint: String,
        project_id: String,
        archive_sha256: String,
        archive_size_bytes: u64,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let caller = managed_context_caller(self, &identity)?;
        let config = self.config_projection.snapshot();
        if caller.target_environment_id != target_environment_id
            || caller.target_kernel_id != target_kernel_id
            || caller.target_key_thumbprint != target_key_thumbprint
        {
            return Err(managed_context_authorization_error(
                "managed context target or owner binding does not match",
            ));
        }
        let destination_parent = config
            .durable_state_path()
            .parent()
            .map(|root| root.join("managed-context-workspaces"))
            .ok_or_else(|| managed_context_error("managed context destination has no parent"))?;
        let now_ms = crate::session::unix_epoch_ms();
        let expires_at_ms = now_ms.saturating_add(TRANSFER_TTL.as_millis() as u64);
        let store = self.managed_context_transfers.clone();
        let armed = run_blocking(move || {
            store.arm(
                ArmManagedContextTransfer {
                    target_environment_id,
                    target_kernel_id,
                    target_key_thumbprint,
                    source_kernel_id: caller.kernel_id,
                    source_key_thumbprint: caller.key_thumbprint,
                    owner_user_id: caller.owner_user_id,
                    realm_id: caller.realm_id,
                    project_id,
                    archive_sha256,
                    archive_size_bytes,
                    destination_parent,
                    expires_at_ms,
                },
                now_ms,
            )
        })
        .await?;
        Ok(RelayPeerResponse::ManagedContextImportArmed {
            transfer_id: armed.transfer_id,
            capability: RelayManagedContextCapability::new(armed.capability),
            expires_at_ms: armed.expires_at_ms,
            max_chunk_bytes: MAX_TRANSFER_CHUNK_BYTES,
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        })
    }

    pub(crate) async fn relay_begin_managed_context_import(
        &self,
        identity: RelayCallerIdentity,
        transfer_id: String,
        capability: String,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let caller = managed_context_caller(self, &identity)?;
        let store = self.managed_context_transfers.clone();
        let status = run_blocking(move || {
            store.begin(
                &transfer_id,
                &capability,
                &caller,
                crate::session::unix_epoch_ms(),
            )
        })
        .await?;
        relay_status_response(status)
    }

    pub(crate) async fn relay_upload_managed_context_chunk(
        &self,
        identity: RelayCallerIdentity,
        transfer_id: String,
        capability: String,
        offset: u64,
        bytes: Vec<u8>,
        chunk_sha256: String,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let caller = managed_context_caller(self, &identity)?;
        let store = self.managed_context_transfers.clone();
        let status = run_blocking(move || {
            store.upload_chunk(
                &transfer_id,
                &capability,
                &caller,
                offset,
                &bytes,
                &chunk_sha256,
                crate::session::unix_epoch_ms(),
            )
        })
        .await?;
        relay_status_response(status)
    }

    pub(crate) async fn relay_get_managed_context_import_status(
        &self,
        identity: RelayCallerIdentity,
        transfer_id: String,
        capability: String,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let caller = managed_context_caller(self, &identity)?;
        let store = self.managed_context_transfers.clone();
        let status = run_blocking(move || {
            store.get_status(
                &transfer_id,
                &capability,
                &caller,
                crate::session::unix_epoch_ms(),
            )
        })
        .await?;
        relay_status_response(status)
    }

    pub(crate) async fn relay_finalize_managed_context_import(
        &self,
        identity: RelayCallerIdentity,
        transfer_id: String,
        capability: String,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let caller = managed_context_caller(self, &identity)?;
        let store = self.managed_context_transfers.clone();
        let claim_store = store.clone();
        let claim_transfer_id = transfer_id.clone();
        let claim_capability = capability.clone();
        let claim_caller = caller.clone();
        let ready = run_blocking(move || {
            claim_store.prepare_and_claim_import(
                &claim_transfer_id,
                &claim_capability,
                &claim_caller,
                crate::session::unix_epoch_ms(),
            )
        })
        .await?;
        let ready = match ready {
            ManagedContextImportClaim::Claimed(ready) => ready,
            ManagedContextImportClaim::InProgress(status)
            | ManagedContextImportClaim::Terminal(status) => return relay_status_response(status),
        };

        let import_request = DevelopmentContextImportRequest {
            archive_path: ready.archive_path,
            expected_archive_sha256: ready.archive_sha256,
            expected_project_id: ready.project_id,
            destination_root: ready.destination_root,
        };
        let publication_id = ready.transfer_id;
        let import_request_for_recovery = import_request.clone();
        let publication_for_recovery = publication_id.clone();
        let imported = run_blocking(move || {
            match recover_development_context_publication(
                &import_request_for_recovery,
                &publication_for_recovery,
            )? {
                Some(receipt) => Ok(receipt),
                None => import_development_context_with_publication(
                    import_request_for_recovery,
                    publication_for_recovery,
                ),
            }
        })
        .await;
        let receipt = match imported {
            Ok(receipt) => receipt,
            Err(import_error) => {
                let (failure_code, retryable) = managed_context_failure_policy(&import_error);
                let failure_store = store.clone();
                let failure_transfer_id = transfer_id.clone();
                if retryable {
                    run_blocking(move || failure_store.release_import(&failure_transfer_id))
                        .await?;
                } else {
                    run_blocking(move || {
                        failure_store.retire_import(
                            &failure_transfer_id,
                            failure_code,
                            crate::session::unix_epoch_ms(),
                        )
                    })
                    .await?;
                }
                return Err(import_error);
            }
        };
        let receipt_json = match serde_json::to_string(&receipt) {
            Ok(receipt_json) => receipt_json,
            Err(error) => {
                let release_store = store.clone();
                let release_transfer_id = transfer_id.clone();
                run_blocking(move || release_store.release_import(&release_transfer_id)).await?;
                return Err(managed_context_error(format!(
                    "serialize managed context import receipt: {error}"
                )));
            }
        };
        let commit_store = store.clone();
        let commit_transfer_id = transfer_id.clone();
        if let Err(commit_error) = run_blocking(move || {
            commit_store.commit_import(
                &commit_transfer_id,
                &receipt_json,
                crate::session::unix_epoch_ms(),
            )
        })
        .await
        {
            let release_store = store.clone();
            let release_transfer_id = transfer_id.clone();
            if let Err(release_error) =
                run_blocking(move || release_store.release_import(&release_transfer_id)).await
            {
                return Err(managed_context_error(format!(
                    "{commit_error}; release import claim: {release_error}"
                )));
            }
            return Err(commit_error);
        }

        let final_store = store;
        let final_status = run_blocking(move || {
            final_store.get_status(
                &transfer_id,
                &capability,
                &caller,
                crate::session::unix_epoch_ms(),
            )
        })
        .await?;
        relay_status_response(final_status)
    }
}

fn managed_context_caller(
    router: &CommandRouter,
    identity: &RelayCallerIdentity,
) -> Result<ManagedContextTransferCaller, DaemonError> {
    let owner_user_id = identity.user_id.clone().ok_or_else(|| {
        managed_context_authorization_error(
            "managed context source kernel has no authenticated owner",
        )
    })?;
    let key_thumbprint = identity.public_key_thumbprint.clone().ok_or_else(|| {
        managed_context_authorization_error("managed context source kernel has no bound sender key")
    })?;
    let registration = router.managed_kernel_registration.as_ref().ok_or_else(|| {
        managed_context_authorization_error(
            "context imports require a confirmed Chariox-managed kernel",
        )
    })?;
    let config = router.config_projection.snapshot();
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        managed_context_authorization_error("managed context target has no Cloud relay profile")
    })?;
    let target_key_thumbprint = public_key_thumbprint(&config.relay_public_key);
    if registration.kernel_id != config.daemon_id
        || registration.machine_id != config.host_machine_id
        || profile.realm_id != identity.realm_id
        || profile.user_id != owner_user_id
    {
        return Err(managed_context_authorization_error(
            "managed context target or owner binding does not match",
        ));
    }
    Ok(ManagedContextTransferCaller {
        kernel_id: identity.subject.clone(),
        key_thumbprint,
        owner_user_id,
        realm_id: identity.realm_id.clone(),
        target_environment_id: registration.environment_id.clone(),
        target_kernel_id: registration.kernel_id.clone(),
        target_key_thumbprint,
    })
}

fn relay_status_response(
    status: ManagedContextTransferStatus,
) -> Result<RelayPeerResponse, DaemonError> {
    if let Some(code) = status.failure_code {
        return Ok(RelayPeerResponse::ManagedContextImportFailed {
            code,
            retryable: false,
        });
    }
    let receipt = match (
        status.import_receipt_json.as_deref(),
        status.import_receipt_sha256.as_deref(),
    ) {
        (Some(json), Some(receipt_sha256)) => {
            let receipt = serde_json::from_str::<DevelopmentContextPublicationReceipt>(json)
                .map_err(|_| managed_context_error("stored import receipt is invalid"))?;
            Some(relay_receipt(receipt, receipt_sha256)?)
        }
        (None, None) => None,
        _ => {
            return Err(managed_context_error(
                "stored import receipt fields are inconsistent",
            ))
        }
    };
    Ok(RelayPeerResponse::ManagedContextImportStatus {
        status: RelayManagedContextTransferStatus {
            transfer_id: status.transfer_id,
            phase: match status.phase {
                ManagedContextTransferPhase::Armed => RelayManagedContextTransferPhase::Armed,
                ManagedContextTransferPhase::Receiving => {
                    RelayManagedContextTransferPhase::Receiving
                }
                ManagedContextTransferPhase::ReadyToImport => {
                    RelayManagedContextTransferPhase::ReadyToImport
                }
                ManagedContextTransferPhase::Importing => {
                    RelayManagedContextTransferPhase::Importing
                }
                ManagedContextTransferPhase::Failed => {
                    return Err(managed_context_error(
                        "failed managed context transfer has no failure code",
                    ))
                }
                ManagedContextTransferPhase::Consumed => RelayManagedContextTransferPhase::Consumed,
            },
            accepted_bytes: status.accepted_bytes,
            archive_size_bytes: status.archive_size_bytes,
            expires_at_ms: status.expires_at_ms,
            receipt,
        },
    })
}

fn managed_context_failure_policy(error: &DaemonError) -> (&'static str, bool) {
    match error {
        DaemonError::ManagedContext {
            code, retryable, ..
        } => (code, *retryable),
        _ => ("managed_context_unavailable", true),
    }
}

fn relay_receipt(
    receipt: DevelopmentContextPublicationReceipt,
    receipt_sha256: &str,
) -> Result<RelayManagedContextImportReceipt, DaemonError> {
    let destination_root = utf8_path(&receipt.destination_root)?;
    let repositories = receipt
        .repositories
        .into_iter()
        .map(|repository| {
            Ok(RelayManagedContextImportedRepository {
                repository_id: repository.repository_id,
                role: repository.role,
                target_directory: repository.target_directory,
                destination_path: utf8_path(&repository.destination_path)?,
                head_sha: repository.head_sha,
            })
        })
        .collect::<Result<Vec<_>, DaemonError>>()?;
    Ok(RelayManagedContextImportReceipt {
        transfer_id: receipt.publication_id,
        archive_sha256: receipt.archive_sha256,
        project_id: receipt.project_id,
        destination_root,
        primary_repository_id: receipt.primary_repository_id,
        repositories,
        receipt_sha256: receipt_sha256.to_string(),
    })
}

fn utf8_path(path: &std::path::Path) -> Result<String, DaemonError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| managed_context_error("managed context destination path is not UTF-8"))
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, DaemonError> + Send + 'static,
) -> Result<T, DaemonError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| managed_context_error(format!("managed context task failed: {error}")))?
}

fn managed_context_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_import_failed",
        operation: "managed context relay import",
        message: message.into(),
        retryable: false,
    }
}

fn managed_context_authorization_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "unauthorized",
        operation: "managed context relay import",
        message: message.into(),
        retryable: false,
    }
}
