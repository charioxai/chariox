use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_bootstrap::ManagedKernelContextPlan;
use crate::managed_context::development::{
    export_development_context, DevelopmentContextExportRequest, DevelopmentRepositorySelection,
};
use crate::managed_context::kernel::{export_kernel_context, KernelContextExportRequest};
use crate::managed_context::outbound::{
    random_managed_context_capability, transfer_managed_context_package,
    ManagedContextOutboundTransferRequest, RelayManagedContextPeerTransport,
};
use crate::managed_context::package::{
    export_managed_context_package, ManagedContextDevelopmentSelection,
    ManagedContextGitCredentialSelection, ManagedContextKernelSelection,
    ManagedContextPackageDevelopment, ManagedContextPackageExportRequest,
    ManagedContextPackageGitCredentials, ManagedContextPackageKernel,
    ManagedContextPackageProviderAccounts, ManagedContextProviderAccountSelection,
    MAX_MANAGED_CONTEXT_PACKAGE_BYTES, MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES,
};
use crate::runtime::cloud_api_client::{cloud_error_is_retryable, post_cloud_json};
use crate::runtime::terminal_pairings::public_key_thumbprint;
use crate::runtime::workspace_git_common::same_fs_path;
use crate::runtime::workspace_worktrees::list_workspace_worktrees;
use crate::secret::export_transferred_vault_snapshot;
use crate::transport::relay_client::RelayClientState;
use crate::transport::relay_peer::RelayManagedContextImportReceipt;

const OUTBOUND_ARTIFACT_SCHEMA_VERSION: u32 = 2;
const MAX_OUTBOUND_ARTIFACT_STATE_BYTES: u64 = 128 * 1024;
const MAX_OUTBOUND_OPERATIONS: usize = 256;
const MAX_CONCURRENT_OUTBOUND_TRANSFERS: usize = 2;
const MAX_DURABLE_OUTBOUND_ARTIFACTS: usize = 2;
const MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES: usize = 256;
const MAX_DURABLE_OUTBOUND_BYTES: u64 =
    MAX_DURABLE_OUTBOUND_ARTIFACTS as u64 * MAX_MANAGED_CONTEXT_PACKAGE_BYTES;
const OUTBOUND_ARTIFACT_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextTransferTarget {
    pub relay_realm_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub relay_public_key: String,
    pub key_thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextTransferTicket {
    pub environment_id: String,
    pub context_plan: ManagedKernelContextPlan,
    pub target: ManagedContextTransferTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedContextOutboundOperationPhase {
    Preparing,
    Uploading,
    Importing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedContextOutboundOperationStatus {
    pub context_id: String,
    pub plan_digest: String,
    pub phase: ManagedContextOutboundOperationPhase,
    pub accepted_bytes: u64,
    pub package_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<RelayManagedContextImportReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    pub retryable: bool,
    pub updated_at_ms: u64,
}

#[derive(Clone)]
pub(crate) struct ManagedContextOutboundOperationStore {
    state: Arc<Mutex<BTreeMap<String, ManagedContextOutboundOperationStatus>>>,
    active: Arc<Mutex<BTreeSet<String>>>,
    transfer_slots: Arc<Semaphore>,
    artifact_lock: Arc<Mutex<()>>,
    artifact_parent: Option<Arc<PathBuf>>,
}

impl Default for ManagedContextOutboundOperationStore {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(BTreeMap::new())),
            active: Arc::new(Mutex::new(BTreeSet::new())),
            transfer_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_OUTBOUND_TRANSFERS)),
            artifact_lock: Arc::new(Mutex::new(())),
            artifact_parent: None,
        }
    }
}

impl ManagedContextOutboundOperationStore {
    pub(crate) fn open(artifact_parent: PathBuf) -> Result<Self, DaemonError> {
        create_private_directory(&artifact_parent)?;
        let store = Self {
            artifact_parent: Some(Arc::new(artifact_parent)),
            ..Self::default()
        };
        let guard = store
            .artifact_lock
            .lock()
            .expect("managed-context outbound artifact lock");
        reconcile_outbound_artifacts(
            store
                .artifact_parent
                .as_deref()
                .expect("configured artifact parent"),
            &BTreeSet::new(),
            crate::session::unix_epoch_ms(),
        )?;
        drop(guard);
        Ok(store)
    }

    pub(crate) fn get(&self, context_id: &str) -> Option<ManagedContextOutboundOperationStatus> {
        self.state
            .lock()
            .expect("managed-context outbound operation lock")
            .get(context_id)
            .cloned()
    }

    fn start(
        &self,
        context_id: &str,
        plan_digest: &str,
    ) -> Result<
        (
            ManagedContextOutboundOperationStatus,
            Option<OwnedSemaphorePermit>,
        ),
        DaemonError,
    > {
        let mut state = self
            .state
            .lock()
            .expect("managed-context outbound operation lock");
        if let Some(existing) = state.get(context_id) {
            if existing.plan_digest != plan_digest {
                return Err(outbound_service_error(
                    "managed-context operation ID was reused with a different plan",
                    false,
                ));
            }
            if existing.phase != ManagedContextOutboundOperationPhase::Failed || !existing.retryable
            {
                return Ok((existing.clone(), None));
            }
            if self
                .active
                .lock()
                .expect("managed-context outbound active lock")
                .contains(context_id)
            {
                return Ok((existing.clone(), None));
            }
        }
        let permit = self
            .transfer_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                outbound_service_error(
                    "managed-context transfer concurrency limit is reached",
                    true,
                )
            })?;
        while state.len() >= MAX_OUTBOUND_OPERATIONS {
            let Some(oldest_terminal_id) = state
                .values()
                .filter(|status| {
                    status.phase == ManagedContextOutboundOperationPhase::Completed
                        || (status.phase == ManagedContextOutboundOperationPhase::Failed
                            && !status.retryable)
                })
                .min_by_key(|status| status.updated_at_ms)
                .map(|status| status.context_id.clone())
            else {
                return Err(outbound_service_error(
                    "too many managed-context transfers are active or retryable",
                    true,
                ));
            };
            state.remove(&oldest_terminal_id);
        }
        let status = ManagedContextOutboundOperationStatus {
            context_id: context_id.to_string(),
            plan_digest: plan_digest.to_string(),
            phase: ManagedContextOutboundOperationPhase::Preparing,
            accepted_bytes: 0,
            package_size_bytes: 0,
            receipt: None,
            failure_code: None,
            failure_message: None,
            retryable: false,
            updated_at_ms: crate::session::unix_epoch_ms(),
        };
        state.insert(context_id.to_string(), status.clone());
        self.active
            .lock()
            .expect("managed-context outbound active lock")
            .insert(context_id.to_string());
        Ok((status, Some(permit)))
    }

    fn update(
        &self,
        context_id: &str,
        update: impl FnOnce(&mut ManagedContextOutboundOperationStatus),
    ) {
        let mut state = self
            .state
            .lock()
            .expect("managed-context outbound operation lock");
        if let Some(status) = state.get_mut(context_id) {
            update(status);
            status.updated_at_ms = crate::session::unix_epoch_ms();
        }
    }

    fn finish(&self, context_id: &str) {
        self.active
            .lock()
            .expect("managed-context outbound active lock")
            .remove(context_id);
    }

    fn active_context_ids(&self) -> BTreeSet<String> {
        self.active
            .lock()
            .expect("managed-context outbound active lock")
            .clone()
    }

    fn artifact_parent(&self, config: &DaemonConfig) -> Result<PathBuf, DaemonError> {
        if let Some(parent) = self.artifact_parent.as_deref() {
            return Ok(parent.clone());
        }
        config
            .durable_state_path()
            .parent()
            .map(|root| root.join("managed-context-outbound"))
            .ok_or_else(|| outbound_service_error("durable state path has no parent", false))
    }
}

