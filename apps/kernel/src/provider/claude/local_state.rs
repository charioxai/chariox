//! MP-08 / MP-10 / MP-11: provider-local onboarding and exact-folder trust.
use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;
use rand::RngCore;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Clone, Copy)]
enum StateUpdate<'a> {
    CompleteOnboarding,
    TrustWorkspace(&'a str),
}

fn state_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "prepare Claude local state",
        message: message.into(),
    }
}

pub(super) fn ensure_claude_headless_onboarding_state_at(
    state_path: &Path,
) -> Result<(), DaemonError> {
    update_claude_state_at(state_path, StateUpdate::CompleteOnboarding)
}

pub(super) fn prepare_workspace_trust(request: &LaunchProviderRequest) -> Result<(), DaemonError> {
    let Some(workspace) = request.working_directory.as_deref() else {
        return Ok(());
    };
    let workspace = workspace.canonicalize().map_err(|_| {
        state_error("Claude workspace is unavailable; restore the selected folder before launching")
    })?;
    if !workspace.is_dir() {
        return Err(state_error("Claude workspace must be a directory"));
    }
    let workspace = workspace
        .to_str()
        .ok_or_else(|| state_error("Claude workspace path must be valid UTF-8"))?;
    let state_path = super::claude_state_path(Some(request))?;
    update_claude_state_at(&state_path, StateUpdate::TrustWorkspace(workspace))
}

fn apply_update(
    state: &mut serde_json::Value,
    update: StateUpdate<'_>,
) -> Result<bool, DaemonError> {
    let object = state
        .as_object_mut()
        .ok_or_else(|| state_error("Claude local state is not a JSON object"))?;
    let (target, key) = match update {
        StateUpdate::CompleteOnboarding => (object, "hasCompletedOnboarding"),
        StateUpdate::TrustWorkspace(workspace) => {
            // The kernel has already admitted this launch into its selected
            // workspace. This records folder trust, never tool permissions.
            let projects = object
                .entry("projects")
                .or_insert_with(|| serde_json::json!({}));
            let projects = projects.as_object_mut().ok_or_else(|| {
                state_error(
                    "Claude projects state is not a JSON object; repair it before launching",
                )
            })?;
            let project = projects
                .entry(workspace)
                .or_insert_with(|| serde_json::json!({}));
            let project = project.as_object_mut().ok_or_else(|| {
                state_error(
                    "Claude workspace state is not a JSON object; repair it before launching",
                )
            })?;
            (project, "hasTrustDialogAccepted")
        }
    };
    if target.get(key).and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(false);
    }
    target.insert(key.into(), serde_json::Value::Bool(true));
    Ok(true)
}

/// Claude saves its profile under a `<file>.lock` directory (proper-lockfile
/// convention). Holding the same lock keeps our single-key update from landing
/// between Claude's read and write.
const LOCK_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(10);
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const COMMIT_ATTEMPTS: usize = 8;

struct ProfileLock(std::path::PathBuf);

impl ProfileLock {
    fn acquire(state_path: &Path) -> Result<Self, DaemonError> {
        let mut lock_path = state_path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock_path = std::path::PathBuf::from(lock_path);
        let deadline = std::time::Instant::now() + LOCK_WAIT;
        loop {
            match fs::create_dir(&lock_path) {
                Ok(()) => return Ok(Self(lock_path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock_path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE_AFTER);
                    if stale {
                        let _ = fs::remove_dir(&lock_path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(state_error(
                            "Claude is saving its profile; retry the launch in a moment",
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "prepare Claude local state",
                        message: error.to_string(),
                    })
                }
            }
        }
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

fn update_claude_state_at(state_path: &Path, update: StateUpdate<'_>) -> Result<(), DaemonError> {
    update_claude_state_with(state_path, update, &mut || {})
}

/// `after_read` runs between reading the profile and committing the change; it
/// exists so tests can write the profile at exactly that moment.
fn update_claude_state_with(
    state_path: &Path,
    update: StateUpdate<'_>,
    after_read: &mut dyn FnMut(),
) -> Result<(), DaemonError> {
    let parent = state_path
        .parent()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: "Claude config directory is unavailable".to_string(),
        })?;
    fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare Claude local state",
        message: error.to_string(),
    })?;
    let _lock = ProfileLock::acquire(state_path)?;
    for _ in 0..COMMIT_ATTEMPTS {
        let snapshot = read_state_bytes(state_path)?;
        let Some(contents) = updated_contents(snapshot.as_deref(), update)? else {
            return Ok(());
        };
        after_read();
        if commit_state(state_path, parent, &contents, snapshot.as_deref())? {
            return Ok(());
        }
    }
    Err(state_error(
        "Claude kept changing its profile while the folder was being trusted; retry the launch",
    ))
}

