//! Named saved-tree restore. The lifecycle gate drains all App code; the sole
//! SQLite writer fences identity and commits state with a restore receipt.
//! A durable prior tree and pinned target precede deletion. Recovery uses the
//! receipt to choose prior (no commit) or target (commit), before App spawn.
use super::app_snapshot_broker::LIMITS;
use crate::{durable_state::DurableKernelStateStore, local::AppRequestErrorCode as Error};
use chariox_app_runtime::{
    app_outbox::EventCatalog,
    worker_process::{PrivateData, TreeCopy},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    snapshot_id: String,
    name: String,
    consistency: String,
    installation_id: String,
    generation: u64,
    package_digest: String,
    owner_id: String,
    data_schema: u32,
    created_at_ms: u64,
    files: Vec<File>,
    bytes: u64,
    skipped: u64,
    state_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: String,
    owner: String,
    installation: String,
    generation: u64,
    digest: String,
    snapshot: String,
    restore_id: String,
    prior: Vec<File>,
    target: Vec<File>,
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.starts_with("snapshot-")
        && id.len() <= 96
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn inventory(tree: TreeCopy) -> Result<Vec<File>> {
    if tree.skipped != 0 {
        return Err(Error::DigestMismatch);
    }
    Ok(tree
        .files
        .into_iter()
        .map(|f| File {
            path: f.path,
            bytes: f.bytes,
            sha256: f.sha256,
        })
        .collect())
}
fn check_tree(path: &Path, files: &[File]) -> Result<()> {
    let actual = inventory(
        PrivateData::copy_saved_tree(path, None, LIMITS).map_err(|_| Error::StorageUnavailable)?,
    )?;
    if actual != files {
        return Err(Error::DigestMismatch);
    }
    Ok(())
}
pub(crate) fn validate(
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
    generation: u64,
    snapshot: &str,
) -> Result<(PathBuf, Value)> {
    if !valid_id(snapshot) {
        return Err(Error::InvalidRequest);
    }
    let active = store
        .get_app_installation(owner, installation)
        .map_err(super::app_control::registry_error)?;
    if active.generation != generation || active.admission_paused {
        return Err(Error::Conflict);
    }
    let active = active.active.ok_or(Error::Conflict)?;
    let root = super::app_snapshot_broker::root(store, installation)
        .map_err(|_| Error::StorageUnavailable)?;
    let path = root.join(snapshot);
    if !path.try_exists().map_err(|_| Error::StorageUnavailable)? {
        return Err(Error::NotFound);
    }
    require_dir(&path)?;
    let manifest: Manifest = read(&path.join("manifest.json"), 4 * 1024 * 1024)?;
    if manifest.schema != "chariox.app-snapshot.v2"
        || manifest.snapshot_id != snapshot
        || manifest.installation_id != installation
        || manifest.owner_id != owner
        || manifest.generation != generation
        || manifest.package_digest != active.release.package_digest
        || manifest.data_schema != active.release.schema_version
        || !matches!(
            manifest.consistency.as_str(),
            "quiescent" | "crash_consistent"
        )
        || manifest.name.is_empty()
        || manifest.created_at_ms == 0
        || manifest.skipped != 0
        || manifest.bytes
            != manifest
                .files
                .iter()
                .try_fold(0u64, |n, f| n.checked_add(f.bytes))
                .ok_or(Error::LimitExceeded)?
    {
        return Err(Error::Conflict);
    }
    check_tree(&path.join("files"), &manifest.files)?;
    let state: Value = read(&path.join("state.json"), 20 * 1024 * 1024)?;
    {
        use sha2::{Digest, Sha256};
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&state).map_err(|_| Error::Conflict)?)
        );
        if manifest.state_sha256 != digest {
            return Err(Error::DigestMismatch);
        }
    }
    crate::durable_state::app_snapshot_restore::validate_state(&state)?;
    reserve(store)?;
    Ok((path, state))
}

