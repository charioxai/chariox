//! MP-08 / MP-10 / MP-11: merge only the requested native Claude config entry.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use fs2::FileExt;
use rand::RngCore;
use serde_json::value::RawValue;

use crate::error::DaemonError;

pub(super) type Object = BTreeMap<String, Box<RawValue>>;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const OPERATION: &str = "merge Claude native config state";

fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message: message.into(),
    }
}

fn safe_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    options
}

fn regular_owned(file: &File) -> Result<(), DaemonError> {
    let metadata = file.metadata().map_err(|e| error(e.to_string()))?;
    if !metadata.is_file() {
        return Err(error("Claude config state is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(error("Claude config state belongs to another user"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(error("Claude config state is not a regular file"));
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut options = safe_options();
    options.read(true);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            #[cfg(unix)]
            if e.raw_os_error() == Some(libc::ELOOP) {
                return Err(error("Claude config state is not a regular file"));
            }
            return Err(error(e.to_string()));
        }
    };
    regular_owned(&file)?;
    if file.metadata().map_err(|e| error(e.to_string()))?.len() > MAX_CONFIG_BYTES {
        return Err(error("Claude config state exceeds its size limit"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Claude config state could not be read"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(error("Claude config state exceeds its size limit"));
    }
    Ok(Some(bytes))
}

pub(super) fn object(raw: &RawValue) -> Result<Object, DaemonError> {
    serde_json::from_str(raw.get()).map_err(|_| error("Claude config entry is not a JSON object"))
}

pub(super) fn raw<T: serde::Serialize>(value: &T) -> Result<Box<RawValue>, DaemonError> {
    serde_json::value::to_raw_value(value)
        .map_err(|_| error("Claude config entry could not be encoded"))
}

fn prepare(
    source: Option<&[u8]>,
    mutate: &impl Fn(&mut Object) -> Result<bool, DaemonError>,
) -> Result<Option<Vec<u8>>, DaemonError> {
    // Keep unknown values opaque: parsing the whole file as Value would round
    // provider-owned numbers outside u64 and needlessly decode credential data.
    let mut state = match source {
        Some(bytes) if !bytes.iter().all(u8::is_ascii_whitespace) => serde_json::from_slice(bytes)
            .map_err(|_| error("Claude config state is invalid JSON or is not a JSON object"))?,
        _ => Object::new(),
    };
    if !mutate(&mut state)? {
        return Ok(None);
    }
    let mut bytes = serde_json::to_vec(&state)
        .map_err(|_| error("Claude config state could not be encoded"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(error("Claude config state exceeds its size limit"));
    }
    Ok(Some(bytes))
}

pub(super) fn merge(
    path: &Path,
    mutate: impl Fn(&mut Object) -> Result<bool, DaemonError>,
    mut before_commit: impl FnMut(),
) -> Result<(), DaemonError> {
    if prepare(read(path)?.as_deref(), &mutate)?.is_none() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| error("Claude config directory is unavailable"))?;
    fs::create_dir_all(parent).map_err(|e| error(e.to_string()))?;
    // Serialize Chariox writers across kernels. Native Claude does not take this
    // lock, so every publication must also compare against a fresh native read.
    let mut lock_options = safe_options();
    lock_options.read(true).write(true).create(true);
    let lock = lock_options
        .open(parent.join(".chariox-claude-config.lock"))
        .map_err(|e| error(e.to_string()))?;
    regular_owned(&lock)?;
    lock.lock_exclusive().map_err(|e| error(e.to_string()))?;
    for _ in 0..8 {
        let source = read(path)?;
        let Some(bytes) = prepare(source.as_deref(), &mutate)? else {
            return Ok(());
        };
        let mut nonce = [0_u8; 8];
        rand::thread_rng().fill_bytes(&mut nonce);
        let pending_path = parent.join(format!(
            ".chariox-claude-config-{}-{}.tmp",
            std::process::id(),
            u64::from_le_bytes(nonce)
        ));
        let mut options = safe_options();
        options.write(true).create_new(true);
        let mut pending = options
            .open(&pending_path)
            .map_err(|e| error(e.to_string()))?;
        let result = (|| {
            pending
                .write_all(&bytes)
                .and_then(|()| pending.sync_all())
                .map_err(|e| error(e.to_string()))?;
            drop(pending);
            before_commit();
            if read(path)? != source {
                return Ok(false);
            }
            if source.is_some() {
                fs::rename(&pending_path, path).map_err(|e| error(e.to_string()))?;
            } else {
                match fs::hard_link(&pending_path, path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
                    Err(e) => return Err(error(e.to_string())),
                }
            }
            #[cfg(unix)]
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|e| error(e.to_string()))?;
            Ok(true)
        })();
        let _ = fs::remove_file(&pending_path);
        if result? {
            return Ok(());
        }
    }
    Err(error(
        "Claude config kept changing; trust must use the native dialog",
    ))
}
