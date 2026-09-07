//! Bounded package uploads owned by the kernel on macOS/Linux.
//!
//! All methods receive an authenticated owner separately from the opaque
//! handle. They accept no client filesystem paths. Finalization checks transport
//! integrity only: the returned bytes still require package inspection and the
//! kernel's enrolled-publisher signature policy before staging or execution.

mod model;
mod storage;

pub use model::{
    UploadCheckpoint, UploadError, UploadLimits, UploadPhase, UploadStatus,
    MAX_UPLOAD_ARCHIVE_BYTES, MAX_UPLOAD_CHUNK_BYTES,
};

use crate::private_fs::Dir;
use model::{
    valid_digest, valid_handle, validate_owner, validate_request_id, DurableState, Entry, Result,
    StoreState, MAX_READERS,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

struct Inner {
    root: Dir,
    limits: UploadLimits,
    state: Mutex<StoreState>,
}

#[derive(Clone)]
pub struct PackageUploadStore {
    inner: Arc<Inner>,
}

/// A read-only descriptor anchored to the finalized bytes, not a verified App.
/// Keep this lease until inspection/verification/staging finishes. Trusted
/// kernel code must not retain cloned descriptors beyond the lease lifetime.
pub struct UploadedPackage {
    file: Option<File>,
    status: UploadStatus,
    inner: Arc<Inner>,
}

impl UploadedPackage {
    pub fn file(&self) -> &File {
        self.file.as_ref().expect("live upload lease has a file")
    }
    pub fn status(&self) -> &UploadStatus {
        &self.status
    }
}

impl Drop for UploadedPackage {
    fn drop(&mut self) {
        drop(self.file.take());
        if let Ok(mut state) = self.inner.state.lock() {
            state.leases.remove(&self.status.handle);
        }
    }
}

impl PackageUploadStore {
    /// The kernel provides an existing, dedicated mode-0700 absolute directory
    /// outside workspaces. Symlink ancestors are rejected. A process lease
    /// prevents two independent owners from mutating this store concurrently.
    pub fn open(root: &Path, limits: UploadLimits, now_ms: u64) -> Result<Self> {
        limits.validate()?;
        let root = Dir::open_private(root)?;
        if !root.try_lock()? {
            return Err(UploadError::Busy);
        }
        // A prior instance may have renamed metadata and failed directory
        // sync. Complete that durability boundary before reading or cleanup.
        root.sync()?;
        let durable = storage::initialize(&root, limits)?;
        let store = Self {
            inner: Arc::new(Inner {
                root,
                limits,
                state: Mutex::new(StoreState {
                    durable,
                    leases: BTreeSet::new(),
                    faulted: false,
                }),
            }),
        };
        {
            let mut state = store.lock()?;
            store.prune(&mut state, now_ms)?;
            storage::recover(&store.inner.root, &state.durable)?;
        }
        Ok(store)
    }

    /// The client supplies a stable opaque request ID. Retries with the same
    /// owner, size and digest return the existing handle and original expiry,
    /// including an aborted receipt. Reusing the ID with different bytes fails.
    /// The kernel assigns expiry on the first attempt; retries never extend it.
    pub fn begin(
        &self,
        owner: &str,
        request_id: &str,
        expected_size: u64,
        sha256: &str,
        expires_at_ms: u64,
        now_ms: u64,
    ) -> Result<UploadStatus> {
        validate_owner(owner)?;
        validate_request_id(request_id)?;
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)?;
        if let Some((handle, entry)) = state
            .durable
            .uploads
            .iter()
            .find(|(_, entry)| entry.owner == owner && entry.request_id == request_id)
        {
            if entry.expected_size != expected_size || entry.sha256 != sha256 {
                return Err(UploadError::Conflict);
            }
            // An expired record can remain while its verifier lease is live.
            // Do not create a second handle under that in-use request identity.
            if entry.expires_at_ms <= now_ms {
                return Err(UploadError::Busy);
            }
            return Ok(entry.status(handle));
        }
        if expected_size == 0
            || expected_size > MAX_UPLOAD_ARCHIVE_BYTES
            || !valid_digest(sha256)
            || expires_at_ms <= now_ms
            || expires_at_ms - now_ms > self.inner.limits.max_ttl_ms
        {
            return Err(UploadError::Invalid("upload declaration"));
        }
        self.admit(&state, owner, expected_size)?;
        let handle = storage::new_handle()?;
        let name = storage::archive_name(&handle);
        let file = self.inner.root.create_private_file(OsStr::new(&name))?;
        file.sync_all()?;
        self.inner.root.sync()?;
        let entry = Entry {
            owner: owner.into(),
            request_id: request_id.into(),
            expected_size,
            sha256: sha256.into(),
            accepted_bytes: 0,
            expires_at_ms,
            phase: UploadPhase::Receiving,
        };
        let status = entry.status(&handle);
        let mut next = state.durable.clone();
        next.uploads.insert(handle, entry);
        self.commit(&mut state, next, &mut |_| Ok(()))?;
        Ok(status)
    }

    pub fn chunk(
        &self,
        owner: &str,
        handle: &str,
        offset: u64,
        bytes: &[u8],
        chunk_sha256: &str,
        now_ms: u64,
    ) -> Result<UploadStatus> {
        self.chunk_with_checkpoint(owner, handle, offset, bytes, chunk_sha256, now_ms, |_| {
            Ok(())
        })
    }

    /// Interruption callbacks execute only trusted kernel code. An error after
    /// archive sync may leave an uncommitted suffix; retry/reopen truncates it.
    pub fn chunk_with_checkpoint(
        &self,
        owner: &str,
        handle: &str,
        offset: u64,
        bytes: &[u8],
        chunk_sha256: &str,
        now_ms: u64,
        mut checkpoint: impl FnMut(UploadCheckpoint) -> Result<()>,
    ) -> Result<UploadStatus> {
        validate_owner(owner)?;
        if bytes.is_empty() || bytes.len() > MAX_UPLOAD_CHUNK_BYTES {
            return Err(UploadError::Limit);
        }
        if !valid_digest(chunk_sha256)
            || format!("sha256:{:x}", Sha256::digest(bytes)) != chunk_sha256
        {
            return Err(UploadError::DigestMismatch);
        }
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)?;
        let entry = self.entry(&state, owner, handle, now_ms)?.clone();
        if entry.phase != UploadPhase::Receiving {
            return Err(UploadError::Conflict);
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .filter(|end| *end <= entry.expected_size)
            .ok_or(UploadError::Conflict)?;
        let name = storage::archive_name(handle);
        let mut file = self.inner.root.open_private_file(OsStr::new(&name))?;
        storage::reconcile_length(&file, entry.accepted_bytes)?;
        if offset < entry.accepted_bytes {
            if end > entry.accepted_bytes {
                return Err(UploadError::Conflict);
            }
            let mut previous = vec![0; bytes.len()];
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut previous)?;
            if previous != bytes {
                return Err(UploadError::Conflict);
            }
            return Ok(entry.status(handle));
        }
        if offset != entry.accepted_bytes {
            return Err(UploadError::Conflict);
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        checkpoint(UploadCheckpoint::ArchiveSynced)?;
        let mut next = state.durable.clone();
        let updated = next
            .uploads
            .get_mut(handle)
            .ok_or(UploadError::CorruptState)?;
        updated.accepted_bytes = end;
        let status = updated.status(handle);
        self.commit(&mut state, next, &mut checkpoint)?;
        Ok(status)
    }

    pub fn status(&self, owner: &str, handle: &str, now_ms: u64) -> Result<UploadStatus> {
        validate_owner(owner)?;
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)?;
        Ok(self.entry(&state, owner, handle, now_ms)?.status(handle))
    }

    pub fn finalize(&self, owner: &str, handle: &str, now_ms: u64) -> Result<UploadedPackage> {
        validate_owner(owner)?;
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)?;
        let entry = self.entry(&state, owner, handle, now_ms)?.clone();
        if entry.phase == UploadPhase::Aborted {
            return Err(UploadError::Conflict);
        }
        if entry.accepted_bytes != entry.expected_size {
            return Err(UploadError::Incomplete);
        }
        if state.leases.contains(handle) || state.leases.len() >= MAX_READERS {
            return Err(UploadError::Busy);
        }
        let name = storage::archive_name(handle);
        let mut file = self.inner.root.read_file(OsStr::new(&name), false)?;
        if file.metadata()?.len() != entry.expected_size {
            return Err(UploadError::CorruptState);
        }
        if storage::file_digest(&file, entry.expected_size)? != entry.sha256 {
            return Err(UploadError::DigestMismatch);
        }
        file.seek(SeekFrom::Start(0))?;
        let mut next = state.durable.clone();
        let updated = next
            .uploads
            .get_mut(handle)
            .ok_or(UploadError::CorruptState)?;
        updated.phase = UploadPhase::Finalized;
        let status = updated.status(handle);
        if entry.phase != UploadPhase::Finalized {
            self.commit(&mut state, next, &mut |_| Ok(()))?;
        }
        // The durable finalized phase blocks all writers before permissions
        // change. Recovery finishes sealing if interrupted at this boundary.
        storage::seal(&file)?;
        state.leases.insert(handle.into());
        Ok(UploadedPackage {
            file: Some(file),
            status,
            inner: self.inner.clone(),
        })
    }

    pub fn abort(&self, owner: &str, handle: &str, now_ms: u64) -> Result<()> {
        validate_owner(owner)?;
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)?;
        if self.entry(&state, owner, handle, now_ms)?.phase == UploadPhase::Aborted {
            return Ok(());
        }
        if state.leases.contains(handle) {
            return Err(UploadError::Busy);
        }
        let mut next = state.durable.clone();
        next.uploads
            .get_mut(handle)
            .ok_or(UploadError::CorruptState)?
            .phase = UploadPhase::Aborted;
        self.commit(&mut state, next, &mut |_| Ok(()))?;
        storage::cleanup(&self.inner.root, &state.durable)
    }

    /// Invoke from the kernel's bounded maintenance tick as well as on startup.
    /// A live verifier lease keeps its reservation even after upload expiry.
    pub fn cleanup_expired(&self, now_ms: u64) -> Result<()> {
        let mut state = self.lock()?;
        self.prune(&mut state, now_ms)
    }

    fn lock(&self) -> Result<MutexGuard<'_, StoreState>> {
        let state = self.inner.state.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => UploadError::Busy,
            TryLockError::Poisoned(_) => UploadError::Unavailable,
        })?;
        if state.faulted {
            return Err(UploadError::Unavailable);
        }
        Ok(state)
    }

    fn entry<'a>(
        &self,
        state: &'a StoreState,
        owner: &str,
        handle: &str,
        now_ms: u64,
    ) -> Result<&'a Entry> {
        if !valid_handle(handle) {
            return Err(UploadError::NotFound);
        }
        state
            .durable
            .uploads
            .get(handle)
            .filter(|entry| entry.owner == owner && entry.expires_at_ms > now_ms)
            .ok_or(UploadError::NotFound)
    }

    fn admit(&self, state: &StoreState, owner: &str, bytes: u64) -> Result<()> {
        let limits = self.inner.limits;
        let own: Vec<_> = state
            .durable
            .uploads
            .values()
            .filter(|entry| entry.owner == owner)
            .collect();
        let total: u64 = state
            .durable
            .uploads
            .values()
            .map(Entry::reserved_bytes)
            .sum();
        let own_total: u64 = own.iter().map(|entry| entry.reserved_bytes()).sum();
        if state.durable.uploads.len() >= limits.max_uploads
            || own.len() >= limits.max_uploads_per_owner
            || bytes > limits.max_reserved_bytes.saturating_sub(total)
            || bytes
                > limits
                    .max_reserved_bytes_per_owner
                    .saturating_sub(own_total)
        {
            return Err(UploadError::Limit);
        }
        Ok(())
    }

    fn prune(&self, state: &mut StoreState, now_ms: u64) -> Result<()> {
        let mut next = state.durable.clone();
        next.uploads
            .retain(|handle, entry| entry.expires_at_ms > now_ms || state.leases.contains(handle));
        if next.uploads.len() != state.durable.uploads.len() {
            self.commit(state, next, &mut |_| Ok(()))?;
        }
        storage::cleanup(&self.inner.root, &state.durable)
    }

    fn commit(
        &self,
        state: &mut StoreState,
        next: DurableState,
        checkpoint: &mut impl FnMut(UploadCheckpoint) -> Result<()>,
    ) -> Result<()> {
        let result = (|| {
            checkpoint(UploadCheckpoint::BeforeStateCommit)?;
            storage::write_state_with_checkpoint(&self.inner.root, &next, checkpoint)?;
            checkpoint(UploadCheckpoint::StateCommitted)
        })();
        if let Err(error) = result {
            // Visible replacement is not proof of durability. Complete the
            // directory sync before accepting reloaded offsets or allowing any
            // cleanup to act on a possibly uncommitted abort/expiry record.
            let recovered = (|| {
                checkpoint(UploadCheckpoint::BeforeRecoverySync)?;
                self.inner.root.sync()?;
                checkpoint(UploadCheckpoint::RecoverySynced)?;
                storage::read_state(&self.inner.root, self.inner.limits)
            })();
            match recovered {
                Ok(Some(durable)) => state.durable = durable,
                _ => state.faulted = true,
            }
            return Err(error);
        }
        state.durable = next;
        Ok(())
    }
}