pub(crate) fn restore(
    store: &DurableKernelStateStore,
    owner: &str,
    catalog: Arc<EventCatalog>,
    data: &PrivateData,
    snapshot: &str,
) -> Result<()> {
    restore_inner(
        store,
        owner,
        catalog,
        data,
        snapshot,
        #[cfg(test)]
        None,
    )
}
fn restore_inner(
    store: &DurableKernelStateStore,
    owner: &str,
    catalog: Arc<EventCatalog>,
    data: &PrivateData,
    snapshot: &str,
    #[cfg(test)] fault: Option<crate::durable_state::app_snapshot_restore::RestoreFault>,
) -> Result<()> {
    if data.installation_id() != catalog.installation_id()
        || data.generation() != catalog.generation()
        || data.release_digest() != catalog.app_catalog().package_digest()
    {
        return Err(Error::Conflict);
    }
    recover(store, owner, data)?;
    let (source, state) = validate(
        store,
        owner,
        catalog.installation_id(),
        catalog.generation(),
        snapshot,
    )?;
    let parent = journal_root(store)?;
    let directory = parent.join(data.installation_id());
    let staging = parent.join(format!(".tmp-{}", data.installation_id()));
    // No destructive write precedes the durable journal publication.
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(io)?;
    }
    private_dir(&staging)?;
    let staged = (|| {
        private_dir(&staging.join("prior"))?;
        private_dir(&staging.join("target"))?;
        let prior = inventory(
            data.copy_tree(&staging.join("prior"), LIMITS, &mut || true)
                .map_err(|_| Error::StorageUnavailable)?,
        )?;
        let target = inventory(
            PrivateData::copy_saved_tree(
                &source.join("files"),
                Some(&staging.join("target")),
                LIMITS,
            )
            .map_err(|_| Error::StorageUnavailable)?,
        )?;
        // Re-read validates the immutable copy against the saved inventory.
        let manifest: Manifest = read(&source.join("manifest.json"), 4 * 1024 * 1024)?;
        if target != manifest.files {
            return Err(Error::DigestMismatch);
        }
        let journal = Journal {
            schema: "chariox.app-restore.v1".into(),
            owner: owner.into(),
            installation: data.installation_id().into(),
            generation: data.generation(),
            digest: data.release_digest().into(),
            snapshot: snapshot.into(),
            restore_id: format!("restore-{:032x}", rand::random::<u128>()),
            prior,
            target,
        };
        write(&staging.join("journal.json"), &journal)?;
        write(&staging.join("journal-sha256.json"), &checksum(&journal)?)?;
        sync(&staging)?;
        std::fs::rename(&staging, &directory).map_err(io)?;
        sync(&parent)?;
        Ok(())
    })();
    if staged.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    staged?;
    let journal: Journal = read_journal(&directory)?;
    let result = store.commit_app_snapshot_restore(
        owner,
        catalog,
        journal.restore_id,
        state,
        #[cfg(test)]
        fault,
    );
    // This writer response is definitive only after reconciliation. Unknown
    // outcomes retain the journal and fail closed until a fresh kernel opens.
    recover(store, owner, data)?;
    result
}

pub(crate) fn require_recovery_generation(
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
    generation: u64,
) -> Result<()> {
    let parent = journal_root(store)?;
    clean_retired(&parent, installation)?;
    let directory = parent.join(installation);
    if !directory.try_exists().map_err(io)? {
        return Ok(());
    }
    let journal: Journal = read_journal(&directory)?;
    if journal.owner != owner
        || journal.installation != installation
        || journal.generation != generation
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
pub(crate) fn recover(
    store: &DurableKernelStateStore,
    owner: &str,
    data: &PrivateData,
) -> Result<()> {
    recover_inner(store, owner, data, &mut || true)
}
fn recover_inner(
    store: &DurableKernelStateStore,
    owner: &str,
    data: &PrivateData,
    proceed: &mut dyn FnMut() -> bool,
) -> Result<()> {
    let parent = journal_root(store)?;
    clean_retired(&parent, data.installation_id())?;
    let staging = parent.join(format!(".tmp-{}", data.installation_id()));
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(io)?;
        sync(&parent)?;
    }
    let directory = parent.join(data.installation_id());
    if !directory.try_exists().map_err(io)? {
        return Ok(());
    }
    require_dir(&directory)?;
    let journal: Journal = read_journal(&directory)?;
    if journal.schema != "chariox.app-restore.v1"
        || journal.owner != owner
        || journal.installation != data.installation_id()
        || journal.generation != data.generation()
        || journal.digest != data.release_digest()
        || !valid_id(&journal.snapshot)
    {
        return Err(Error::Conflict);
    }
    let committed =
        store.app_snapshot_restore_committed(owner, data.installation_id(), &journal.restore_id)?;
    let (name, files) = if committed {
        ("target", &journal.target)
    } else {
        ("prior", &journal.prior)
    };
    let source = directory.join(name);
    check_tree(&source, files)?;
    data.restore_tree_while(&source, LIMITS, proceed)
        .map_err(|_| Error::StorageUnavailable)?;
    // Once the decision tree is durable, atomically retire the journal before
    // admitting code. Never delete an active journal piecemeal.
    let done = parent.join(format!(".done-{}", data.installation_id()));
    if done.exists() {
        std::fs::remove_dir_all(&done).map_err(io)?;
    }
    std::fs::rename(&directory, &done).map_err(io)?;
    sync(&parent)?;
    std::fs::remove_dir_all(done).map_err(io)?;
    sync(&parent)
}
fn checksum(value: &impl Serialize) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| Error::StorageUnavailable)?)
    ))
}
fn read_journal(directory: &Path) -> Result<Journal> {
    let journal: Journal = read(&directory.join("journal.json"), 8 * 1024 * 1024)?;
    let expected: String = read(&directory.join("journal-sha256.json"), 128)?;
    if expected != checksum(&journal)? {
        return Err(Error::DigestMismatch);
    }
    Ok(journal)
}
fn clean_retired(parent: &Path, installation: &str) -> Result<()> {
    let done = parent.join(format!(".done-{installation}"));
    if done.try_exists().map_err(io)? {
        // A previous parent fsync or cleanup may have failed after rename. Make
        // retirement durable before admitting any new writes or generation.
        sync(parent)?;
        std::fs::remove_dir_all(done).map_err(io)?;
        sync(parent)?;
    }
    Ok(())
}
fn journal_root(store: &DurableKernelStateStore) -> Result<PathBuf> {
    let root = store
        .path()
        .parent()
        .ok_or(Error::StorageUnavailable)?
        .join("app-restores");
    private_dir(&root)?;
    std::fs::canonicalize(root).map_err(io)
}
fn require_dir(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::StorageUnavailable);
    }
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(io(e)),
    }
    require_dir(path)
}
fn read<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> Result<T> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.len() > max {
        return Err(Error::LimitExceeded);
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > max {
        return Err(Error::LimitExceeded);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Conflict)
}
fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io)?;
    file.write_all(&serde_json::to_vec(value).map_err(|_| Error::StorageUnavailable)?)
        .map_err(io)?;
    file.sync_all().map_err(io)
}
fn sync(path: &Path) -> Result<()> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io)
}
fn io(_: std::io::Error) -> Error {
    Error::StorageUnavailable
}