fn read_state_bytes(state_path: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut state_options = fs::OpenOptions::new();
    state_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        state_options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        state_options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    match state_options.open(state_path) {
        Ok(file) => {
            let metadata = file
                .metadata()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: error.to_string(),
                })?;
            if !metadata.is_file() {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: format!(
                        "Claude onboarding state is not a regular file: {}",
                        state_path.display()
                    ),
                });
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(DaemonError::LocalTransport {
                        operation: "prepare Claude local state",
                        message: format!(
                            "Claude onboarding state is not a regular file: {}",
                            state_path.display()
                        ),
                    });
                }
            }
            const MAX_ONBOARDING_BYTES: u64 = 1024 * 1024;
            if metadata.len() > MAX_ONBOARDING_BYTES {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: "Claude onboarding state exceeds its size limit".into(),
                });
            }
            let mut bytes = Vec::new();
            file.take(MAX_ONBOARDING_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: "Claude onboarding state could not be read".into(),
                })?;
            if bytes.len() as u64 > MAX_ONBOARDING_BYTES {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: "Claude onboarding state exceeds its size limit".into(),
                });
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            #[cfg(unix)]
            if error.raw_os_error() == Some(libc::ELOOP) {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: format!(
                        "Claude onboarding state is not a regular file: {}",
                        state_path.display()
                    ),
                });
            }
            Err(DaemonError::LocalTransport {
                operation: "prepare Claude local state",
                message: error.to_string(),
            })
        }
    }
}

/// The bytes to write, or `None` when the profile already has the change.
fn updated_contents(
    snapshot: Option<&[u8]>,
    update: StateUpdate<'_>,
) -> Result<Option<Vec<u8>>, DaemonError> {
    let mut state = match snapshot {
        None => serde_json::json!({}),
        Some(bytes) if bytes.iter().all(|byte| byte.is_ascii_whitespace()) => {
            serde_json::Value::Object(serde_json::Map::new())
        }
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "prepare Claude local state",
                message: format!("Claude local state is invalid JSON: {error}"),
            }
        })?,
    };
    if !apply_update(&mut state, update)? {
        return Ok(None);
    }
    let mut bytes = serde_json::to_vec(&state).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare Claude local state",
        message: error.to_string(),
    })?;
    bytes.push(b'\n');
    Ok(Some(bytes))
}

/// Replaces the profile only if it still holds `snapshot`; `Ok(false)` means it
/// changed since the read and the caller must merge again.
fn commit_state(
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
    let mut pending_options = fs::OpenOptions::new();
    pending_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        pending_options.mode(0o600);
    }
    let mut pending =
        pending_options
            .open(&pending_path)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "prepare Claude local state",
                message: error.to_string(),
            })?;
    let write_result = pending
        .write_all(contents)
        .and_then(|()| pending.sync_all());
    drop(pending);
    if let Err(error) = write_result {
        let _ = fs::remove_file(pending_path);
        return Err(DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: error.to_string(),
        });
    }
    let committed = if snapshot.is_some() {
        // Re-read immediately before the rename so a save that landed after
        // our first read is merged on the next pass instead of overwritten.
        if read_state_bytes(state_path)?.as_deref() != snapshot {
            let _ = fs::remove_file(&pending_path);
            return Ok(false);
        }
        fs::rename(&pending_path, state_path).map(|()| true)
    } else {
        match fs::hard_link(&pending_path, state_path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    };
    let _ = fs::remove_file(&pending_path);
    if !committed.map_err(|error| DaemonError::LocalTransport {
        operation: "prepare Claude local state",
        message: error.to_string(),
    })? {
        return Ok(false);
    }
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: error.to_string(),
        })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "chariox-claude-state-{name}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join(".claude.json")
    }

    #[test]
    fn trust_keeps_a_claude_save_that_lands_after_the_read() {
        let state_path = temp_state("race");
        fs::write(&state_path, br#"{"numStartups":1}"#).unwrap();
        let mut raced = false;
        update_claude_state_with(
            &state_path,
            StateUpdate::TrustWorkspace("/work/a"),
            &mut || {
                if !raced {
                    raced = true;
                    fs::write(&state_path, br#"{"numStartups":2,"theme":"dark"}"#).unwrap();
                }
            },
        )
        .unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        assert_eq!(state["numStartups"], 2);
        assert_eq!(state["theme"], "dark");
        assert_eq!(state["projects"]["/work/a"]["hasTrustDialogAccepted"], true);
        fs::remove_dir_all(state_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn trust_keeps_a_claude_profile_created_after_the_read() {
        let state_path = temp_state("create");
        let mut raced = false;
        update_claude_state_with(
            &state_path,
            StateUpdate::TrustWorkspace("/work/a"),
            &mut || {
                if !raced {
                    raced = true;
                    fs::write(&state_path, br#"{"theme":"dark"}"#).unwrap();
                }
            },
        )
        .unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        assert_eq!(state["theme"], "dark");
        assert_eq!(state["projects"]["/work/a"]["hasTrustDialogAccepted"], true);
        fs::remove_dir_all(state_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn trust_waits_for_the_profile_lock_claude_holds() {
        let state_path = temp_state("lock");
        fs::write(&state_path, br#"{}"#).unwrap();
        let mut lock_path = state_path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock_path = std::path::PathBuf::from(lock_path);
        fs::create_dir(&lock_path).unwrap();
        let release = lock_path.clone();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            fs::remove_dir(release).unwrap();
        });
        update_claude_state_at(&state_path, StateUpdate::TrustWorkspace("/work/a")).unwrap();
        releaser.join().unwrap();
        assert!(!lock_path.exists());
        fs::remove_dir_all(state_path.parent().unwrap()).unwrap();
    }
}