pub(crate) fn start_managed_context_outbound_operation(
    config: DaemonConfig,
    relay_state: Arc<RwLock<RelayClientState>>,
    store: ManagedContextOutboundOperationStore,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    ticket: ManagedContextTransferTicket,
) -> Result<ManagedContextOutboundOperationStatus, DaemonError> {
    validate_ticket(&config, &ticket)?;
    let plan = ticket.context_plan.package_binding();
    let (status, permit) = store.start(&plan.context_id, &plan.plan_digest)?;
    let Some(permit) = permit else {
        return Ok(status);
    };
    let context_id = plan.context_id.clone();
    let active = ActiveOutboundOperation::new(store.clone(), context_id.clone());
    tokio::spawn(async move {
        let _permit = permit;
        let _active = active;
        let authoritative_ticket = match fetch_authoritative_ticket(&config, &ticket).await {
            Ok(authoritative) if authoritative == ticket => authoritative,
            Ok(_) => {
                let error = outbound_service_error(
                    "caller-supplied managed-context ticket does not match Cloud",
                    false,
                );
                let retirement_error =
                    retire_matching_artifact_after_terminal_preflight(&config, &store, &ticket)
                        .err();
                store.update(&context_id, |status| {
                    fail_terminal_preflight_status(status, &error, retirement_error.as_ref())
                });
                return;
            }
            Err(error) => {
                let retirement_error = if error_is_retryable(&error) {
                    None
                } else {
                    retire_matching_artifact_after_terminal_preflight(&config, &store, &ticket)
                        .err()
                };
                store.update(&context_id, |status| {
                    fail_terminal_preflight_status(status, &error, retirement_error.as_ref())
                });
                return;
            }
        };
        let task_store = store.clone();
        let task_context_id = context_id.clone();
        let task_config = config.clone();
        let task_ticket = authoritative_ticket.clone();
        let task_provider_account_profiles = provider_account_profiles.clone();
        let package_store = task_store.clone();
        let package = tokio::task::spawn_blocking(move || {
            prepare_managed_context_package(
                &task_config,
                &package_store,
                &task_provider_account_profiles,
                &task_ticket,
            )
        })
        .await
        .map_err(|error| {
            outbound_service_error(format!("managed-context export task failed: {error}"), true)
        })
        .and_then(|result| result);
        let prepared = match package {
            Ok(result) => result,
            Err(error) => {
                task_store.update(&task_context_id, |status| fail_status(status, &error));
                return;
            }
        };
        task_store.update(&task_context_id, |status| {
            status.phase = ManagedContextOutboundOperationPhase::Uploading;
            status.package_size_bytes = prepared.package.package_size_bytes;
        });
        let transport = RelayManagedContextPeerTransport::new(
            config,
            relay_state,
            authoritative_ticket.target.kernel_id.clone(),
            authoritative_ticket.target.relay_public_key.clone(),
        );
        let transfer = transfer_managed_context_package(
            &transport,
            ManagedContextOutboundTransferRequest {
                plan,
                target_environment_id: authoritative_ticket.environment_id,
                target_kernel_id: authoritative_ticket.target.kernel_id,
                target_key_thumbprint: authoritative_ticket.target.key_thumbprint,
                package: prepared.package,
                capability: prepared.capability,
            },
            |target_status| {
                task_store.update(&task_context_id, |status| {
                    status.accepted_bytes = target_status.accepted_bytes;
                    status.package_size_bytes = target_status.archive_size_bytes;
                    if matches!(
                        target_status.phase,
                        crate::transport::relay_peer::RelayManagedContextTransferPhase::Importing
                            | crate::transport::relay_peer::RelayManagedContextTransferPhase::ReadyToImport
                    ) {
                        status.phase = ManagedContextOutboundOperationPhase::Importing;
                    }
                });
            },
        )
        .await;
        match transfer {
            Ok(result) => match remove_artifact_root(&prepared.artifact_root) {
                Ok(()) => task_store.update(&task_context_id, |status| {
                    status.phase = ManagedContextOutboundOperationPhase::Completed;
                    status.accepted_bytes = result.package_size_bytes;
                    status.package_size_bytes = result.package_size_bytes;
                    status.receipt = Some(result.receipt);
                    status.failure_code = None;
                    status.failure_message = None;
                    status.retryable = false;
                }),
                Err(error) => task_store.update(&task_context_id, |status| {
                    status.receipt = Some(result.receipt);
                    fail_status(status, &error);
                }),
            },
            Err(error) => {
                if error_is_retryable(&error) {
                    task_store.update(&task_context_id, |status| fail_status(status, &error));
                } else {
                    let terminal_error = retire_artifact_root(&prepared.artifact_root).err();
                    task_store.update(&task_context_id, |status| {
                        fail_status(status, terminal_error.as_ref().unwrap_or(&error));
                    });
                }
            }
        }
    });
    Ok(status)
}

async fn fetch_authoritative_ticket(
    config: &DaemonConfig,
    requested: &ManagedContextTransferTicket,
) -> Result<ManagedContextTransferTicket, DaemonError> {
    let cloud = config.cloud_relay.as_ref().ok_or_else(|| {
        outbound_service_error("source kernel is not connected to Chariox Cloud", true)
    })?;
    let machine_id = cloud.machine_id.as_deref().ok_or_else(|| {
        outbound_service_error("source kernel Cloud Machine identity is unavailable", true)
    })?;
    let machine_credential = cloud.machine_credential.as_deref().ok_or_else(|| {
        outbound_service_error(
            "source kernel Cloud Machine credential is unavailable",
            true,
        )
    })?;
    let body = serde_json::json!({
        "accountId": cloud.account_id,
        "environmentId": requested.environment_id,
        "machineId": machine_id,
        "kernelId": config.daemon_id,
        "relayRealmId": cloud.realm_id,
        "keyThumbprint": public_key_thumbprint(&config.relay_public_key),
        "machineCredential": machine_credential,
    });
    post_cloud_json(
        cloud.api_url.clone(),
        "/v1/managed-kernels/context/ticket",
        body,
    )
    .await
    .map_err(|error| {
        let retryable = cloud_error_is_retryable(&error);
        outbound_service_error(
            format!("Cloud could not authorize the managed-context transfer ticket: {error}"),
            retryable,
        )
    })
}

