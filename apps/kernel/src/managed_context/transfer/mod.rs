use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

mod model;
mod policy;
mod storage;

pub(crate) use model::{
    ArmManagedContextTransfer, ArmedManagedContextTransfer, ManagedContextImportClaim,
    ManagedContextTransferCaller, ManagedContextTransferPhase, ManagedContextTransferStatus,
    ReadyManagedContextImport,
};
use policy::{
    authorize_entry, current_time_ms, prune_expired, random_identifier, sha256_bytes, status,
    transfer_error, validate_arm_request, validate_persisted_state, validate_sha256,
};
use storage::{
    create_or_validate_empty_archive, ensure_private_directory, open_private_archive,
    read_private_state_file, remove_archive_if_present, sha256_file, transfer_io_error,
    write_private_state_file, MAX_STATE_FILE_BYTES,
};

const TRANSFER_STATE_SCHEMA_VERSION: u32 = 5;
const MAX_ACTIVE_TRANSFERS: usize = 64;
const MAX_TRANSFER_RECORDS: usize = 256;
const MAX_ARCHIVE_BYTES: u64 = crate::managed_context::package::MAX_MANAGED_CONTEXT_PACKAGE_BYTES;
pub(crate) const MAX_TRANSFER_CHUNK_BYTES: usize = 512 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 4096;
const MAX_DESTINATION_BYTES: usize = 16 * 1024;
const MAX_TRANSFER_TTL_MS: u64 = 30 * 60 * 1_000;
const COMPLETED_TRANSFER_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;
pub(crate) const MAX_IMPORT_RECEIPT_BYTES: usize = 128 * 1024;
const MAX_PERSISTED_IMPORT_BYTES: usize = MAX_IMPORT_RECEIPT_BYTES * 3 + 16 * 1024;
const STATE_CAPACITY_MARGIN_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTransferState {
    schema_version: u32,
    entries: BTreeMap<String, PersistedTransfer>,
    #[serde(default)]
    consumed_context_ids: BTreeSet<String>,
    #[serde(default)]
    applied_contexts: BTreeMap<String, crate::local::ManagedContextLaunchTarget>,
}

impl Default for PersistedTransferState {
    fn default() -> Self {
        Self {
            schema_version: TRANSFER_STATE_SCHEMA_VERSION,
            entries: BTreeMap::new(),
            consumed_context_ids: BTreeSet::new(),
            applied_contexts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTransfer {
    capability_sha256: String,
    #[serde(default = "legacy_plan_binding")]
    plan: crate::managed_context::package::ManagedContextPlanBinding,
    #[serde(
        default,
        rename = "context_id",
        skip_serializing_if = "String::is_empty"
    )]
    legacy_context_id: String,
    #[serde(
        default,
        rename = "project_id",
        skip_serializing_if = "String::is_empty"
    )]
    legacy_project_id: String,
    target_environment_id: String,
    target_kernel_id: String,
    target_key_thumbprint: String,
    source_kernel_id: String,
    source_key_thumbprint: String,
    owner_user_id: String,
    realm_id: String,
    archive_sha256: String,
    archive_size_bytes: u64,
    destination_root: PathBuf,
    expires_at_ms: u64,
    phase: ManagedContextTransferPhase,
    accepted_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import_receipt_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import_receipt_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import_started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
}

