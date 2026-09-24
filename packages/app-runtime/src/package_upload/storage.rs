use super::model::*;
use crate::private_fs::{is_atomic_temporary, try_lock_file, Dir, FsError};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;

const STATE_FILE: &str = "uploads.json";
const MAX_STATE_BYTES: u64 = 128 * 1024;
const MAX_ROOT_ENTRIES: usize = MAX_UPLOADS * 2 + 8;

pub(super) fn read_state(root: &Dir, limits: UploadLimits) -> Result<Option<DurableState>> {
    let file = match root.read_file(OsStr::new(STATE_FILE), false) {
        Ok(file) => file,
        Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_STATE_BYTES {
        return Err(UploadError::CorruptState);
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(UploadError::CorruptState);
    }
    let state: DurableState =
        serde_json::from_slice(&bytes).map_err(|_| UploadError::CorruptState)?;
    validate_state(&state, limits)?;
    Ok(Some(state))
}

pub(super) fn write_state(root: &Dir, state: &DurableState) -> Result<()> {
    root.atomic_replace(OsStr::new(STATE_FILE), &state_bytes(state)?)?;
    Ok(())
}

pub(super) fn write_state_with_checkpoint(
    root: &Dir,
    state: &DurableState,
    checkpoint: &mut impl FnMut(UploadCheckpoint) -> Result<()>,
) -> Result<()> {
    root.atomic_replace_with_checkpoint(OsStr::new(STATE_FILE), &state_bytes(state)?, || {
        checkpoint(UploadCheckpoint::StateRenamed).map_err(std::io::Error::other)
    })?;
    Ok(())
}

fn state_bytes(state: &DurableState) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(state).map_err(|_| UploadError::CorruptState)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(UploadError::Limit);
    }
    Ok(bytes)
}

pub(super) fn initialize(root: &Dir, limits: UploadLimits) -> Result<DurableState> {
    if let Some(state) = read_state(root, limits)? {
        return Ok(state);
    }
    // An interrupted initial metadata write may leave only its temporary.
    // Archives without any durable state are corruption, not an empty store.
    for name in root.entries(MAX_ROOT_ENTRIES)? {
        if !is_atomic_temporary(&name) {
            return Err(UploadError::CorruptState);
        }
    }
    let state = DurableState::default();
    write_state(root, &state)?;
    Ok(state)
}

pub(super) fn archive_name(handle: &str) -> String {
    format!("{handle}.cxapp")
}

/// Run under the store's exclusive root lease and mutation mutex. Durable abort
/// or expiry precedes archive unlink; receipts remain bounded until expiry.
pub(super) fn cleanup(root: &Dir, state: &DurableState) -> Result<()> {
    let mut changed = false;
    for name in root.entries(MAX_ROOT_ENTRIES)? {
        if name == OsStr::new(STATE_FILE) {
            continue;
        }
        if is_atomic_temporary(&name) {
            let file = root.read_file(&name, false)?;
            if !try_lock_file(&file)? {
                return Err(UploadError::Busy);
            }
            root.remove_file(&name)?;
            changed = true;
            continue;
        }
        let handle = name
            .to_str()
            .and_then(|name| name.strip_suffix(".cxapp"))
            .filter(|handle| valid_handle(handle))
            .ok_or(UploadError::UnsafeEntry)?;
        if state
            .uploads
            .get(handle)
            .is_none_or(|entry| entry.phase == UploadPhase::Aborted)
        {
            root.remove_file(&name)?;
            changed = true;
        }
    }
    if changed {
        root.sync()?;
    }
    Ok(())
}

pub(super) fn recover(root: &Dir, state: &DurableState) -> Result<()> {
    cleanup(root, state)?;
    for (handle, entry) in &state.uploads {
        let name = archive_name(handle);
        match entry.phase {
            UploadPhase::Receiving => {
                let file = root.open_private_file(OsStr::new(&name))?;
                reconcile_length(&file, entry.accepted_bytes)?;
            }
            UploadPhase::Finalized => {
                let file = root.read_file(OsStr::new(&name), false)?;
                if file.metadata()?.len() != entry.expected_size {
                    return Err(UploadError::CorruptState);
                }
                seal(&file)?;
            }
            UploadPhase::Aborted => {}
        }
    }
    Ok(())
}

pub(super) fn reconcile_length(file: &File, accepted: u64) -> Result<()> {
    let length = file.metadata()?.len();
    if length < accepted {
        return Err(UploadError::CorruptState);
    }
    if length > accepted {
        file.set_len(accepted)?;
        file.sync_all()?;
    }
    Ok(())
}

pub(super) fn seal(file: &File) -> Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(0o400))?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn file_digest(file: &File, expected_size: u64) -> Result<String> {
    let mut source = file.take(expected_size + 1);
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length += read as u64;
        digest.update(&buffer[..read]);
    }
    if length != expected_size {
        return Err(UploadError::CorruptState);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

pub(super) fn new_handle() -> Result<String> {
    let mut bytes = [0_u8; 32];
    if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(format!(
        "upload_{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}
