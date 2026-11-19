//! Claude's own state file (`.claude.json`): read as Claude left it, one update
//! merged in, replaced atomically while Claude cannot be saving at the same time.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rand::RngCore;

use super::project_trust::{ClaudeStateUpdate, StateChange};
use crate::error::DaemonError;

const OPERATION: &str = "initialize Claude headless onboarding state";
const MAX_STATE_BYTES: u64 = 1024 * 1024;
/// Claude saves its state under a `<file>.lock` directory (proper-lockfile
/// convention); holding it keeps this update from landing between Claude's read
/// and its write.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(10);
const LOCK_WAIT: Duration = Duration::from_secs(5);
const COMMIT_ATTEMPTS: usize = 8;

fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message: message.into(),
    }
}

fn not_regular(state_path: &Path) -> DaemonError {
    error(format!(
        "Claude onboarding state is not a regular file: {}",
        state_path.display()
    ))
}

/// Merges `update` into the state file. Every other field stays as Claude
/// wrote it, and nothing is written when the file already holds the update.
pub(super) fn update(
    state_path: &Path,
    update: &ClaudeStateUpdate,
) -> Result<StateChange, DaemonError> {
    let parent = state_path
        .parent()
        .ok_or_else(|| error("Claude config directory is unavailable"))?;
    fs::create_dir_all(parent).map_err(|source| error(source.to_string()))?;
    let _lock = ProfileLock::acquire(state_path)?;
    for _ in 0..COMMIT_ATTEMPTS {
        let snapshot = read(state_path)?;
        let Some((contents, change)) = merged(snapshot.as_deref(), update)? else {
            return Ok(StateChange::default());
        };
        if commit(state_path, parent, &contents, snapshot.as_deref())? {
            return Ok(change);
        }
    }
    Err(error(
        "Claude kept changing its state file while the launch was preparing it; retry the launch",
    ))
}

struct ProfileLock(PathBuf);