fn legacy_plan_binding() -> crate::managed_context::package::ManagedContextPlanBinding {
    use crate::managed_context::package::{
        ManagedContextDevelopmentSelection, ManagedContextGitCredentialSelection,
        ManagedContextKernelSelection, ManagedContextPlanBinding,
        ManagedContextProviderAccountSelection,
    };
    ManagedContextPlanBinding {
        context_id: "legacy-pending-migration".to_string(),
        plan_digest: format!("sha256:{}", "0".repeat(64)),
        kernel_context: ManagedContextKernelSelection::Empty,
        development: ManagedContextDevelopmentSelection::Empty,
        provider_accounts: ManagedContextProviderAccountSelection::None,
        git_credentials: ManagedContextGitCredentialSelection::None,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedContextLaunchRecoveryBinding {
    pub environment_id: String,
    pub kernel_id: String,
    pub plan: crate::managed_context::package::ManagedContextPlanBinding,
}

#[derive(Clone)]
pub(crate) struct ManagedContextTransferStore {
    root: PathBuf,
    state: Arc<Mutex<PersistedTransferState>>,
    active_imports: Arc<Mutex<HashSet<String>>>,
}

impl ManagedContextTransferStore {
    pub(crate) fn open(root: PathBuf) -> Result<Self, DaemonError> {
        Self::open_with_launch_recovery(root, None)
    }

    pub(crate) fn open_with_launch_recovery(
        root: PathBuf,
        recovery: Option<&ManagedContextLaunchRecoveryBinding>,
    ) -> Result<Self, DaemonError> {
        ensure_private_directory(&root)?;
        let state_path = root.join("state.json");
        let state = match read_private_state_file(&state_path)? {
            Some(bytes) => {
                serde_json::from_slice::<PersistedTransferState>(&bytes).map_err(|error| {
                    transfer_error(format!("parse managed context transfer state: {error}"))
                })?
            }
            None => PersistedTransferState::default(),
        };
        if !matches!(
            state.schema_version,
            1 | 2 | 3 | 4 | TRANSFER_STATE_SCHEMA_VERSION
        ) {
            return Err(transfer_error(format!(
                "unsupported managed context transfer state version {}",
                state.schema_version
            )));
        }
        let store = Self {
            root,
            state: Arc::new(Mutex::new(state)),
            active_imports: Arc::new(Mutex::new(HashSet::new())),
        };
        if store.lock_state().schema_version < TRANSFER_STATE_SCHEMA_VERSION {
            store.migrate_legacy_state(recovery)?;
        }
        validate_persisted_state(&store.lock_state())?;
        store.cleanup_failed_transfers()?;
        store.prune_expired_transfers(current_time_ms())?;
        store.cleanup_interrupted_import_staging()?;
        store.cleanup_consumed_archives()?;
        store.reconcile_archive_lengths()?;
        Ok(store)
    }

    pub(crate) fn arm(
        &self,
        request: ArmManagedContextTransfer,
        now_ms: u64,
    ) -> Result<ArmedManagedContextTransfer, DaemonError> {
        validate_arm_request(&request, now_ms)?;
        ensure_private_directory(&request.destination_parent)?;
        let canonical_destination_parent = fs::canonicalize(&request.destination_parent)
            .map_err(|error| transfer_io_error("resolve managed context destination", error))?;
        self.prune_expired_transfers(now_ms)?;
        let mut state = self.lock_state();
        if state
            .consumed_context_ids
            .contains(&request.plan.context_id)
        {
            return Err(transfer_error(
                "managed context launch authorization has already been consumed",
            ));
        }
        let capability_sha256 = sha256_bytes(request.capability.as_bytes());
        if let Some((transfer_id, entry)) = state
            .entries
            .iter()
            .find(|(_, entry)| entry.plan.context_id == request.plan.context_id)
        {
            if entry.plan == request.plan
                && entry.target_environment_id == request.target_environment_id
                && entry.target_kernel_id == request.target_kernel_id
                && entry.target_key_thumbprint == request.target_key_thumbprint
                && entry.source_kernel_id == request.source_kernel_id
                && entry.source_key_thumbprint == request.source_key_thumbprint
                && entry.owner_user_id == request.owner_user_id
                && entry.realm_id == request.realm_id
                && entry.archive_sha256 == request.archive_sha256.to_ascii_lowercase()
                && entry.archive_size_bytes == request.archive_size_bytes
                && entry.capability_sha256 == capability_sha256
            {
                return Ok(ArmedManagedContextTransfer {
                    transfer_id: transfer_id.clone(),
                    capability: request.capability,
                    expires_at_ms: entry.expires_at_ms,
                });
            }
            return Err(transfer_error(
                "managed context launch authorization conflicts with an existing transfer",
            ));
        }
        let reserved_contexts = state.consumed_context_ids.len()
            + state
                .entries
                .values()
                .filter(|entry| {
                    !matches!(
                        entry.phase,
                        ManagedContextTransferPhase::Consumed | ManagedContextTransferPhase::Failed
                    )
                })
                .count();
        if reserved_contexts >= MAX_TRANSFER_RECORDS {
            return Err(transfer_error(
                "managed context consumed authorization capacity is full",
            ));
        }
        if state
            .entries
            .values()
            .filter(|entry| {
                !matches!(
                    entry.phase,
                    ManagedContextTransferPhase::Consumed | ManagedContextTransferPhase::Failed
                )
            })
            .count()
            >= MAX_ACTIVE_TRANSFERS
        {
            return Err(transfer_error("managed context transfer capacity is full"));
        }
        if state.entries.len() >= MAX_TRANSFER_RECORDS {
            return Err(transfer_error(
                "managed context transfer record capacity is full",
            ));
        }
        let transfer_id = random_identifier("ctx");
        let capability = request.capability;
        state.entries.insert(
            transfer_id.clone(),
            PersistedTransfer {
                capability_sha256,
                plan: request.plan,
                legacy_context_id: String::new(),
                legacy_project_id: String::new(),
                target_environment_id: request.target_environment_id,
                target_kernel_id: request.target_kernel_id,
                target_key_thumbprint: request.target_key_thumbprint,
                source_kernel_id: request.source_kernel_id,
                source_key_thumbprint: request.source_key_thumbprint,
                owner_user_id: request.owner_user_id,
                realm_id: request.realm_id,
                archive_sha256: request.archive_sha256.to_ascii_lowercase(),
                archive_size_bytes: request.archive_size_bytes,
                destination_root: canonical_destination_parent.join(&transfer_id),
                expires_at_ms: request.expires_at_ms,
                phase: ManagedContextTransferPhase::Armed,
                accepted_bytes: 0,
                import_receipt_sha256: None,
                import_receipt_json: None,
                completed_at_ms: None,
                import_started_at_ms: None,
                failure_code: None,
            },
        );
        if let Err(error) = ensure_state_capacity_with_receipt_reservations(&state) {
            state.entries.remove(&transfer_id);
            return Err(error);
        }
        if let Err(error) = self.persist_locked(&state) {
            state.entries.remove(&transfer_id);
            return Err(error);
        }
        Ok(ArmedManagedContextTransfer {
            transfer_id,
            capability,
            expires_at_ms: request.expires_at_ms,
        })
    }

    pub(crate) fn begin(
        &self,
        transfer_id: &str,
        capability: &str,
        caller: &ManagedContextTransferCaller,
        now_ms: u64,
    ) -> Result<ManagedContextTransferStatus, DaemonError> {
        let mut state = self.lock_state();
        let (result, changed) = {
            let entry = authorize_entry(&mut state, transfer_id, capability, caller, now_ms)?;
            let mut changed = false;
            if entry.phase == ManagedContextTransferPhase::Armed {
                let archive_path = self.archive_path(transfer_id);
                create_or_validate_empty_archive(&archive_path)?;
                entry.phase = ManagedContextTransferPhase::Receiving;
                changed = true;
            }
            (status(transfer_id, entry), changed)
        };
        if changed {
            if let Err(error) = self.persist_locked(&state) {
                if let Some(entry) = state.entries.get_mut(transfer_id) {
                    entry.phase = ManagedContextTransferPhase::Armed;
                }
                return Err(error);
            }
        }
        Ok(result)
    }

    pub(crate) fn upload_chunk(
        &self,
        transfer_id: &str,
        capability: &str,
        caller: &ManagedContextTransferCaller,
        offset: u64,
        bytes: &[u8],
        chunk_sha256: &str,
        now_ms: u64,
    ) -> Result<ManagedContextTransferStatus, DaemonError> {
        if bytes.is_empty() || bytes.len() > MAX_TRANSFER_CHUNK_BYTES {
            return Err(transfer_error(format!(
                "managed context chunk must contain between 1 and {MAX_TRANSFER_CHUNK_BYTES} bytes"
            )));
        }
        validate_sha256(chunk_sha256, "chunk")?;
        if sha256_bytes(bytes) != chunk_sha256.to_ascii_lowercase() {
            return Err(transfer_error(
                "managed context chunk digest does not match",
            ));
        }
        let mut state = self.lock_state();
        let entry = authorize_entry(&mut state, transfer_id, capability, caller, now_ms)?;
        if entry.phase != ManagedContextTransferPhase::Receiving {
            return Err(transfer_error(
                "managed context transfer is not receiving chunks",
            ));
        }
        let end = offset.saturating_add(bytes.len() as u64);
        if end > entry.archive_size_bytes {
            return Err(transfer_error(
                "managed context chunk exceeds the declared archive size",
            ));
        }
        let archive_path = self.archive_path(transfer_id);
        let mut archive = open_private_archive(&archive_path)?;
        let length = archive
            .metadata()
            .map_err(|error| transfer_io_error("inspect managed context archive", error))?
            .len();
        if length != entry.accepted_bytes {
            return Err(transfer_error(
                "managed context archive length does not match its durable offset",
            ));
        }
        if offset < entry.accepted_bytes {
            if end > entry.accepted_bytes {
                return Err(transfer_error(
                    "managed context chunk overlaps the accepted offset",
                ));
            }
            let mut existing = vec![0_u8; bytes.len()];
            archive
                .seek(SeekFrom::Start(offset))
                .and_then(|_| archive.read_exact(&mut existing))
                .map_err(|error| transfer_io_error("read accepted managed context chunk", error))?;
            if existing != bytes {
                return Err(transfer_error(
                    "managed context chunk retry conflicts with accepted bytes",
                ));
            }
            return Ok(status(transfer_id, entry));
        }
        if offset != entry.accepted_bytes {
            return Err(transfer_error(format!(
                "managed context chunk offset must equal {}",
                entry.accepted_bytes
            )));
        }
        archive
            .seek(SeekFrom::End(0))
            .and_then(|_| archive.write_all(bytes))
            .and_then(|_| archive.sync_all())
            .map_err(|error| transfer_io_error("append managed context chunk", error))?;
        entry.accepted_bytes = end;
        let result = status(transfer_id, entry);
        if let Err(error) = self.persist_locked(&state) {
            if let Err(reconciliation) = self.reconcile_uncertain_chunk_persist(
                &mut state,
                transfer_id,
                offset,
                end,
                &archive,
            ) {
                return Err(reconciliation);
            }
            return Err(error);
        }
        Ok(result)
    }

    pub(crate) fn prepare_and_claim_import(
        &self,
        transfer_id: &str,
        capability: &str,
        caller: &ManagedContextTransferCaller,
        now_ms: u64,
    ) -> Result<ManagedContextImportClaim, DaemonError> {
        let mut state = self.lock_state();
        {
            let entry = authorize_entry(&mut state, transfer_id, capability, caller, now_ms)?;
            if matches!(
                entry.phase,
                ManagedContextTransferPhase::Failed | ManagedContextTransferPhase::Consumed
            ) {
                return Ok(ManagedContextImportClaim::Terminal(status(
                    transfer_id,
                    entry,
                )));
            }
        }
        ensure_state_capacity_with_receipt_reservations(&state)?;
        let entry = state
            .entries
            .get_mut(transfer_id)
            .ok_or_else(|| transfer_error("managed context transfer disappeared"))?;
        let mut active_imports = self.lock_active_imports();
        if entry.phase == ManagedContextTransferPhase::Importing {
            if active_imports.contains(transfer_id) {
                return Ok(ManagedContextImportClaim::InProgress(status(
                    transfer_id,
                    entry,
                )));
            }
            active_imports.insert(transfer_id.to_string());
            return Ok(ManagedContextImportClaim::Claimed(ready_import(
                &self.archive_path(transfer_id),
                transfer_id,
                entry,
            )));
        }
        if !matches!(
            entry.phase,
            ManagedContextTransferPhase::Receiving | ManagedContextTransferPhase::ReadyToImport
        ) {
            return Err(transfer_error(
                "managed context transfer is not ready to import",
            ));
        }
        if entry.accepted_bytes != entry.archive_size_bytes {
            return Err(transfer_error("managed context archive is incomplete"));
        }
        let archive_path = self.archive_path(transfer_id);
        if sha256_file(&archive_path)? != entry.archive_sha256 {
            return Err(transfer_error(
                "managed context archive digest does not match",
            ));
        }
        let prior_phase = entry.phase;
        entry.phase = ManagedContextTransferPhase::Importing;
        entry.import_started_at_ms = Some(now_ms);
        active_imports.insert(transfer_id.to_string());
        let ready = ready_import(&archive_path, transfer_id, entry);
        if let Err(error) = self.persist_locked(&state) {
            if let Some(entry) = state.entries.get_mut(transfer_id) {
                entry.phase = prior_phase;
                entry.import_started_at_ms = None;
            }
            active_imports.remove(transfer_id);
            return Err(error);
        }
        Ok(ManagedContextImportClaim::Claimed(ready))
    }

    pub(crate) fn release_import(&self, transfer_id: &str) -> Result<(), DaemonError> {
        self.lock_active_imports().remove(transfer_id);
        Ok(())
    }

    pub(crate) fn retire_import(
        &self,
        transfer_id: &str,
        failure_code: &str,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        if failure_code.is_empty() || failure_code.len() > 128 {
            return Err(transfer_error(
                "managed context import failure code is invalid",
            ));
        }
        let mut state = self.lock_state();
        let destination_root = {
            let entry = state
                .entries
                .get_mut(transfer_id)
                .ok_or_else(|| transfer_error("managed context transfer does not exist"))?;
            if entry.phase != ManagedContextTransferPhase::Importing {
                return Err(transfer_error("managed context transfer is not importing"));
            }
            entry.phase = ManagedContextTransferPhase::Failed;
            entry.failure_code = Some(failure_code.to_string());
            entry.completed_at_ms = Some(now_ms);
            entry.destination_root.clone()
        };
        let persist_result = self.persist_locked(&state);
        self.lock_active_imports().remove(transfer_id);
        if let Err(error) = persist_result {
            if let Some(entry) = state.entries.get_mut(transfer_id) {
                entry.phase = ManagedContextTransferPhase::Importing;
                entry.failure_code = None;
                entry.completed_at_ms = None;
            }
            return Err(error);
        }
        drop(state);
        crate::managed_context::development::cleanup_development_context_publication_staging(
            &destination_root,
            transfer_id,
        )?;
        crate::managed_context::development::cleanup_development_context_publication(
            &destination_root,
            transfer_id,
        )?;
        self.cleanup_transfer_artifacts(transfer_id)
    }

    pub(crate) fn commit_import(
        &self,
        transfer_id: &str,
        import_receipt_json: &str,
        now_ms: u64,
    ) -> Result<(), DaemonError> {
        if import_receipt_json.is_empty() || import_receipt_json.len() > MAX_IMPORT_RECEIPT_BYTES {
            return Err(transfer_error(
                "managed context import receipt size is invalid",
            ));
        }
        serde_json::from_str::<serde_json::Value>(import_receipt_json)
            .map_err(|_| transfer_error("managed context import receipt is invalid JSON"))?;
        let import_receipt = serde_json::from_str::<
            crate::managed_context::package::ManagedContextPackageImportReceipt,
        >(import_receipt_json)
        .ok();
        let receipt_sha256 = sha256_bytes(import_receipt_json.as_bytes());
        let mut state = self.lock_state();
        let existing = state
            .entries
            .get(transfer_id)
            .ok_or_else(|| transfer_error("managed context transfer does not exist"))?;
        let launch_target = import_receipt
            .as_ref()
            .map(|receipt| launch_target_from_receipt(transfer_id, existing, receipt))
            .transpose()?;
        if existing.phase == ManagedContextTransferPhase::Consumed {
            return if existing.import_receipt_sha256.as_deref() == Some(receipt_sha256.as_str())
                && existing.import_receipt_json.as_deref() == Some(import_receipt_json)
                && launch_target.as_ref().is_none_or(|target| {
                    state.applied_contexts.get(&existing.plan.context_id) == Some(target)
                }) {
                drop(state);
                self.cleanup_transfer_artifacts(transfer_id)
            } else {
                Err(transfer_error(
                    "managed context import receipt conflicts with the consumed transfer",
                ))
            };
        }
        if state.consumed_context_ids.len() >= MAX_TRANSFER_RECORDS {
            return Err(transfer_error(
                "managed context consumed authorization capacity is full",
            ));
        }
        if launch_target.is_some()
            && !state
                .applied_contexts
                .contains_key(&existing.plan.context_id)
            && state.applied_contexts.len() >= MAX_TRANSFER_RECORDS
        {
            return Err(transfer_error(
                "managed context launch target capacity is full",
            ));
        }
        let context_id = existing.plan.context_id.clone();
        let entry = state
            .entries
            .get_mut(transfer_id)
            .ok_or_else(|| transfer_error("managed context transfer disappeared"))?;
        if entry.phase != ManagedContextTransferPhase::Importing {
            return Err(transfer_error(
                "managed context transfer is not ready to commit",
            ));
        }
        entry.phase = ManagedContextTransferPhase::Consumed;
        entry.import_receipt_sha256 = Some(receipt_sha256);
        entry.import_receipt_json = Some(import_receipt_json.to_string());
        entry.completed_at_ms = Some(now_ms);
        state.consumed_context_ids.insert(context_id.clone());
        let launch_target_changed = launch_target.is_some();
        let prior_launch_target = launch_target
            .and_then(|target| state.applied_contexts.insert(context_id.clone(), target));
        if let Err(error) = self.persist_locked(&state) {
            if let Some(entry) = state.entries.get_mut(transfer_id) {
                entry.phase = ManagedContextTransferPhase::Importing;
                entry.import_receipt_sha256 = None;
                entry.import_receipt_json = None;
                entry.completed_at_ms = None;
            }
            state.consumed_context_ids.remove(&context_id);
            if launch_target_changed {
                if let Some(target) = prior_launch_target {
                    state.applied_contexts.insert(context_id, target);
                } else {
                    state.applied_contexts.remove(&context_id);
                }
            }
            return Err(error);
        }
        self.lock_active_imports().remove(transfer_id);
        drop(state);
        self.cleanup_transfer_artifacts(transfer_id)
    }

    pub(crate) fn get_status(
        &self,
        transfer_id: &str,
        capability: &str,
        caller: &ManagedContextTransferCaller,
        now_ms: u64,
    ) -> Result<ManagedContextTransferStatus, DaemonError> {
        let mut state = self.lock_state();
        let entry = authorize_entry(&mut state, transfer_id, capability, caller, now_ms)?;
        Ok(status(transfer_id, entry))
    }

    pub(crate) fn launch_target(
        &self,
        context_id: &str,
        plan_digest: &str,
    ) -> Result<crate::local::ManagedContextLaunchTarget, DaemonError> {
        let state = self.lock_state();
        let Some(target) = state.applied_contexts.get(context_id) else {
            if state.entries.values().any(|entry| {
                entry.plan.context_id == context_id && entry.plan.plan_digest == plan_digest
            }) {
                return Err(DaemonError::ManagedContext {
                    code: "managed_context_launch_target_unavailable",
                    operation: "get managed context launch target",
                    message: "managed context launch target is not committed yet".to_string(),
                    retryable: true,
                });
            }
            return Err(transfer_error(
                "managed context launch target is unavailable",
            ));
        };
        if target.plan_digest != plan_digest {
            return Err(transfer_error(
                "managed context launch target plan digest does not match",
            ));
        }
        Ok(target.clone())
    }

    fn archive_path(&self, transfer_id: &str) -> PathBuf {
        self.root.join(format!("{transfer_id}.archive"))
    }

    fn cleanup_transfer_artifacts(&self, transfer_id: &str) -> Result<(), DaemonError> {
        let archive_path = self.archive_path(transfer_id);
        crate::managed_context::package::cleanup_package_components(&archive_path)?;
        remove_archive_if_present(&archive_path)
    }

    fn migrate_legacy_state(
        &self,
        recovery: Option<&ManagedContextLaunchRecoveryBinding>,
    ) -> Result<(), DaemonError> {
        let mut state = self.lock_state();
        if state.schema_version >= TRANSFER_STATE_SCHEMA_VERSION {
            return Ok(());
        }
        let legacy_version = state.schema_version;
        if legacy_version <= 2 {
            for (transfer_id, entry) in std::mem::take(&mut state.entries) {
                if entry.phase == ManagedContextTransferPhase::Consumed {
                    let context_id = if entry.legacy_context_id.is_empty() {
                        format!("legacy-v{legacy_version}-{transfer_id}")
                    } else {
                        entry.legacy_context_id.clone()
                    };
                    state.consumed_context_ids.insert(context_id);
                    self.cleanup_transfer_artifacts(&transfer_id)?;
                    continue;
                }

                crate::managed_context::development::cleanup_development_context_publication_staging(
                    &entry.destination_root,
                    &transfer_id,
                )?;
                if matches!(
                    entry.phase,
                    ManagedContextTransferPhase::Importing | ManagedContextTransferPhase::Failed
                ) {
                    crate::managed_context::development::cleanup_development_context_publication(
                        &entry.destination_root,
                        &transfer_id,
                    )?;
                }
                self.cleanup_transfer_artifacts(&transfer_id)?;
            }
        } else {
            let mut applied_contexts = Vec::new();
            for (transfer_id, entry) in &state.entries {
                if entry.phase != ManagedContextTransferPhase::Consumed {
                    continue;
                }
                let receipt_json = entry.import_receipt_json.as_deref().ok_or_else(|| {
                    transfer_error("consumed managed context transfer has no import receipt")
                })?;
                let receipt = serde_json::from_str::<
                    crate::managed_context::package::ManagedContextPackageImportReceipt,
                >(receipt_json)
                .map_err(|_| transfer_error("stored managed context import receipt is invalid"))?;
                applied_contexts.push((
                    entry.plan.context_id.clone(),
                    launch_target_from_receipt(transfer_id, entry, &receipt)?,
                ));
            }
            for (context_id, target) in applied_contexts {
                state.applied_contexts.insert(context_id, target);
            }
            if let Some(recovery) = recovery.filter(|recovery| {
                state
                    .consumed_context_ids
                    .contains(&recovery.plan.context_id)
                    && !state
                        .applied_contexts
                        .contains_key(&recovery.plan.context_id)
            }) {
                let target = recover_launch_target_from_publication(&self.root, recovery)?;
                state
                    .applied_contexts
                    .insert(recovery.plan.context_id.clone(), target);
            }
            if legacy_version == 4 {
                let state_root = self
                    .root
                    .parent()
                    .ok_or_else(|| transfer_error("managed context transfer root has no parent"))?;
                for (context_id, target) in &mut state.applied_contexts {
                    if let crate::local::ManagedContextDevelopmentLaunchTarget::Empty {
                        workspace_path,
                    } = &mut target.development
                    {
                        if workspace_path.is_empty() {
                            *workspace_path = state_root
                                .join("managed-context-empty-workspaces")
                                .join(sha256_bytes(context_id.as_bytes()))
                                .join("workspace")
                                .to_string_lossy()
                                .into_owned();
                        }
                    }
                }
            }
        }
        state.schema_version = TRANSFER_STATE_SCHEMA_VERSION;
        validate_persisted_state(&state)?;
        let removed = compact_consumed_entries_for_state_capacity(&mut state)?;
        for transfer_id in removed {
            self.cleanup_transfer_artifacts(&transfer_id)?;
        }
        self.persist_locked(&state)
    }

    fn persist_locked(&self, state: &PersistedTransferState) -> Result<(), DaemonError> {
        ensure_state_file_capacity(state)?;
        let bytes = serde_json::to_vec(state).map_err(|error| {
            transfer_error(format!("serialize managed context transfer state: {error}"))
        })?;
        write_private_state_file(&self.root.join("state.json"), &bytes)
    }

    fn reconcile_uncertain_chunk_persist(
        &self,
        state: &mut PersistedTransferState,
        transfer_id: &str,
        prior_offset: u64,
        appended_offset: u64,
        archive: &std::fs::File,
    ) -> Result<(), DaemonError> {
        let bytes = read_private_state_file(&self.root.join("state.json"))?
            .ok_or_else(|| transfer_error("managed context transfer state disappeared"))?;
        let durable = serde_json::from_slice::<PersistedTransferState>(&bytes)
            .map_err(|error| transfer_error(format!("parse durable transfer state: {error}")))?;
        if durable.schema_version != TRANSFER_STATE_SCHEMA_VERSION {
            return Err(transfer_error(
                "durable managed context transfer state version changed",
            ));
        }
        validate_persisted_state(&durable)?;
        let durable_offset = durable
            .entries
            .get(transfer_id)
            .ok_or_else(|| transfer_error("durable managed context transfer disappeared"))?
            .accepted_bytes;
        match durable_offset {
            offset if offset == appended_offset => {}
            offset if offset == prior_offset => {
                archive
                    .set_len(prior_offset)
                    .and_then(|_| archive.sync_all())
                    .map_err(|error| {
                        transfer_io_error("roll back uncommitted managed context chunk", error)
                    })?;
            }
            _ => {
                return Err(transfer_error(
                    "durable managed context transfer offset is inconsistent",
                ))
            }
        }
        *state = durable;
        Ok(())
    }

    fn reconcile_archive_lengths(&self) -> Result<(), DaemonError> {
        let state = self.lock_state();
        for (transfer_id, entry) in &state.entries {
            if matches!(
                entry.phase,
                ManagedContextTransferPhase::Armed
                    | ManagedContextTransferPhase::Failed
                    | ManagedContextTransferPhase::Consumed
            ) {
                continue;
            }
            let path = self.archive_path(transfer_id);
            let file = open_private_archive(&path)?;
            let length = file
                .metadata()
                .map_err(|error| transfer_io_error("inspect managed context archive", error))?
                .len();
            if length < entry.accepted_bytes {
                return Err(transfer_error(format!(
                    "managed context transfer `{transfer_id}` lost accepted bytes"
                )));
            }
            if length > entry.accepted_bytes {
                file.set_len(entry.accepted_bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|error| {
                        transfer_io_error("reconcile managed context archive", error)
                    })?;
            }
        }
        Ok(())
    }

    fn cleanup_interrupted_import_staging(&self) -> Result<(), DaemonError> {
        let state = self.lock_state();
        for (transfer_id, entry) in &state.entries {
            if entry.phase == ManagedContextTransferPhase::Importing {
                crate::managed_context::package::cleanup_package_components(
                    &self.archive_path(transfer_id),
                )?;
                crate::managed_context::development::cleanup_development_context_publication_staging(
                    &entry.destination_root,
                    transfer_id,
                )?;
            }
        }
        Ok(())
    }

    fn cleanup_consumed_archives(&self) -> Result<(), DaemonError> {
        let state = self.lock_state();
        for (transfer_id, entry) in &state.entries {
            if entry.phase == ManagedContextTransferPhase::Consumed {
                self.cleanup_transfer_artifacts(transfer_id)?;
            }
        }
        Ok(())
    }

    fn cleanup_failed_transfers(&self) -> Result<(), DaemonError> {
        let failed = {
            let state = self.lock_state();
            state
                .entries
                .iter()
                .filter(|(_, entry)| entry.phase == ManagedContextTransferPhase::Failed)
                .map(|(transfer_id, entry)| (transfer_id.clone(), entry.destination_root.clone()))
                .collect::<Vec<_>>()
        };
        for (transfer_id, destination_root) in failed {
            crate::managed_context::development::cleanup_development_context_publication_staging(
                &destination_root,
                &transfer_id,
            )?;
            crate::managed_context::development::cleanup_development_context_publication(
                &destination_root,
                &transfer_id,
            )?;
            self.cleanup_transfer_artifacts(&transfer_id)?;
        }
        Ok(())
    }

    fn prune_expired_transfers(&self, now_ms: u64) -> Result<(), DaemonError> {
        let mut state = self.lock_state();
        let state_before_prune = state.clone();
        let expired = prune_expired(&mut state, now_ms);
        if expired.is_empty() {
            return Ok(());
        }
        for expired_id in expired {
            if let Some(entry) = state_before_prune.entries.get(&expired_id) {
                if entry.phase == ManagedContextTransferPhase::Failed {
                    if let Err(error) = crate::managed_context::development::cleanup_development_context_publication_staging(
                        &entry.destination_root,
                        &expired_id,
                    ) {
                        *state = state_before_prune;
                        return Err(error);
                    }
                    if let Err(error) =
                        crate::managed_context::development::cleanup_development_context_publication(
                            &entry.destination_root,
                            &expired_id,
                        )
                    {
                        *state = state_before_prune;
                        return Err(error);
                    }
                }
            }
            if let Err(error) = self.cleanup_transfer_artifacts(&expired_id) {
                *state = state_before_prune;
                return Err(error);
            }
        }
        if let Err(error) = self.persist_locked(&state) {
            *state = state_before_prune;
            return Err(error);
        }
        Ok(())
    }

    fn lock_state(&self) -> MutexGuard<'_, PersistedTransferState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_active_imports(&self) -> MutexGuard<'_, HashSet<String>> {
        self.active_imports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn launch_target_from_receipt(
    transfer_id: &str,
    entry: &PersistedTransfer,
    receipt: &crate::managed_context::package::ManagedContextPackageImportReceipt,
) -> Result<crate::local::ManagedContextLaunchTarget, DaemonError> {
    use crate::local::{ManagedContextDevelopmentLaunchTarget, ManagedContextLaunchTarget};
    use crate::managed_context::package::ManagedContextImportedDevelopment;

    if receipt.transfer_id != transfer_id
        || receipt.plan_digest != entry.plan.plan_digest
        || receipt.package_sha256 != entry.archive_sha256
    {
        return Err(transfer_error(
            "managed context import receipt does not match the transfer",
        ));
    }
    let development = match &receipt.development {
        ManagedContextImportedDevelopment::Empty => {
            if !matches!(
                entry.plan.development,
                crate::managed_context::package::ManagedContextDevelopmentSelection::Empty
            ) {
                return Err(transfer_error(
                    "managed context import receipt omits selected development context",
                ));
            }
            ManagedContextDevelopmentLaunchTarget::Empty {
                workspace_path: entry
                    .destination_root
                    .join("workspace")
                    .to_string_lossy()
                    .into_owned(),
            }
        }
        ManagedContextImportedDevelopment::FromSource {
            project_id,
            receipt,
        } => {
            let crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                project_id: expected_project_id,
                ..
            } = &entry.plan.development
            else {
                return Err(transfer_error(
                    "managed context import receipt contains unexpected development context",
                ));
            };
            if project_id != expected_project_id || receipt.project_id != *expected_project_id {
                return Err(transfer_error(
                    "managed context import receipt project does not match",
                ));
            }
            development_launch_target(project_id, receipt)?
        }
    };
    Ok(ManagedContextLaunchTarget {
        environment_id: entry.target_environment_id.clone(),
        kernel_id: entry.target_kernel_id.clone(),
        context_id: entry.plan.context_id.clone(),
        plan_digest: entry.plan.plan_digest.clone(),
        development,
    })
}

fn development_launch_target(
    project_id: &str,
    receipt: &crate::managed_context::development::DevelopmentContextPublicationReceipt,
) -> Result<crate::local::ManagedContextDevelopmentLaunchTarget, DaemonError> {
    let destination_root = receipt
        .destination_root
        .to_str()
        .ok_or_else(|| transfer_error("managed context destination is not UTF-8"))?
        .to_string();
    let repositories = receipt
        .repositories
        .iter()
        .map(|repository| {
            Ok(crate::local::ManagedContextRepositoryLaunchTarget {
                repository_id: repository.repository_id.clone(),
                role: repository.role,
                target_directory: repository.target_directory.clone(),
                workspace_path: repository
                    .destination_path
                    .to_str()
                    .ok_or_else(|| transfer_error("managed context Workspace path is not UTF-8"))?
                    .to_string(),
                head_sha: repository.head_sha.clone(),
            })
        })
        .collect::<Result<Vec<_>, DaemonError>>()?;
    Ok(
        crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
            project_id: project_id.to_string(),
            destination_root,
            primary_repository_id: receipt.primary_repository_id.clone(),
            repositories,
        },
    )
}

fn recover_launch_target_from_publication(
    transfer_root: &std::path::Path,
    recovery: &ManagedContextLaunchRecoveryBinding,
) -> Result<crate::local::ManagedContextLaunchTarget, DaemonError> {
    let development = match &recovery.plan.development {
        crate::managed_context::package::ManagedContextDevelopmentSelection::Empty => {
            let state_root = transfer_root
                .parent()
                .ok_or_else(|| transfer_error("managed context transfer root has no parent"))?;
            let workspace_path = state_root
                .join("managed-context-empty-workspaces")
                .join(sha256_bytes(recovery.plan.context_id.as_bytes()))
                .join("workspace")
                .to_string_lossy()
                .into_owned();
            crate::local::ManagedContextDevelopmentLaunchTarget::Empty { workspace_path }
        }
        crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        } => {
            let workspace_parent = transfer_root
                .parent()
                .ok_or_else(|| transfer_error("managed context transfer root has no parent"))?
                .join("managed-context-workspaces");
            let receipt = crate::managed_context::development::recover_pruned_development_context_publication(
                &workspace_parent,
                project_id,
                repositories,
            )?
            .ok_or_else(|| {
                transfer_error("schema-3 managed context publication could not be recovered")
            })?;
            development_launch_target(project_id, &receipt)?
        }
    };
    Ok(crate::local::ManagedContextLaunchTarget {
        environment_id: recovery.environment_id.clone(),
        kernel_id: recovery.kernel_id.clone(),
        context_id: recovery.plan.context_id.clone(),
        plan_digest: recovery.plan.plan_digest.clone(),
        development,
    })
}

fn compact_consumed_entries_for_state_capacity(
    state: &mut PersistedTransferState,
) -> Result<Vec<String>, DaemonError> {
    let mut removed = Vec::new();
    while ensure_state_capacity_with_receipt_reservations(state).is_err() {
        let Some(transfer_id) = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.phase == ManagedContextTransferPhase::Consumed)
            .min_by_key(|(_, entry)| entry.completed_at_ms.unwrap_or(0))
            .map(|(transfer_id, _)| transfer_id.clone())
        else {
            return Err(transfer_error(
                "managed context transfer state cannot fit the migrated launch targets",
            ));
        };
        state.entries.remove(&transfer_id);
        removed.push(transfer_id);
    }
    Ok(removed)
}