fn retire_matching_artifact_after_terminal_preflight(
    config: &DaemonConfig,
    store: &ManagedContextOutboundOperationStore,
    ticket: &ManagedContextTransferTicket,
) -> Result<bool, DaemonError> {
    let plan = ticket.context_plan.package_binding();
    let artifact_parent = store.artifact_parent(config)?;
    if !path_entry_exists(&artifact_parent)? {
        return Ok(false);
    }
    validate_artifact_root(&artifact_parent)?;
    let _artifact_guard = store
        .artifact_lock
        .lock()
        .expect("managed-context outbound artifact lock");
    let artifact_root = artifact_parent.join(&plan.context_id);
    if !path_entry_exists(&artifact_root)? {
        return Ok(false);
    }
    validate_artifact_root(&artifact_root)?;
    let state_path = artifact_root.join("state.json");
    if !path_entry_exists(&state_path)? {
        return Ok(false);
    }
    let state_bytes = read_bounded_regular_file(&state_path, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
    let persisted =
        serde_json::from_slice::<PersistedOutboundArtifact>(&state_bytes).map_err(|error| {
            outbound_service_error(
                format!("parse managed-context outbound state before retirement: {error}"),
                true,
            )
        })?;
    if !persisted_artifact_matches_ticket(&persisted, ticket, &plan) {
        return Ok(false);
    }
    retire_artifact_root(&artifact_root)?;
    Ok(true)
}

fn prepare_managed_context_package(
    config: &DaemonConfig,
    store: &ManagedContextOutboundOperationStore,
    provider_account_profiles: &crate::account_profile::ProviderAccountProfileRegistry,
    ticket: &ManagedContextTransferTicket,
) -> Result<PreparedOutboundArtifact, DaemonError> {
    let plan = ticket.context_plan.package_binding();
    let artifact_parent = store.artifact_parent(config)?;
    create_private_directory(&artifact_parent)?;
    let artifact_guard = store
        .artifact_lock
        .lock()
        .expect("managed-context outbound artifact lock");
    let mut active_context_ids = store.active_context_ids();
    active_context_ids.remove(&plan.context_id);
    let inventory = reconcile_outbound_artifacts(
        &artifact_parent,
        &active_context_ids,
        crate::session::unix_epoch_ms(),
    )?;
    let artifact_root = artifact_parent.join(&plan.context_id);
    if path_entry_exists(&artifact_root)? {
        validate_artifact_root(&artifact_root)?;
        let state_path = artifact_root.join("state.json");
        if path_entry_exists(&state_path)? {
            let restored =
                restore_prepared_artifact(ticket, plan.clone(), artifact_root.clone(), &state_path);
            if matches!(
                &restored,
                Err(DaemonError::ManagedContext {
                    code: "managed_context_outbound_artifact_incomplete",
                    ..
                })
            ) {
                remove_artifact_root(&artifact_root)?;
            } else {
                return restored;
            }
        }
        remove_artifact_root(&artifact_root)?;
    }
    if inventory.root_count >= MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES {
        return Err(outbound_service_error(
            "managed-context outbound artifact root quota is reached",
            false,
        ));
    }
    if inventory.artifact_count >= MAX_DURABLE_OUTBOUND_ARTIFACTS {
        return Err(outbound_service_error(
            "managed-context outbound artifact quota is reached; retry or wait for an existing transfer",
            true,
        ));
    }
    create_private_directory(&artifact_root)?;
    drop(artifact_guard);
    let mut cleanup = ArtifactRootCleanup::new(artifact_root.clone());
    let development = match &plan.development {
        ManagedContextDevelopmentSelection::Empty => ManagedContextPackageDevelopment::Empty,
        ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        } => {
            let selections = repositories
                .iter()
                .map(resolve_repository_selection)
                .collect::<Result<Vec<_>, DaemonError>>()?;
            let exported = export_development_context(DevelopmentContextExportRequest {
                project_id: project_id.clone(),
                repositories: selections,
                archive_path: artifact_root.join("development.tar.gz"),
            })?;
            ManagedContextPackageDevelopment::FromSource {
                archive_path: exported.archive_path,
                archive_sha256: exported.archive_sha256,
            }
        }
    };
    let provider_accounts = match &plan.provider_accounts {
        ManagedContextProviderAccountSelection::None => ManagedContextPackageProviderAccounts::None,
        ManagedContextProviderAccountSelection::Selected { accounts } => {
            let cloud_user_id = config
                .cloud_relay
                .as_ref()
                .map(|profile| profile.user_id.as_str())
                .ok_or_else(|| {
                    outbound_service_error(
                        "source kernel has no Cloud owner for provider-account transfer",
                        false,
                    )
                })?;
            let owner_user_id = crate::account_profile::provider_account_authority_owner_user_id(
                config,
                cloud_user_id,
            );
            let mut materializations = Vec::with_capacity(accounts.len());
            let mut serialized_component_bytes = 2u64;
            for account in accounts {
                let materialization = provider_account_profiles
                    .export_managed_context_materialization(
                        &owner_user_id,
                        &account.provider,
                        &account.account_profile,
                    )
                    .map_err(|_| {
                        outbound_service_error(
                            format!(
                                "selected {} provider account `{}` has no transferable credentials",
                                account.provider, account.account_profile
                            ),
                            false,
                        )
                    })?;
                append_bounded_provider_account_materialization(
                    &mut materializations,
                    &mut serialized_component_bytes,
                    materialization,
                )?;
            }
            ManagedContextPackageProviderAccounts::Selected { materializations }
        }
    };
    let git_credentials = match &plan.git_credentials {
        ManagedContextGitCredentialSelection::None => ManagedContextPackageGitCredentials::None,
        selection @ ManagedContextGitCredentialSelection::Selected { .. } => {
            let command_context =
                crate::managed_context::scm::GitCredentialCommandContext::source_from_process()?;
            let materializations = crate::managed_context::scm::export_selected_git_credentials(
                selection,
                &command_context,
            )?;
            ManagedContextPackageGitCredentials::Selected { materializations }
        }
    };
    let source_key_thumbprint = public_key_thumbprint(&config.relay_public_key);
    let kernel_context = match plan.kernel_context {
        ManagedContextKernelSelection::Empty => ManagedContextPackageKernel::Empty,
        ManagedContextKernelSelection::SourceKernel => {
            let vault = export_transferred_vault_snapshot(
                &config.user_config.credential_vault.path,
                &plan.context_id,
                &config.daemon_id,
                &config.relay_private_key,
                &ticket.target.kernel_id,
                &ticket.target.relay_public_key,
            )?;
            ManagedContextPackageKernel::FromKernel(export_kernel_context(
                KernelContextExportRequest {
                    context_id: plan.context_id.clone(),
                    source_kernel_id: config.daemon_id.clone(),
                    source_key_thumbprint: source_key_thumbprint.clone(),
                    target_kernel_id: ticket.target.kernel_id.clone(),
                    target_key_thumbprint: ticket.target.key_thumbprint.clone(),
                    vault,
                },
            )?)
        }
    };
    let development_archive_path = match &development {
        ManagedContextPackageDevelopment::Empty => None,
        ManagedContextPackageDevelopment::FromSource { archive_path, .. } => {
            Some(archive_path.clone())
        }
    };
    let package = export_managed_context_package(ManagedContextPackageExportRequest {
        plan,
        target_environment_id: ticket.environment_id.clone(),
        source_kernel_id: config.daemon_id.clone(),
        source_key_thumbprint,
        target_kernel_id: ticket.target.kernel_id.clone(),
        target_key_thumbprint: ticket.target.key_thumbprint.clone(),
        development,
        kernel_context,
        provider_accounts,
        git_credentials,
        package_path: artifact_root.join("managed-context.pkg"),
    })?;
    if let Some(path) = development_archive_path {
        fs::remove_file(path).map_err(|error| {
            outbound_service_io_error("remove packaged development context archive", error)
        })?;
    }
    let capability = random_managed_context_capability();
    let persisted = PersistedOutboundArtifact {
        schema_version: OUTBOUND_ARTIFACT_SCHEMA_VERSION,
        created_at_ms: crate::session::unix_epoch_ms(),
        environment_id: ticket.environment_id.clone(),
        plan_digest: package.plan.plan_digest.clone(),
        target_kernel_id: ticket.target.kernel_id.clone(),
        target_key_thumbprint: ticket.target.key_thumbprint.clone(),
        package_sha256: package.package_sha256.clone(),
        package_size_bytes: package.package_size_bytes,
        development_archive_sha256: package.development_archive_sha256.clone(),
        kernel_context_snapshot_sha256: package.kernel_context_snapshot_sha256.clone(),
        provider_accounts_sha256: package.provider_accounts_sha256.clone(),
        git_credentials_sha256: package.git_credentials_sha256.clone(),
        capability: capability.clone(),
    };
    let state_bytes = serde_json::to_vec(&persisted).map_err(|error| {
        outbound_service_error(
            format!("serialize managed-context outbound state: {error}"),
            true,
        )
    })?;
    if state_bytes.len() as u64 > MAX_OUTBOUND_ARTIFACT_STATE_BYTES {
        return Err(outbound_service_error(
            "managed-context outbound state exceeds its size limit",
            false,
        ));
    }
    crate::config::write_private_file(&artifact_root.join("state.json"), &state_bytes).map_err(
        |error| outbound_service_io_error("persist managed-context outbound state", error),
    )?;
    cleanup.keep();
    Ok(PreparedOutboundArtifact {
        package,
        capability,
        artifact_root,
    })
}

