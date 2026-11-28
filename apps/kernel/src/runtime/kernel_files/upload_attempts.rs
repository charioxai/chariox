//! An upload ID belongs to an attempt, rather than to every upload of one path.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use sha2::{Digest, Sha256};
use uuid::Uuid;

struct Started {
    id: Uuid,
    revision: String,
    active_at: Instant,
}

static STARTED: LazyLock<Mutex<HashMap<PathBuf, Started>>> = LazyLock::new(Default::default);
const CAP: usize = 4096;

/// Stat can identify a resumable partial after a kernel restart. The version includes its
/// device, inode, length and nanosecond modification/change times; an inode alone can be reused.
pub(super) fn resumable_id(metadata: &std::fs::Metadata) -> Uuid {
    let digest = Sha256::digest(crate::agent_file_copy::revision(metadata).as_bytes());
    Uuid::from_bytes(
        digest[..16]
            .try_into()
            .expect("a SHA-256 holds sixteen bytes"),
    )
}

pub(super) fn known(path: &Path, id: Uuid) -> bool {
    STARTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .is_some_and(|a| a.id == id)
}

pub(super) fn matches(path: &Path, id: Uuid, metadata: &std::fs::Metadata) -> bool {
    let revision = crate::agent_file_copy::revision(metadata);
    STARTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .is_some_and(|a| a.id == id && a.revision == revision)
        || id == resumable_id(metadata)
}

pub(super) fn begin(path: &Path, id: Uuid, metadata: &std::fs::Metadata) -> bool {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if !is_current(path, metadata) {
        return false;
    }
    insert(&mut started, path, id, metadata);
    true
}

pub(super) fn claim(path: &Path, id: Uuid, metadata: &std::fs::Metadata) -> bool {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    let revision = crate::agent_file_copy::revision(metadata);
    let known = started
        .get(path)
        .is_some_and(|a| a.id == id && a.revision == revision);
    if !is_current(path, metadata) || (!known && id != resumable_id(metadata)) {
        return false;
    }
    insert(&mut started, path, id, metadata);
    true
}

fn is_current(path: &Path, metadata: &std::fs::Metadata) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|current| {
        crate::agent_file_copy::revision(&current) == crate::agent_file_copy::revision(metadata)
    })
}

/// Called only after the opened staging file passed the attempt and file checks.
/// A late writer must not replace the record of a newer offset-zero opener.
pub(super) fn progress(path: &Path, id: Uuid, metadata: &std::fs::Metadata) -> bool {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if !is_current(path, metadata) || started.get(path).is_some_and(|a| a.id != id) {
        return false;
    }
    insert(&mut started, path, id, metadata);
    true
}

fn insert(
    started: &mut HashMap<PathBuf, Started>,
    path: &Path,
    id: Uuid,
    metadata: &std::fs::Metadata,
) {
    if !started.contains_key(path) && started.len() >= CAP {
        if let Some(oldest) = started
            .iter()
            .min_by_key(|(_, a)| a.active_at)
            .map(|(path, _)| path.clone())
        {
            started.remove(&oldest);
        }
    }
    started.insert(
        path.to_path_buf(),
        Started {
            id,
            revision: crate::agent_file_copy::revision(metadata),
            active_at: Instant::now(),
        },
    );
}

pub(super) fn forget(path: &Path, id: Option<Uuid>) {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if id.is_none() || started.get(path).is_some_and(|a| Some(a.id) == id) {
        started.remove(path);
    }
}