#[cfg(test)]
pub(crate) fn fixture_restore(
    store: &DurableKernelStateStore,
    owner: &str,
    catalog: Arc<EventCatalog>,
    data: &PrivateData,
    snapshot: &str,
    fault: crate::durable_state::app_snapshot_restore::RestoreFault,
) -> Result<()> {
    restore_inner(store, owner, catalog, data, snapshot, Some(fault))
}
#[cfg(test)]
pub(crate) fn fixture_interrupt_replay(
    store: &DurableKernelStateStore,
    owner: &str,
    data: &PrivateData,
) -> Result<()> {
    recover_inner(store, owner, data, &mut || false)
}

fn reserve(store: &DurableKernelStateStore) -> Result<()> {
    let root = journal_root(store)?;
    let free = super::app_snapshot_broker::free_bytes(&root);
    #[cfg(test)]
    let free = FIXTURE_FREE_BYTES.with(|value| value.get()).or(free);
    if free.is_none_or(|free| free < 2 * LIMITS.bytes + 2 * 1024 * 1024 * 1024) {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}
#[cfg(test)]
std::thread_local! { static FIXTURE_FREE_BYTES: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(crate) fn fixture_with_free_bytes<T>(bytes: u64, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<u64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FIXTURE_FREE_BYTES.with(|v| v.set(self.0));
        }
    }
    let _reset = Reset(FIXTURE_FREE_BYTES.with(|v| v.replace(Some(bytes))));
    action()
}

pub(crate) fn pending(store: &DurableKernelStateStore, installation: &str) -> Result<bool> {
    journal_root(store)?
        .join(installation)
        .try_exists()
        .map_err(io)
}
/// Called under the lifecycle gate after authoritative uninstall/data deletion.
/// An inactive installation can no longer adopt this restore's authority/data.
pub(crate) fn retire_uninstalled(
    store: &DurableKernelStateStore,
    owner: &str,
    installation: &str,
    generation: u64,
) -> Result<()> {
    let current = store
        .get_app_installation(owner, installation)
        .map_err(super::app_control::registry_error)?;
    if current.active.is_some() || current.generation != generation {
        return Err(Error::Conflict);
    }
    let parent = journal_root(store)?;
    for name in [
        installation.to_owned(),
        format!(".tmp-{installation}"),
        format!(".done-{installation}"),
    ] {
        let path = parent.join(name);
        if path.try_exists().map_err(io)? {
            require_dir(&path)?;
            std::fs::remove_dir_all(path).map_err(io)?;
        }
    }
    sync(&parent)
}