fn append_bounded_provider_account_materialization(
    materializations: &mut Vec<crate::account_profile::ProviderAccountMaterialization>,
    serialized_component_bytes: &mut u64,
    materialization: crate::account_profile::ProviderAccountMaterialization,
) -> Result<(), DaemonError> {
    let item_bytes = serde_json::to_vec(&materialization)
        .map_err(|error| outbound_service_error(error.to_string(), false))?
        .len() as u64;
    let separator_bytes = u64::from(!materializations.is_empty());
    let next_bytes = serialized_component_bytes
        .checked_add(separator_bytes)
        .and_then(|bytes| bytes.checked_add(item_bytes))
        .ok_or_else(|| {
            outbound_service_error(
                "selected provider accounts exceed the managed-context component limit",
                false,
            )
        })?;
    if next_bytes > MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES {
        return Err(outbound_service_error(
            "selected provider accounts exceed the managed-context component limit",
            false,
        ));
    }
    *serialized_component_bytes = next_bytes;
    materializations.push(materialization);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedOutboundArtifact {
    schema_version: u32,
    created_at_ms: u64,
    environment_id: String,
    plan_digest: String,
    target_kernel_id: String,
    target_key_thumbprint: String,
    package_sha256: String,
    package_size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    development_archive_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kernel_context_snapshot_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_accounts_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_credentials_sha256: Option<String>,
    capability: crate::transport::relay_peer::RelayManagedContextCapability,
}

fn persisted_artifact_matches_ticket(
    persisted: &PersistedOutboundArtifact,
    ticket: &ManagedContextTransferTicket,
    plan: &crate::managed_context::package::ManagedContextPlanBinding,
) -> bool {
    persisted.schema_version == OUTBOUND_ARTIFACT_SCHEMA_VERSION
        && persisted.environment_id == ticket.environment_id
        && persisted.plan_digest == plan.plan_digest
        && persisted.target_kernel_id == ticket.target.kernel_id
        && persisted.target_key_thumbprint == ticket.target.key_thumbprint
}

struct OutboundArtifactInventory {
    root_count: usize,
    artifact_count: usize,
    package_bytes: u64,
}

fn reconcile_outbound_artifacts(
    artifact_parent: &Path,
    active_context_ids: &BTreeSet<String>,
    now_ms: u64,
) -> Result<OutboundArtifactInventory, DaemonError> {
    create_private_directory(artifact_parent)?;
    let entries = fs::read_dir(artifact_parent)
        .map_err(|error| outbound_service_io_error("list outbound artifacts", error))?
        .take(MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES + 1)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| outbound_service_io_error("list outbound artifacts", error))?;
    if entries.len() > MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES {
        return Err(outbound_service_error(
            "managed-context outbound artifact inventory exceeds its scan limit",
            false,
        ));
    }
    let mut inventory = OutboundArtifactInventory {
        root_count: 0,
        artifact_count: 0,
        package_bytes: 0,
    };
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|_| {
            outbound_service_error(
                "managed-context outbound artifact name is not portable UTF-8",
                false,
            )
        })?;
        if !valid_artifact_name(&name) {
            return Err(outbound_service_error(
                "managed-context outbound artifact name is invalid",
                false,
            ));
        }
        let root = entry.path();
        validate_artifact_root(&root)?;
        if active_context_ids.contains(&name) {
            inventory.root_count = inventory.root_count.saturating_add(1);
            inventory.artifact_count = inventory.artifact_count.saturating_add(1);
            inventory.package_bytes = inventory
                .package_bytes
                .saturating_add(MAX_MANAGED_CONTEXT_PACKAGE_BYTES);
            continue;
        }
        let state_path = root.join("state.json");
        if !path_entry_exists(&state_path)? {
            remove_artifact_root(&root)?;
            continue;
        }
        let state_bytes =
            read_bounded_regular_file(&state_path, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
        let persisted = match serde_json::from_slice::<PersistedOutboundArtifact>(&state_bytes) {
            Ok(persisted) => persisted,
            Err(_) => {
                remove_artifact_root(&root)?;
                continue;
            }
        };
        if persisted.schema_version != OUTBOUND_ARTIFACT_SCHEMA_VERSION
            || persisted.created_at_ms == 0
            || persisted.package_size_bytes == 0
            || persisted.package_size_bytes > MAX_MANAGED_CONTEXT_PACKAGE_BYTES
        {
            remove_artifact_root(&root)?;
            continue;
        }
        let retired_marker = root.join("retired");
        if path_entry_exists(&retired_marker)? {
            if retired_marker_age_ms(&retired_marker, now_ms)? >= OUTBOUND_ARTIFACT_RETENTION_MS {
                remove_artifact_root(&root)?;
                continue;
            }
            retire_artifact_root(&root)?;
            validate_settled_artifact_entries(&root)?;
            inventory.root_count = inventory.root_count.saturating_add(1);
            continue;
        }
        if now_ms.saturating_sub(persisted.created_at_ms) >= OUTBOUND_ARTIFACT_RETENTION_MS {
            retire_artifact_root(&root)?;
            validate_settled_artifact_entries(&root)?;
            inventory.root_count = inventory.root_count.saturating_add(1);
            continue;
        }
        let package_path = root.join("managed-context.pkg");
        if !path_entry_exists(&package_path)? {
            retire_artifact_root(&root)?;
            validate_settled_artifact_entries(&root)?;
            inventory.root_count = inventory.root_count.saturating_add(1);
            continue;
        }
        let actual_size = bounded_regular_file_size(
            &package_path,
            MAX_MANAGED_CONTEXT_PACKAGE_BYTES,
            "inspect retained managed-context package",
        )?;
        if actual_size != persisted.package_size_bytes {
            retire_artifact_root(&root)?;
            validate_settled_artifact_entries(&root)?;
            inventory.root_count = inventory.root_count.saturating_add(1);
            continue;
        }
        validate_settled_artifact_entries(&root)?;
        inventory.root_count = inventory.root_count.saturating_add(1);
        inventory.artifact_count = inventory.artifact_count.saturating_add(1);
        inventory.package_bytes = inventory.package_bytes.saturating_add(actual_size);
    }
    if inventory.artifact_count > MAX_DURABLE_OUTBOUND_ARTIFACTS
        || inventory.package_bytes > MAX_DURABLE_OUTBOUND_BYTES
    {
        return Err(outbound_service_error(
            "managed-context outbound artifacts exceed their durable quota",
            false,
        ));
    }
    Ok(inventory)
}

fn valid_artifact_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && (value.as_bytes()[0].is_ascii_lowercase() || value.as_bytes()[0].is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b':' | b'-')
        })
}

fn validate_settled_artifact_entries(root: &Path) -> Result<(), DaemonError> {
    let entries = fs::read_dir(root)
        .map_err(|error| outbound_service_io_error("list retained outbound artifact", error))?
        .take(4)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| outbound_service_io_error("list retained outbound artifact", error))?;
    if entries.len() > 3
        || entries.iter().any(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some("state.json" | "managed-context.pkg" | "retired")
            )
        })
    {
        return Err(outbound_service_error(
            "managed-context outbound artifact contains unexpected files",
            false,
        ));
    }
    Ok(())
}

fn bounded_regular_file_size(
    path: &Path,
    maximum: u64,
    operation: &'static str,
) -> Result<u64, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|error| outbound_service_io_error(operation, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| outbound_service_io_error(operation, error))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(outbound_service_error(
            "managed-context outbound artifact is not a bounded regular file",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(outbound_service_error(
                "managed-context outbound artifact must not be hard-linked",
                false,
            ));
        }
    }
    Ok(metadata.len())
}

fn retired_marker_age_ms(marker: &Path, now_ms: u64) -> Result<u64, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options.open(marker).map_err(|error| {
        outbound_service_io_error("open retired outbound artifact marker", error)
    })?;
    let metadata = file.metadata().map_err(|error| {
        outbound_service_io_error("inspect retired outbound artifact marker", error)
    })?;
    if !metadata.is_file() || metadata.len() > 64 {
        return Err(outbound_service_error(
            "managed-context retired artifact marker is invalid",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(outbound_service_error(
                "managed-context retired artifact marker must not be hard-linked",
                false,
            ));
        }
    }
    let modified_ms = metadata
        .modified()
        .map_err(|error| {
            outbound_service_io_error("read retired outbound artifact marker time", error)
        })?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            outbound_service_error(
                "managed-context retired artifact marker time is invalid",
                false,
            )
        })?
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    Ok(now_ms.saturating_sub(modified_ms))
}

fn retire_artifact_root(root: &Path) -> Result<(), DaemonError> {
    let marker = root.join("retired");
    if path_entry_exists(&marker)? {
        retired_marker_age_ms(&marker, crate::session::unix_epoch_ms())?;
    } else {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&marker)
            .map_err(|error| outbound_service_io_error("retire outbound artifact", error))?;
        file.write_all(b"retired\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| outbound_service_io_error("retire outbound artifact", error))?;
    }
    for path in [
        root.join("managed-context.pkg"),
        root.join("development.tar.gz"),
    ] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(outbound_service_io_error(
                    "remove retired outbound artifact",
                    error,
                ))
            }
        }
    }
    Ok(())
}

