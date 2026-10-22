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

fn update_claude_state_at(state_path: &Path, update: StateUpdate<'_>) -> Result<(), DaemonError> {
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
    let existing_state = match state_options.open(state_path) {
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
            let mut state = if bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "prepare Claude local state",
                        message: format!("Claude local state is invalid JSON: {error}"),
                    }
                })?
            };
            if !apply_update(&mut state, update)? {
                return Ok(());
            }
            let mut bytes =
                serde_json::to_vec(&state).map_err(|error| DaemonError::LocalTransport {
                    operation: "prepare Claude local state",
                    message: error.to_string(),
                })?;
            bytes.push(b'\n');
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
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
            return Err(DaemonError::LocalTransport {
                operation: "prepare Claude local state",
                message: error.to_string(),
            });
        }
    };
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
    let state_existed = existing_state.is_some();
    let contents = match existing_state {
        Some(ref bytes) => bytes.clone(),
        None => {
            let mut state = serde_json::json!({});
            apply_update(&mut state, update)?;
            let mut bytes = serde_json::to_vec(&state)
                .map_err(|_| state_error("Claude local state could not be encoded"))?;
            bytes.push(b'\n');
            bytes
        }
    };
    let write_result = pending
        .write_all(&contents)
        .and_then(|()| pending.sync_all());
    if let Err(error) = write_result {
        drop(pending);
        let _ = fs::remove_file(pending_path);
        return Err(DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: error.to_string(),
        });
    }
    drop(pending);
    let result = if state_existed {
        fs::rename(&pending_path, state_path).map_err(|error| DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: error.to_string(),
        })
    } else {
        match fs::hard_link(&pending_path, state_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&pending_path);
                return update_claude_state_at(state_path, update);
            }
            Err(error) => Err(DaemonError::LocalTransport {
                operation: "prepare Claude local state",
                message: error.to_string(),
            }),
        }
    };
    let _ = fs::remove_file(pending_path);
    result?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "prepare Claude local state",
            message: error.to_string(),
        })?;
    Ok(())
}