fn persisted_state_size(state: &PersistedTransferState) -> Result<usize, DaemonError> {
    serde_json::to_vec(state)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            transfer_error(format!(
                "serialize managed context transfer state for capacity check: {error}"
            ))
        })
}

fn ensure_state_capacity_with_receipt_reservations(
    state: &PersistedTransferState,
) -> Result<(), DaemonError> {
    let future_receipts = state
        .entries
        .values()
        .filter(|entry| {
            !matches!(
                entry.phase,
                ManagedContextTransferPhase::Consumed | ManagedContextTransferPhase::Failed
            )
        })
        .count()
        .saturating_mul(MAX_PERSISTED_IMPORT_BYTES);
    let required = persisted_state_size(state)?
        .saturating_add(future_receipts)
        .saturating_add(STATE_CAPACITY_MARGIN_BYTES);
    if required > MAX_STATE_FILE_BYTES as usize {
        return Err(transfer_error(
            "managed context transfer state has no capacity for another import receipt",
        ));
    }
    Ok(())
}

fn ensure_state_file_capacity(state: &PersistedTransferState) -> Result<(), DaemonError> {
    if persisted_state_size(state)? > MAX_STATE_FILE_BYTES as usize {
        return Err(transfer_error(
            "managed context transfer state exceeds its file capacity",
        ));
    }
    Ok(())
}

fn ready_import(
    archive_path: &std::path::Path,
    transfer_id: &str,
    entry: &PersistedTransfer,
) -> ReadyManagedContextImport {
    ReadyManagedContextImport {
        transfer_id: transfer_id.to_string(),
        archive_path: archive_path.to_path_buf(),
        plan: entry.plan.clone(),
        archive_sha256: entry.archive_sha256.clone(),
        destination_root: entry.destination_root.clone(),
        target_environment_id: entry.target_environment_id.clone(),
        target_kernel_id: entry.target_kernel_id.clone(),
        target_key_thumbprint: entry.target_key_thumbprint.clone(),
        source_kernel_id: entry.source_kernel_id.clone(),
        source_key_thumbprint: entry.source_key_thumbprint.clone(),
    }
}

#[cfg(test)]
mod tests;