struct PreparedOutboundArtifact {
    package: crate::managed_context::package::ManagedContextPackageExportResult,
    capability: crate::transport::relay_peer::RelayManagedContextCapability,
    artifact_root: PathBuf,
}

fn restore_prepared_artifact(
    ticket: &ManagedContextTransferTicket,
    plan: crate::managed_context::package::ManagedContextPlanBinding,
    artifact_root: PathBuf,
    state_path: &Path,
) -> Result<PreparedOutboundArtifact, DaemonError> {
    if path_entry_exists(&artifact_root.join("retired"))? {
        return Err(outbound_service_error(
            "managed-context source artifacts were retired; create a new managed context",
            false,
        ));
    }
    let state_bytes = read_bounded_regular_file(state_path, MAX_OUTBOUND_ARTIFACT_STATE_BYTES)?;
    let persisted =
        serde_json::from_slice::<PersistedOutboundArtifact>(&state_bytes).map_err(|error| {
            outbound_service_error_with_code(
                "managed_context_outbound_artifact_incomplete",
                format!("parse managed-context outbound state: {error}"),
                true,
            )
        })?;
    if !persisted_artifact_matches_ticket(&persisted, ticket, &plan)
        || persisted.created_at_ms == 0
        || persisted.package_size_bytes == 0
        || persisted.package_size_bytes > MAX_MANAGED_CONTEXT_PACKAGE_BYTES
    {
        return Err(outbound_service_error(
            "persisted managed-context outbound state conflicts with the launch ticket",
            false,
        ));
    }
    if crate::session::unix_epoch_ms().saturating_sub(persisted.created_at_ms)
        >= OUTBOUND_ARTIFACT_RETENTION_MS
    {
        retire_artifact_root(&artifact_root)?;
        return Err(outbound_service_error(
            "managed-context source artifacts expired; create a new managed context",
            false,
        ));
    }
    let package_path = artifact_root.join("managed-context.pkg");
    if !path_entry_exists(&package_path)? {
        retire_artifact_root(&artifact_root)?;
        return Err(outbound_service_error(
            "persisted managed-context package is missing; create a new managed context",
            false,
        ));
    }
    Ok(PreparedOutboundArtifact {
        package: crate::managed_context::package::ManagedContextPackageExportResult {
            plan,
            package_path,
            package_sha256: persisted.package_sha256,
            package_size_bytes: persisted.package_size_bytes,
            development_archive_sha256: persisted.development_archive_sha256,
            kernel_context_snapshot_sha256: persisted.kernel_context_snapshot_sha256,
            provider_accounts_sha256: persisted.provider_accounts_sha256,
            git_credentials_sha256: persisted.git_credentials_sha256,
        },
        capability: persisted.capability,
        artifact_root,
    })
}

pub(crate) fn validate_ticket(
    config: &DaemonConfig,
    ticket: &ManagedContextTransferTicket,
) -> Result<(), DaemonError> {
    ticket
        .context_plan
        .validate()
        .map_err(|message| outbound_service_error(message, false))?;
    let source = ticket.context_plan.source_binding().ok_or_else(|| {
        outbound_service_error(
            "managed-context plan does not select this source kernel",
            false,
        )
    })?;
    let cloud = config.cloud_relay.as_ref().ok_or_else(|| {
        outbound_service_error("source kernel is not connected to Chariox Cloud", true)
    })?;
    let source_thumbprint = public_key_thumbprint(&config.relay_public_key);
    let source_machine_id = cloud.machine_id.as_deref().ok_or_else(|| {
        outbound_service_error("source kernel Cloud Machine identity is unavailable", true)
    })?;
    if source.kernel_id != config.daemon_id
        || source.machine_id != source_machine_id
        || source.key_thumbprint != source_thumbprint
        || source.relay_realm_id != cloud.realm_id
        || ticket.target.relay_realm_id != cloud.realm_id
    {
        return Err(outbound_service_error(
            "managed-context ticket does not match the source kernel identity or relay realm",
            false,
        ));
    }
    if ticket.environment_id.trim().is_empty()
        || ticket.target.machine_id.trim().is_empty()
        || ticket.target.kernel_id.trim().is_empty()
        || public_key_thumbprint(&ticket.target.relay_public_key) != ticket.target.key_thumbprint
    {
        return Err(outbound_service_error(
            "managed-context target identity is invalid",
            false,
        ));
    }
    Ok(())
}

pub(crate) fn resolve_repository_selection(
    binding: &crate::managed_context::development::DevelopmentSourceRepositoryBinding,
) -> Result<DevelopmentRepositorySelection, DaemonError> {
    let worktree_path = match binding.worktree_id.as_deref() {
        None => PathBuf::from(&binding.workspace_id),
        Some(worktree_id) => {
            let worktrees = list_workspace_worktrees(&binding.workspace_id, Some(worktree_id))?;
            let registered = worktrees
                .into_iter()
                .find(|worktree| same_fs_path(&worktree.path, worktree_id))
                .ok_or_else(|| {
                    outbound_service_error(
                        format!(
                            "worktree `{worktree_id}` is not registered for Workspace `{}`",
                            binding.workspace_id
                        ),
                        false,
                    )
                })?;
            PathBuf::from(registered.path)
        }
    };
    Ok(DevelopmentRepositorySelection {
        workspace_id: binding.workspace_id.clone(),
        worktree_id: binding.worktree_id.clone(),
        worktree_path,
        role: binding.role,
    })
}

fn fail_status(status: &mut ManagedContextOutboundOperationStatus, error: &DaemonError) {
    status.phase = ManagedContextOutboundOperationPhase::Failed;
    status.failure_message = Some(error.to_string());
    match error {
        DaemonError::ManagedContext {
            code, retryable, ..
        } => {
            status.failure_code = Some((*code).to_string());
            status.retryable = *retryable;
        }
        _ => {
            status.failure_code = Some("managed_context_transfer_failed".to_string());
            status.retryable = true;
        }
    }
}

fn fail_terminal_preflight_status(
    status: &mut ManagedContextOutboundOperationStatus,
    terminal_error: &DaemonError,
    retirement_error: Option<&DaemonError>,
) {
    fail_status(status, terminal_error);
    if retirement_error.is_some() {
        status.failure_message = Some(format!(
            "{}; retained artifact cleanup did not complete",
            status
                .failure_message
                .as_deref()
                .unwrap_or("managed-context transfer authorization failed")
        ));
    }
}

fn error_is_retryable(error: &DaemonError) -> bool {
    match error {
        DaemonError::ManagedContext { retryable, .. } => *retryable,
        _ => true,
    }
}

fn create_private_directory(path: &Path) -> Result<(), DaemonError> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
                    outbound_service_io_error("secure outbound artifact directory", error)
                })?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(outbound_service_io_error(
                "create outbound artifact directory",
                error,
            ));
        }
    }
    validate_artifact_root(path)
}

fn path_entry_exists(path: &Path) -> Result<bool, DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(outbound_service_io_error(
            "inspect outbound artifact path",
            error,
        )),
    }
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}

fn validate_artifact_root(path: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| outbound_service_io_error("inspect outbound artifact root", error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(outbound_service_error(
            "managed-context outbound artifact root is not a private directory",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(outbound_service_error(
                "managed-context outbound artifact root ownership or mode is invalid",
                false,
            ));
        }
    }
    Ok(())
}

fn read_bounded_regular_file(path: &Path, maximum: u64) -> Result<Vec<u8>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options
        .open(path)
        .map_err(|error| outbound_service_io_error("open outbound state", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| outbound_service_io_error("inspect outbound state", error))?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(outbound_service_error(
            "managed-context outbound state is not a bounded regular file",
            false,
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(outbound_service_error(
                "managed-context outbound state must not be hard-linked",
                false,
            ));
        }
    }
    use std::io::Read;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| outbound_service_io_error("read outbound state", error))?;
    if bytes.len() as u64 > maximum {
        return Err(outbound_service_error(
            "managed-context outbound state exceeds its size limit",
            false,
        ));
    }
    Ok(bytes)
}