impl ProfileLock {
    fn acquire(state_path: &Path) -> Result<Self, DaemonError> {
        let mut path = state_path.as_os_str().to_owned();
        path.push(".lock");
        let path = PathBuf::from(path);
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE_AFTER);
                    if stale {
                        match fs::remove_dir(&path) {
                            Ok(()) => {}
                            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                            Err(source) => {
                                return Err(error(format!(
                                    "Claude's stale lock {} cannot be removed ({source}); delete it and retry the launch",
                                    path.display()
                                )))
                            }
                        }
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(error(
                            "Claude is saving its state; retry the launch in a moment",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(source) => return Err(error(source.to_string())),
            }
        }
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

fn read(state_path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(state_path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        #[cfg(unix)]
        Err(source) if source.raw_os_error() == Some(libc::ELOOP) => {
            return Err(not_regular(state_path))
        }
        Err(source) => return Err(error(source.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|source| error(source.to_string()))?;
    if !metadata.is_file() {
        return Err(not_regular(state_path));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(not_regular(state_path));
        }
    }
    if metadata.len() > MAX_STATE_BYTES {
        return Err(error("Claude onboarding state exceeds its size limit"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Claude onboarding state could not be read"))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(error("Claude onboarding state exceeds its size limit"));
    }
    Ok(Some(bytes))
}

/// The bytes to write and what they change, or `None` when the file already
/// holds the update.
fn merged(
    snapshot: Option<&[u8]>,
    update: &ClaudeStateUpdate,
) -> Result<Option<(Vec<u8>, StateChange)>, DaemonError> {
    let mut state = match snapshot {
        Some(bytes) if !bytes.iter().all(u8::is_ascii_whitespace) => serde_json::from_slice::<
            serde_json::Value,
        >(bytes)
        .map_err(|source| error(format!("Claude onboarding state is invalid JSON: {source}")))?,
        _ => serde_json::Value::Object(serde_json::Map::new()),
    };
    let object = state
        .as_object_mut()
        .ok_or_else(|| error("Claude onboarding state is not a JSON object"))?;
    let change = update.apply(object)?;
    if !change.changed {
        return Ok(None);
    }
    let mut bytes = serde_json::to_vec(&state).map_err(|source| error(source.to_string()))?;
    bytes.push(b'\n');
    Ok(Some((bytes, change)))
}

/// Replaces the file only while it still holds `snapshot`; `Ok(false)` means
/// Claude changed it since the read and the caller merges again.
fn commit(
    state_path: &Path,
    parent: &Path,
    contents: &[u8],
    snapshot: Option<&[u8]>,
) -> Result<bool, DaemonError> {
    let mut nonce = [0_u8; 8];
    rand::thread_rng().fill_bytes(&mut nonce);
    let pending_path = parent.join(format!(
        ".claude-headless-onboarding-{}-{}.tmp",
        std::process::id(),
        u64::from_le_bytes(nonce),
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut pending = options
        .open(&pending_path)
        .map_err(|source| error(source.to_string()))?;
    let written = pending
        .write_all(contents)
        .and_then(|()| pending.sync_all());
    drop(pending);
    let outcome = written
        .map_err(|source| error(source.to_string()))
        .and_then(|()| {
            if read(state_path)?.as_deref() != snapshot {
                return Ok(false);
            }
            let placed = match snapshot {
                Some(_) => fs::rename(&pending_path, state_path),
                None => fs::hard_link(&pending_path, state_path),
            };
            match placed {
                Ok(()) => Ok(true),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                Err(source) => Err(error(source.to_string())),
            }
        });
    let _ = fs::remove_file(&pending_path);
    if matches!(outcome, Ok(true)) {
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| error(source.to_string()))?;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_dir(name: &str) -> crate::test_support::TestWorktree {
        crate::test_support::TestWorktree::new(name)
    }

    fn trust(workspace: &str) -> ClaudeStateUpdate<'_> {
        ClaudeStateUpdate {
            onboarding: false,
            trusted_workspace: Some(Path::new(workspace)),
        }
    }

    #[test]
    fn a_save_claude_lands_after_the_read_is_not_overwritten() {
        let dir = state_dir("claude-state-concurrent-save");
        let state_path = dir.path().join(".claude.json");
        fs::write(&state_path, r#"{"theme":"dark"}"#).unwrap();
        let snapshot = read(&state_path).unwrap();
        let (contents, _) = merged(snapshot.as_deref(), &trust("/work/a"))
            .unwrap()
            .unwrap();
        // Claude saves between our read and our commit.
        fs::write(&state_path, r#"{"theme":"light"}"#).unwrap();
        assert!(!commit(&state_path, dir.path(), &contents, snapshot.as_deref()).unwrap());
        assert_eq!(
            fs::read_to_string(&state_path).unwrap(),
            r#"{"theme":"light"}"#
        );
        update(&state_path, &trust("/work/a")).unwrap();
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        assert_eq!(merged["theme"], "light");
        assert_eq!(
            merged["projects"]["/work/a"]["hasTrustDialogAccepted"],
            true
        );
    }

    #[test]
    fn the_update_waits_for_the_lock_claude_holds() {
        let dir = state_dir("claude-state-lock");
        let state_path = dir.path().join(".claude.json");
        let lock = dir.path().join(".claude.json.lock");
        fs::create_dir(&lock).unwrap();
        let release = lock.clone();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            fs::remove_dir(release).unwrap();
        });
        update(&state_path, &trust("/work/a")).unwrap();
        releaser.join().unwrap();
        assert!(state_path.exists());
        assert!(!lock.exists(), "the kernel's own lock is released");
    }

    #[test]
    fn a_stale_lock_that_cannot_be_removed_fails_with_what_to_delete() {
        let dir = state_dir("claude-state-stale-lock");
        let state_path = dir.path().join(".claude.json");
        let lock = dir.path().join(".claude.json.lock");
        fs::create_dir(&lock).unwrap();
        fs::write(lock.join("owner"), b"x").unwrap();
        fs::File::open(&lock)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        let refusal = update(&state_path, &trust("/work/a")).unwrap_err();
        assert!(
            refusal.to_string().contains("delete it and retry"),
            "{refusal}"
        );
        assert!(!state_path.exists());
    }
}
