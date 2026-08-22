use std::time::Duration;

use chariox_relay::protocol::RelayCallerIdentity;

use crate::error::DaemonError;
use crate::managed_context::cloud_completion::{
    complete_managed_context_import, context_manifest_digest,
    validate_managed_context_completion_binding,
};
use crate::managed_context::package::{
    apply_managed_context_package, rollback_managed_context_package_application,
    rollback_persisted_managed_context_publication, ManagedContextGitCredentialImportTarget,
    ManagedContextImportedKernelContext, ManagedContextPackageApplicationRequest,
    ManagedContextPackageBinding, ManagedContextPackageImportReceipt,
    ManagedContextPackageImportRequest, ManagedContextProviderAccountImportTarget,
};
use crate::managed_context::transfer::{
    ArmManagedContextTransfer, ManagedContextImportClaim, ManagedContextTransferCaller,
    ManagedContextTransferPhase, ManagedContextTransferStatus, ManagedContextTransferStore,
    MAX_TRANSFER_CHUNK_BYTES,
};
use crate::runtime::terminal_pairings::public_key_thumbprint;
use crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE;
use crate::transport::relay_peer::{
    RelayManagedContextCapability, RelayManagedContextImportReceipt,
    RelayManagedContextImportedRepository, RelayManagedContextTransferPhase,
    RelayManagedContextTransferStatus, RelayManagedDevelopmentContextImportReceipt,
    RelayManagedKernelContextImportReceipt, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

use super::CommandRouter;

const TRANSFER_TTL: Duration = Duration::from_secs(30 * 60);

struct AuthorizedManagedContextCaller {
    caller: ManagedContextTransferCaller,
    plan: crate::managed_context::package::ManagedContextPlanBinding,
}

impl CommandRouter {
    pub(crate) async fn relay_arm_managed_context_import(
        &self,
        identity: RelayCallerIdentity,
        context_id: String,
        plan_digest: String,
        target_environment_id: String,
        target_kernel_id: String,
        target_key_thumbprint: String,
        capability: String,
        archive_sha256: String,
        archive_size_bytes: u64,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let authorization = managed_context_caller(self, &identity)?;
        if authorization.plan.context_id != context_id
            || authorization.plan.plan_digest != plan_digest
        {
            return Err(managed_context_authorization_error(
                "managed context selection does not match the Cloud launch plan",
            ));
        }
        let caller = authorization.caller;
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
                    plan: authorization.plan,
                    target_environment_id,
                    target_kernel_id,
                    target_key_thumbprint,
                    source_kernel_id: caller.kernel_id,
                    source_key_thumbprint: caller.key_thumbprint,
                    owner_user_id: caller.owner_user_id,
                    realm_id: caller.realm_id,
                    capability,
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
        let caller = managed_context_caller(self, &identity)?.caller;
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
        let caller = managed_context_caller(self, &identity)?.caller;
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
        let caller = managed_context_caller(self, &identity)?.caller;
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
        let authorization = managed_context_caller(self, &identity)?;
        let caller = authorization.caller;
        let completion_plan = authorization.plan;
        let registration = self.managed_kernel_registration.clone().ok_or_else(|| {
            managed_context_authorization_error(
                "context completion requires a confirmed Chariox-managed kernel",
            )
        })?;
        let completion_config = self.config_projection.snapshot();
        let provider_account_target = ManagedContextProviderAccountImportTarget {
            registry: self.provider_account_profiles.clone(),
            owner_user_id: crate::account_profile::provider_account_authority_owner_user_id(
                &completion_config,
                &caller.owner_user_id,
            ),
        };
        let git_credential_target = if matches!(
            completion_plan.git_credentials,
            crate::managed_context::package::ManagedContextGitCredentialSelection::None
        ) {
            None
        } else {
            let git_credential_home = std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
                .ok_or_else(|| {
                    managed_context_authorization_error(
                        "target kernel HOME is unavailable for Git credential transfer",
                    )
                })?;
            Some(ManagedContextGitCredentialImportTarget {
                command_context:
                    crate::managed_context::scm::GitCredentialCommandContext::managed_target(
                        git_credential_home,
                    )?,
            })
        };
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

        let target_private_key = self.relay_private_key();
        let early_terminal_error = if ready.plan != completion_plan {
            Some(managed_context_authorization_error(
                "managed context transfer no longer matches the Cloud launch plan",
            ))
        } else {
            validate_managed_context_completion_binding(
                &completion_config,
                &registration,
                &completion_plan,
            )
            .err()
        };
        if let Some(error) = early_terminal_error {
            let rollback_request = ManagedContextPackageImportRequest {
                package_path: ready.archive_path.clone(),
                expected_package_sha256: ready.archive_sha256.clone(),
                expected_binding: ManagedContextPackageBinding {
                    plan: ready.plan.clone(),
                    target_environment_id: ready.target_environment_id.clone(),
                    source_kernel_id: ready.source_kernel_id.clone(),
                    source_key_thumbprint: ready.source_key_thumbprint.clone(),
                    target_kernel_id: ready.target_kernel_id.clone(),
                    target_key_thumbprint: ready.target_key_thumbprint.clone(),
                },
            };
            let rollback_private_key = target_private_key.clone();
            let rollback_provider_account_target = provider_account_target.clone();
            let rollback_git_credential_target = git_credential_target.clone();
            if let Err(rollback_error) = run_import_blocking(move || {
                rollback_persisted_managed_context_publication(
                    rollback_request,
                    &rollback_private_key,
                    Some(&rollback_provider_account_target),
                    rollback_git_credential_target.as_ref(),
                )
            })
            .await
            {
                let release_store = store.clone();
                let release_transfer_id = transfer_id.clone();
                let _ =
                    run_blocking(move || release_store.release_import(&release_transfer_id)).await;
                return Err(managed_context_unavailable(format!(
                    "roll back recovered managed context publication: {rollback_error}"
                )));
            }
            return Err(retire_terminal_import(store.clone(), transfer_id.clone(), error).await);
        }
        let rollback_private_key = target_private_key.clone();
        let import_provider_account_target = provider_account_target.clone();
        let import_git_credential_target = git_credential_target.clone();
        let imported = run_import_blocking(move || {
            apply_managed_context_package(ManagedContextPackageApplicationRequest {
                transfer_id: ready.transfer_id,
                package_path: ready.archive_path,
                expected_package_sha256: ready.archive_sha256,
                expected_binding: ManagedContextPackageBinding {
                    plan: ready.plan,
                    target_environment_id: ready.target_environment_id,
                    source_kernel_id: ready.source_kernel_id,
                    source_key_thumbprint: ready.source_key_thumbprint,
                    target_kernel_id: ready.target_kernel_id,
                    target_key_thumbprint: ready.target_key_thumbprint,
                },
                development_destination_root: ready.destination_root,
                target_private_key,
                provider_account_target: Some(import_provider_account_target),
                git_credential_target: import_git_credential_target,
            })
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
        let manifest_digest = match context_manifest_digest(&receipt_json) {
            Ok(digest) => digest,
            Err(error) => {
                let release_store = store.clone();
                let release_transfer_id = transfer_id.clone();
                run_blocking(move || release_store.release_import(&release_transfer_id)).await?;
                return Err(error);
            }
        };
        if let Err(completion_error) = complete_managed_context_import(
            &completion_config,
            &registration,
            &completion_plan,
            &manifest_digest,
        )
        .await
        {
            let (_, retryable) = managed_context_failure_policy(&completion_error);
            let failure_store = store.clone();
            let failure_transfer_id = transfer_id.clone();
            if retryable {
                let _ =
                    run_blocking(move || failure_store.release_import(&failure_transfer_id)).await;
            } else {
                let rollback_receipt = receipt.clone();
                let rollback_provider_account_target = provider_account_target.clone();
                let rollback_git_credential_target = git_credential_target.clone();
                if let Err(rollback_error) = run_import_blocking(move || {
                    rollback_managed_context_package_application(
                        &rollback_receipt,
                        &rollback_private_key,
                        Some(&rollback_provider_account_target),
                        rollback_git_credential_target.as_ref(),
                    )
                })
                .await
                {
                    let _ =
                        run_blocking(move || failure_store.release_import(&failure_transfer_id))
                            .await;
                    return Err(managed_context_unavailable(format!(
                        "roll back rejected managed context import: {rollback_error}"
                    )));
                }
                return Err(retire_terminal_import(
                    store.clone(),
                    transfer_id.clone(),
                    completion_error,
                )
                .await);
            }
            return Err(completion_error);
        }
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
) -> Result<AuthorizedManagedContextCaller, DaemonError> {
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
    let context_plan = registration.context_plan.as_ref().ok_or_else(|| {
        managed_context_authorization_error(
            "managed kernel registration has no Cloud context authorization",
        )
    })?;
    let source = context_plan.source_binding().ok_or_else(|| {
        managed_context_authorization_error(
            "managed kernel launch plan has no source context selection",
        )
    })?;
    let config = router.config_projection.snapshot();
    let profile = config.cloud_relay.as_ref().ok_or_else(|| {
        managed_context_authorization_error("managed context target has no Cloud relay profile")
    })?;
    let target_key_thumbprint = public_key_thumbprint(&config.relay_public_key);
    if registration.kernel_id != config.daemon_id
        || registration.machine_id != config.host_machine_id
        || source.kernel_id != identity.subject
        || source.key_thumbprint != key_thumbprint
        || source.relay_realm_id != identity.realm_id
        || profile.realm_id != identity.realm_id
        || profile.user_id != owner_user_id
    {
        return Err(managed_context_authorization_error(
            "managed context target or owner binding does not match",
        ));
    }
    Ok(AuthorizedManagedContextCaller {
        caller: ManagedContextTransferCaller {
            kernel_id: identity.subject.clone(),
            key_thumbprint,
            owner_user_id,
            realm_id: identity.realm_id.clone(),
            target_environment_id: registration.environment_id.clone(),
            target_kernel_id: registration.kernel_id.clone(),
            target_key_thumbprint,
        },
        plan: context_plan.package_binding(),
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
            let receipt = serde_json::from_str::<ManagedContextPackageImportReceipt>(json)
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

async fn retire_terminal_import(
    store: ManagedContextTransferStore,
    transfer_id: String,
    original_error: DaemonError,
) -> DaemonError {
    let (failure_code, _) = managed_context_failure_policy(&original_error);
    match run_blocking(move || {
        store.retire_import(&transfer_id, failure_code, crate::session::unix_epoch_ms())
    })
    .await
    {
        Ok(()) => original_error,
        Err(error) => managed_context_unavailable(format!(
            "persist terminal managed context failure before replying: {error}"
        )),
    }
}

fn relay_receipt(
    receipt: ManagedContextPackageImportReceipt,
    receipt_sha256: &str,
) -> Result<RelayManagedContextImportReceipt, DaemonError> {
    let development = match receipt.development {
        crate::managed_context::package::ManagedContextImportedDevelopment::Empty => {
            RelayManagedDevelopmentContextImportReceipt::Empty
        }
        crate::managed_context::package::ManagedContextImportedDevelopment::FromSource {
            project_id,
            receipt,
        } => {
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
            RelayManagedDevelopmentContextImportReceipt::FromSource {
                project_id,
                destination_root,
                primary_repository_id: receipt.primary_repository_id,
                repositories,
            }
        }
    };
    Ok(RelayManagedContextImportReceipt {
        transfer_id: receipt.transfer_id,
        archive_sha256: receipt.package_sha256,
        plan_digest: receipt.plan_digest,
        development,
        kernel_context: match receipt.kernel_context {
            ManagedContextImportedKernelContext::Empty => {
                RelayManagedKernelContextImportReceipt::Empty
            }
            ManagedContextImportedKernelContext::FromKernel { receipt } => {
                RelayManagedKernelContextImportReceipt::FromKernel {
                    context_id: receipt.context_id,
                    source_kernel_id: receipt.source_kernel_id,
                    source_key_thumbprint: receipt.source_key_thumbprint,
                    snapshot_sha256: receipt.snapshot_sha256,
                    extension_count: receipt.extension_count,
                    dependency_count: receipt.dependency_count,
                }
            }
        },
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

async fn run_import_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, DaemonError> + Send + 'static,
) -> Result<T, DaemonError> {
    run_import_blocking_with_spawner(operation, |task| {
        std::thread::Builder::new()
            .name("chariox-context-import".to_string())
            .stack_size(KERNEL_RUNTIME_THREAD_STACK_SIZE)
            .spawn(task)
            .map(|_| ())
    })
    .await
}

type ImportThreadTask = Box<dyn FnOnce() + Send + 'static>;

async fn run_import_blocking_with_spawner<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, DaemonError> + Send + 'static,
    spawn: impl FnOnce(ImportThreadTask) -> std::io::Result<()>,
) -> Result<T, DaemonError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    spawn(Box::new(move || {
        let _ = sender.send(operation());
    }))
    .map_err(|error| {
        managed_context_unavailable(format!("start managed context import task: {error}"))
    })?;
    receiver.await.map_err(|_| {
        managed_context_unavailable("managed context import task exited without a result")
    })?
}

fn managed_context_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_import_failed",
        operation: "managed context relay import",
        message: message.into(),
        retryable: false,
    }
}

fn managed_context_unavailable(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_import_unavailable",
        operation: "managed context relay import",
        message: message.into(),
        retryable: true,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn import_thread_creation_failure_is_retryable() {
        let error = run_import_blocking_with_spawner(
            || Ok::<_, DaemonError>(()),
            |_task| Err(std::io::Error::other("injected thread capacity failure")),
        )
        .await
        .expect_err("thread allocation failure must remain retryable");
        assert!(matches!(
            error,
            DaemonError::ManagedContext {
                code: "managed_context_import_unavailable",
                retryable: true,
                ..
            }
        ));
    }
}