fn remove_artifact_root(path: &Path) -> Result<(), DaemonError> {
    if path_entry_exists(path)? {
        validate_artifact_root(path)?;
    }
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(outbound_service_io_error(
            "remove outbound artifacts",
            error,
        )),
    }
}

struct ArtifactRootCleanup {
    path: PathBuf,
    keep: bool,
}

struct ActiveOutboundOperation {
    store: ManagedContextOutboundOperationStore,
    context_id: String,
}

impl ActiveOutboundOperation {
    fn new(store: ManagedContextOutboundOperationStore, context_id: String) -> Self {
        Self { store, context_id }
    }
}

impl Drop for ActiveOutboundOperation {
    fn drop(&mut self) {
        self.store.finish(&self.context_id);
    }
}

impl ArtifactRootCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for ArtifactRootCleanup {
    fn drop(&mut self) {
        if !self.keep {
            let _ = remove_artifact_root(&self.path);
        }
    }
}

fn outbound_service_error(message: impl Into<String>, retryable: bool) -> DaemonError {
    outbound_service_error_with_code("managed_context_source_unavailable", message, retryable)
}

fn outbound_service_error_with_code(
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> DaemonError {
    DaemonError::ManagedContext {
        code,
        operation: "prepare managed context",
        message: message.into(),
        retryable,
    }
}

fn outbound_service_io_error(operation: &'static str, error: std::io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_source_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_profile::{
        ProviderAccountMaterialization, ProviderAccountMaterializationFile,
        ProviderAccountProfileOrigin, ProviderAccountReplicaMetadata,
    };
    use crate::config::PersistedCloudRelayProfile;
    use crate::transport::relay_crypto;

    #[test]
    fn provider_account_component_budget_rejects_before_retaining_the_next_account() {
        fn materialization(profile_id: &str) -> ProviderAccountMaterialization {
            ProviderAccountMaterialization {
                profile: ProviderAccountReplicaMetadata {
                    owner_user_id: "owner-a".to_string(),
                    provider: "codex".to_string(),
                    profile_id: profile_id.to_string(),
                    label: profile_id.to_string(),
                    origin: ProviderAccountProfileOrigin::CharioxCreated,
                    is_default: false,
                },
                files: vec![ProviderAccountMaterializationFile {
                    relative_path: "auth.json".to_string(),
                    contents_base64: "A"
                        .repeat(MAX_PROVIDER_ACCOUNT_COMPONENT_BYTES as usize / 2 + 1_024),
                }],
                generated_at_ms: 1,
            }
        }

        let mut materializations = Vec::new();
        let mut serialized_bytes = 2;
        append_bounded_provider_account_materialization(
            &mut materializations,
            &mut serialized_bytes,
            materialization("first"),
        )
        .expect("first provider account fits");
        assert_eq!(materializations.len(), 1);
        append_bounded_provider_account_materialization(
            &mut materializations,
            &mut serialized_bytes,
            materialization("second"),
        )
        .expect_err("second provider account exceeds aggregate limit");
        assert_eq!(materializations.len(), 1);
    }

    fn persisted_test_artifact(
        ticket: &ManagedContextTransferTicket,
        created_at_ms: u64,
        package_size_bytes: u64,
    ) -> PersistedOutboundArtifact {
        PersistedOutboundArtifact {
            schema_version: OUTBOUND_ARTIFACT_SCHEMA_VERSION,
            created_at_ms,
            environment_id: ticket.environment_id.clone(),
            plan_digest: ticket.context_plan.package_binding().plan_digest,
            target_kernel_id: ticket.target.kernel_id.clone(),
            target_key_thumbprint: ticket.target.key_thumbprint.clone(),
            package_sha256: "c".repeat(64),
            package_size_bytes,
            development_archive_sha256: None,
            kernel_context_snapshot_sha256: None,
            provider_accounts_sha256: None,
            git_credentials_sha256: None,
            capability: crate::transport::relay_peer::RelayManagedContextCapability::new(
                "restart-capability-canary".to_string(),
            ),
        }
    }

    fn persisted_test_ticket(context_id: &str) -> ManagedContextTransferTicket {
        ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan: ManagedKernelContextPlan::source_project_for_tests(
                context_id,
                "realm-1",
                "source-kernel",
                &"a".repeat(64),
                "project-1",
            ),
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                relay_public_key: "target-public-key".to_string(),
                key_thumbprint: "b".repeat(64),
            },
        }
    }

    fn write_persisted_test_artifact(
        root: &Path,
        ticket: &ManagedContextTransferTicket,
        created_at_ms: u64,
        package: Option<&[u8]>,
    ) {
        create_private_directory(root).expect("artifact root");
        if let Some(package) = package {
            fs::write(root.join("managed-context.pkg"), package).expect("package file");
        }
        let persisted = persisted_test_artifact(
            ticket,
            created_at_ms,
            package.map_or(7, |bytes| bytes.len() as u64),
        );
        crate::config::write_private_file(
            &root.join("state.json"),
            &serde_json::to_vec(&persisted).expect("state JSON"),
        )
        .expect("persist state");
    }

    #[test]
    fn operation_store_is_idempotent_and_retryable_failures_can_restart() {
        let store = ManagedContextOutboundOperationStore::default();
        let (first, first_permit) = store.start("context-1", "sha256:one").expect("first start");
        assert!(first_permit.is_some());
        let (same, duplicate_permit) = store
            .start("context-1", "sha256:one")
            .expect("idempotent start");
        assert!(duplicate_permit.is_none());
        assert_eq!(same, first);
        drop(first_permit);
        store.finish("context-1");
        store.update("context-1", |status| {
            status.phase = ManagedContextOutboundOperationPhase::Failed;
            status.retryable = true;
        });
        let (retried, retry_permit) = store.start("context-1", "sha256:one").expect("retry start");
        assert!(retry_permit.is_some());
        assert_eq!(
            retried.phase,
            ManagedContextOutboundOperationPhase::Preparing
        );
        assert!(store.start("context-1", "sha256:two").is_err());
    }

    #[test]
    fn operation_store_is_bounded_and_prunes_only_terminal_operations() {
        let store = ManagedContextOutboundOperationStore::default();
        for index in 0..MAX_OUTBOUND_OPERATIONS {
            let (_, permit) = store
                .start(&format!("context-{index:03}"), "sha256:one")
                .expect("fill operation store");
            drop(permit);
            store.finish(&format!("context-{index:03}"));
        }
        assert!(store.start("context-overflow", "sha256:one").is_err());

        store.update("context-000", |status| {
            status.phase = ManagedContextOutboundOperationPhase::Completed;
        });
        let (_, permit) = store
            .start("context-replacement", "sha256:one")
            .expect("terminal operation should be pruned");
        assert!(permit.is_some());
        assert!(store.get("context-000").is_none());
        assert!(store.get("context-replacement").is_some());
    }

    #[test]
    fn operation_store_caps_expensive_transfers_at_two() {
        let store = ManagedContextOutboundOperationStore::default();
        let (_, first) = store.start("context-1", "sha256:one").expect("first start");
        let (_, second) = store
            .start("context-2", "sha256:two")
            .expect("second start");
        assert!(first.is_some());
        assert!(second.is_some());
        assert!(store.start("context-3", "sha256:three").is_err());
        drop(first);
        store.finish("context-1");
        assert!(store
            .start("context-3", "sha256:three")
            .expect("slot released")
            .1
            .is_some());
    }

    #[test]
    fn ticket_validation_binds_both_kernels_to_the_cloud_realm() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            realm_id: "realm-1".to_string(),
            machine_id: Some("source-machine-test".to_string()),
            ..PersistedCloudRelayProfile::default()
        });
        let source_thumbprint = public_key_thumbprint(&config.relay_public_key);
        let target_private_key = relay_crypto::generate_private_key_base64();
        let target_public_key =
            relay_crypto::public_key_from_private_key_base64(&target_private_key)
                .expect("target public key");
        let mut ticket = ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan: ManagedKernelContextPlan::source_project_for_tests(
                "context-1",
                "realm-1",
                &config.daemon_id,
                &source_thumbprint,
                "project-1",
            ),
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                key_thumbprint: public_key_thumbprint(&target_public_key),
                relay_public_key: target_public_key,
            },
        };
        validate_ticket(&config, &ticket).expect("matching ticket");
        config
            .cloud_relay
            .as_mut()
            .expect("cloud profile")
            .machine_id = Some("another-source-machine".to_string());
        assert!(validate_ticket(&config, &ticket).is_err());
        config
            .cloud_relay
            .as_mut()
            .expect("cloud profile")
            .machine_id = Some("source-machine-test".to_string());
        ticket.target.relay_realm_id = "realm-2".to_string();
        assert!(validate_ticket(&config, &ticket).is_err());
    }

    #[tokio::test]
    async fn authoritative_ticket_is_fetched_with_the_source_machine_credential() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud fixture");
        let address = listener.local_addr().expect("Cloud fixture address");
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: format!("http://{address}"),
            account_id: "account-1".to_string(),
            realm_id: "realm-1".to_string(),
            machine_id: Some("source-machine-test".to_string()),
            machine_credential: Some(format!("mcred_{}", "s".repeat(43))),
            ..PersistedCloudRelayProfile::default()
        });
        let source_thumbprint = public_key_thumbprint(&config.relay_public_key);
        let target_private_key = relay_crypto::generate_private_key_base64();
        let target_public_key =
            relay_crypto::public_key_from_private_key_base64(&target_private_key)
                .expect("target public key");
        let ticket = ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan: ManagedKernelContextPlan::source_project_for_tests(
                "context-1",
                "realm-1",
                &config.daemon_id,
                &source_thumbprint,
                "project-1",
            ),
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                key_thumbprint: public_key_thumbprint(&target_public_key),
                relay_public_key: target_public_key,
            },
        };
        let response = serde_json::to_vec(&ticket).expect("serialize ticket");
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept Cloud request");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("request timeout");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk).expect("read Cloud request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.len()
            )
            .expect("write Cloud response headers");
            stream.write_all(&response).expect("write Cloud response");
            String::from_utf8(request).expect("request UTF-8")
        });
        assert_eq!(
            fetch_authoritative_ticket(&config, &ticket)
                .await
                .expect("authoritative ticket"),
            ticket
        );
        let request = fixture.join().expect("Cloud fixture thread");
        assert!(request.starts_with("POST /v1/managed-kernels/context/ticket "));
        assert!(request.contains("\"machineId\":\"source-machine-test\""));
        assert!(request.contains("\"kernelId\":"));
        assert!(request.contains("\"machineCredential\":\"mcred_"));
    }

    #[test]
    fn unknown_transport_failures_keep_restart_artifacts() {
        let error = DaemonError::LocalTransport {
            operation: "send peer request",
            message: "relay disconnected".to_string(),
        };
        assert!(error_is_retryable(&error));
        let mut status = ManagedContextOutboundOperationStatus {
            context_id: "context-1".to_string(),
            plan_digest: "sha256:one".to_string(),
            phase: ManagedContextOutboundOperationPhase::Uploading,
            accepted_bytes: 1,
            package_size_bytes: 2,
            receipt: None,
            failure_code: None,
            failure_message: None,
            retryable: false,
            updated_at_ms: 0,
        };
        fail_status(&mut status, &error);
        assert!(status.retryable);
    }

    #[test]
    fn cloud_ticket_failures_preserve_terminal_and_transient_classes() {
        let terminal = DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict: body"
                .to_string(),
        };
        assert!(!cloud_error_is_retryable(&terminal));
        let unavailable = DaemonError::LocalTransport {
            operation: "cloud relay request",
            message:
                "cloud relay request failed with 503: cloud_api_code=dependency_unavailable: body"
                    .to_string(),
        };
        assert!(cloud_error_is_retryable(&unavailable));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
        assert!(!cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 404".to_string(),
        }));
    }

    #[cfg(unix)]
    #[test]
    fn private_artifact_state_rejects_symbolic_and_hard_links() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-link-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        create_private_directory(&root).expect("artifact root");
        let target = root.join("target.json");
        fs::write(&target, b"{}").expect("target state");
        let symbolic = root.join("symbolic.json");
        symlink(&target, &symbolic).expect("symbolic link");
        assert!(read_bounded_regular_file(&symbolic, 128).is_err());
        let hard = root.join("hard.json");
        fs::hard_link(&target, &hard).expect("hard link");
        assert!(read_bounded_regular_file(&hard, 128).is_err());
        remove_artifact_root(&root).expect("cleanup");
    }

    #[test]
    fn prepared_package_and_capability_survive_source_restart() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-state-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        create_private_directory(&root).expect("artifact root");
        fs::write(root.join("managed-context.pkg"), b"package").expect("package file");
        let plan = ManagedKernelContextPlan::source_project_for_tests(
            "context-1",
            "realm-1",
            "source-kernel",
            &"a".repeat(64),
            "project-1",
        );
        let ticket = ManagedContextTransferTicket {
            environment_id: "environment-1".to_string(),
            context_plan: plan,
            target: ManagedContextTransferTarget {
                relay_realm_id: "realm-1".to_string(),
                machine_id: "target-machine".to_string(),
                kernel_id: "target-kernel".to_string(),
                relay_public_key: "target-public-key".to_string(),
                key_thumbprint: "b".repeat(64),
            },
        };
        let binding = ticket.context_plan.package_binding();
        let capability = crate::transport::relay_peer::RelayManagedContextCapability::new(
            "restart-capability-canary".to_string(),
        );
        let persisted = PersistedOutboundArtifact {
            schema_version: OUTBOUND_ARTIFACT_SCHEMA_VERSION,
            created_at_ms: crate::session::unix_epoch_ms(),
            environment_id: ticket.environment_id.clone(),
            plan_digest: binding.plan_digest.clone(),
            target_kernel_id: ticket.target.kernel_id.clone(),
            target_key_thumbprint: ticket.target.key_thumbprint.clone(),
            package_sha256: "c".repeat(64),
            package_size_bytes: 7,
            development_archive_sha256: Some("d".repeat(64)),
            kernel_context_snapshot_sha256: Some("e".repeat(64)),
            provider_accounts_sha256: None,
            git_credentials_sha256: None,
            capability: capability.clone(),
        };
        crate::config::write_private_file(
            &root.join("state.json"),
            &serde_json::to_vec(&persisted).expect("state JSON"),
        )
        .expect("persist state");
        let restored =
            restore_prepared_artifact(&ticket, binding, root.clone(), &root.join("state.json"))
                .expect("restore prepared package");
        assert_eq!(restored.capability, capability);
        assert_eq!(restored.package.package_size_bytes, 7);
        assert_eq!(
            restored.package.package_path,
            root.join("managed-context.pkg")
        );
        let state_debug = format!("{persisted:?}");
        assert!(!state_debug.contains("restart-capability-canary"));
        remove_artifact_root(&root).expect("cleanup");
    }

    #[test]
    fn missing_persisted_package_is_retired_across_restarts() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-retired-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        create_private_directory(&parent).expect("artifact parent");
        let root = parent.join("context-1");
        let ticket = persisted_test_ticket("context-1");
        write_persisted_test_artifact(&root, &ticket, crate::session::unix_epoch_ms(), None);
        let error = restore_prepared_artifact(
            &ticket,
            ticket.context_plan.package_binding(),
            root.clone(),
            &root.join("state.json"),
        )
        .err()
        .expect("missing package must retire the transfer");
        assert!(!error_is_retryable(&error));
        assert!(root.join("retired").is_file());
        let replay = restore_prepared_artifact(
            &ticket,
            ticket.context_plan.package_binding(),
            root.clone(),
            &root.join("state.json"),
        )
        .err()
        .expect("retired transfer must remain terminal");
        assert!(!error_is_retryable(&replay));
        let inventory = reconcile_outbound_artifacts(
            &parent,
            &BTreeSet::new(),
            crate::session::unix_epoch_ms(),
        )
        .expect("reconcile retired transfer");
        assert_eq!(inventory.root_count, 1);
        assert_eq!(inventory.artifact_count, 0);
        assert!(root.exists());
        remove_artifact_root(&parent).expect("cleanup");
    }

    #[test]
    fn terminal_preflight_retires_only_the_matching_persisted_artifact() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-preflight-retirement-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = ManagedContextOutboundOperationStore::open(parent.clone()).expect("store");
        let ticket = persisted_test_ticket("context-1");
        let root = parent.join("context-1");
        write_persisted_test_artifact(
            &root,
            &ticket,
            crate::session::unix_epoch_ms(),
            Some(b"package"),
        );

        let mut mismatched = ticket.clone();
        mismatched.target.kernel_id = "different-target-kernel".to_string();
        assert!(!retire_matching_artifact_after_terminal_preflight(
            &DaemonConfig::for_tests(),
            &store,
            &mismatched,
        )
        .expect("mismatched preflight must not mutate artifacts"));
        assert!(root.join("managed-context.pkg").is_file());
        assert!(!root.join("retired").exists());

        assert!(retire_matching_artifact_after_terminal_preflight(
            &DaemonConfig::for_tests(),
            &store,
            &ticket,
        )
        .expect("matching terminal preflight must retire artifacts"));
        assert!(!root.join("managed-context.pkg").exists());
        assert!(root.join("retired").is_file());
        remove_artifact_root(&parent).expect("cleanup");
    }

    #[test]
    fn terminal_preflight_remains_terminal_when_artifact_retirement_cannot_verify_binding() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-preflight-corrupt-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = ManagedContextOutboundOperationStore::open(parent.clone()).expect("store");
        let ticket = persisted_test_ticket("context-corrupt");
        let root = parent.join("context-corrupt");
        create_private_directory(&root).expect("artifact root");
        fs::write(root.join("state.json"), b"{partial").expect("partial state");
        fs::write(root.join("managed-context.pkg"), b"package").expect("package");

        let retirement_error = retire_matching_artifact_after_terminal_preflight(
            &DaemonConfig::for_tests(),
            &store,
            &ticket,
        )
        .err()
        .expect("unverifiable binding must not be retired");
        let terminal_error = outbound_service_error("Cloud rejected the source ticket", false);
        let mut status = ManagedContextOutboundOperationStatus {
            context_id: "context-corrupt".to_string(),
            plan_digest: ticket.context_plan.package_binding().plan_digest,
            phase: ManagedContextOutboundOperationPhase::Preparing,
            accepted_bytes: 0,
            package_size_bytes: 0,
            receipt: None,
            failure_code: None,
            failure_message: None,
            retryable: true,
            updated_at_ms: 0,
        };
        fail_terminal_preflight_status(&mut status, &terminal_error, Some(&retirement_error));

        assert_eq!(status.phase, ManagedContextOutboundOperationPhase::Failed);
        assert!(!status.retryable);
        assert_eq!(
            status.failure_code.as_deref(),
            Some("managed_context_source_unavailable")
        );
        assert!(status
            .failure_message
            .as_deref()
            .is_some_and(|message| message.contains("cleanup did not complete")));
        assert!(root.join("managed-context.pkg").is_file());
        assert!(!root.join("retired").exists());
        remove_artifact_root(&parent).expect("cleanup");
    }

    #[test]
    fn startup_reconciliation_expires_packages_and_discards_partial_state() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-reconcile-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        create_private_directory(&parent).expect("artifact parent");
        let now = crate::session::unix_epoch_ms();
        let current_ticket = persisted_test_ticket("context-current");
        write_persisted_test_artifact(
            &parent.join("context-current"),
            &current_ticket,
            now,
            Some(b"package"),
        );
        let expired_ticket = persisted_test_ticket("context-expired");
        write_persisted_test_artifact(
            &parent.join("context-expired"),
            &expired_ticket,
            now.saturating_sub(OUTBOUND_ARTIFACT_RETENTION_MS),
            Some(b"package"),
        );
        let partial_root = parent.join("context-partial");
        create_private_directory(&partial_root).expect("partial root");
        fs::write(partial_root.join("state.json"), b"{partial")
            .expect("partial state simulates an interrupted Windows write");
        let inventory = reconcile_outbound_artifacts(&parent, &BTreeSet::new(), now)
            .expect("startup reconciliation");
        assert_eq!(inventory.root_count, 2);
        assert_eq!(inventory.artifact_count, 1);
        assert_eq!(inventory.package_bytes, 7);
        assert!(parent.join("context-current").exists());
        assert!(parent.join("context-expired/retired").is_file());
        assert!(!parent.join("context-expired/managed-context.pkg").exists());
        assert!(!partial_root.exists());
        let second_ticket = persisted_test_ticket("context-second");
        write_persisted_test_artifact(
            &parent.join("context-second"),
            &second_ticket,
            now,
            Some(b"package"),
        );
        assert_eq!(
            reconcile_outbound_artifacts(&parent, &BTreeSet::new(), now)
                .expect("two retained packages fit")
                .artifact_count,
            2
        );
        let overflow_ticket = persisted_test_ticket("context-overflow");
        write_persisted_test_artifact(
            &parent.join("context-overflow"),
            &overflow_ticket,
            now,
            Some(b"package"),
        );
        assert!(reconcile_outbound_artifacts(&parent, &BTreeSet::new(), now).is_err());
        remove_artifact_root(&parent).expect("cleanup");
    }

    #[test]
    fn retry_enforces_expiry_and_prunes_the_later_tombstone() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-expiry-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = ManagedContextOutboundOperationStore::open(parent.clone()).expect("store");
        let ticket = persisted_test_ticket("context-expired");
        let root = parent.join("context-expired");
        let now = crate::session::unix_epoch_ms();
        write_persisted_test_artifact(
            &root,
            &ticket,
            now.saturating_sub(OUTBOUND_ARTIFACT_RETENTION_MS),
            Some(b"package"),
        );
        store
            .active
            .lock()
            .expect("active operations")
            .insert("context-expired".to_string());

        let provider_accounts = crate::account_profile::ProviderAccountProfileRegistry::open(
            parent.with_extension("accounts.json"),
        )
        .expect("account registry");
        let error = prepare_managed_context_package(
            &DaemonConfig::for_tests(),
            &store,
            &provider_accounts,
            &ticket,
        )
        .err()
        .expect("expired retry must fail");
        assert!(!error_is_retryable(&error));
        assert!(root.join("retired").is_file());
        assert!(!root.join("managed-context.pkg").exists());

        let future = crate::session::unix_epoch_ms()
            .saturating_add(OUTBOUND_ARTIFACT_RETENTION_MS)
            .saturating_add(60_000);
        let inventory = reconcile_outbound_artifacts(&parent, &BTreeSet::new(), future)
            .expect("expired tombstone cleanup");
        assert_eq!(inventory.root_count, 0);
        assert!(!root.exists());
        remove_artifact_root(&parent).expect("cleanup");
    }

    #[test]
    fn retained_tombstones_consume_the_bounded_root_inventory() {
        let parent = std::env::temp_dir().join(format!(
            "chariox-managed-context-outbound-root-quota-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = ManagedContextOutboundOperationStore::open(parent.clone()).expect("store");
        let now = crate::session::unix_epoch_ms();
        for index in 0..MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES {
            let context_id = format!("context-{index:03}");
            let ticket = persisted_test_ticket(&context_id);
            let root = parent.join(&context_id);
            write_persisted_test_artifact(&root, &ticket, now, Some(b"package"));
            retire_artifact_root(&root).expect("retire artifact");
        }
        let inventory = reconcile_outbound_artifacts(&parent, &BTreeSet::new(), now)
            .expect("bounded tombstone inventory");
        assert_eq!(inventory.root_count, MAX_OUTBOUND_ARTIFACT_SCAN_ENTRIES);
        assert_eq!(inventory.artifact_count, 0);

        let overflow = persisted_test_ticket("context-overflow");
        let provider_accounts = crate::account_profile::ProviderAccountProfileRegistry::open(
            parent.with_extension("accounts.json"),
        )
        .expect("account registry");
        let error = prepare_managed_context_package(
            &DaemonConfig::for_tests(),
            &store,
            &provider_accounts,
            &overflow,
        )
        .err()
        .expect("root quota must reject another export");
        assert!(!error_is_retryable(&error));
        assert!(!parent.join("context-overflow").exists());
        remove_artifact_root(&parent).expect("cleanup");
    }
}
