//! Moving files and folders to the machine's own trash, and back (Files "Move to Trash" and Undo).
//!
//! Nothing here deletes for good: an item goes to the trash the machine's own file manager uses
//! (freedesktop.org Trash specification on Linux, the system trash on macOS), so the owner can also
//! empty or restore it there. `restore` puts an item back at its original path and never overwrites.
use std::path::{Path, PathBuf};

use super::kernel_files::{fail, io_failure, resolve};
use crate::error::DaemonError;
use crate::local::{
    KernelFileFailure, LocalDaemonResponse, RestoreKernelTrashRequest, TrashKernelFilesRequest,
    TrashedKernelFile,
};
use crate::machine_files::StateGuard;

#[cfg(not(target_os = "macos"))]
mod dir;
#[cfg(not(target_os = "macos"))]
mod freedesktop;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(not(target_os = "macos"))]
use freedesktop as platform;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(all(test, not(target_os = "macos")))]
mod tests;

pub(crate) fn trash(
    request: &TrashKernelFilesRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    trash_with(&platform::Trash::from_env()?, request, guard)
}

fn trash_with(
    trash: &platform::Trash,
    request: &TrashKernelFilesRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.paths.is_empty() {
        return Err(fail(
            "file_path_invalid",
            "Name at least one file or folder to move to the Trash.",
        ));
    }
    let mut resolved: Vec<(String, PathBuf)> = Vec::new();
    let mut refused = Vec::new();
    for raw in &request.paths {
        match resolve(raw, guard) {
            Ok(path) if resolved.iter().any(|(_, seen)| *seen == path) => {}
            Ok(path) => resolved.push((raw.clone(), path)),
            Err(error) => refused.push((raw.clone(), error)),
        }
    }
    // Folders first, so something named inside a folder that moved is found with it. When the folder
    // is refused, what was named inside it is tried on its own and ends with its own result.
    resolved.sort_by_key(|(_, path)| path.components().count());
    let mut trashed: Vec<TrashedKernelFile> = Vec::new();
    for (raw, path) in &resolved {
        let item = |trash_path: &Path| TrashedKernelFile {
            path: path.display().to_string(),
            trash_path: trash_path.display().to_string(),
        };
        let inside = trashed.iter().find_map(|moved| {
            let rest = path.strip_prefix(&moved.path).ok()?;
            Some(Path::new(&moved.trash_path).join(rest))
        });
        if let Some(trash_path) = inside {
            trashed.push(item(&trash_path));
            continue;
        }
        match trash_one(trash, path, guard) {
            Ok(trash_path) => trashed.push(item(&trash_path)),
            Err(error) => refused.push((raw.clone(), error)),
        }
    }
    let failed = answer(refused, trashed.is_empty())?;
    Ok(LocalDaemonResponse::KernelFilesTrashed { trashed, failed })
}

pub(crate) fn restore(
    request: &RestoreKernelTrashRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    restore_with(&platform::Trash::from_env()?, request, guard)
}

fn restore_with(
    trash: &platform::Trash,
    request: &RestoreKernelTrashRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.items.is_empty() {
        return Err(fail(
            "file_path_invalid",
            "Name at least one item to restore from the Trash.",
        ));
    }
    // Outer folders first: an item that was moved inside a folder named in the same request came back with it.
    let mut items: Vec<&TrashedKernelFile> = request.items.iter().collect();
    items.sort_by_key(|item| Path::new(&item.trash_path).components().count());
    let mut restored: Vec<String> = Vec::new();
    let mut back: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut refused = Vec::new();
    for item in items {
        let trash_path = Path::new(&item.trash_path);
        let with_folder = back.iter().any(|(outer_trash, outer_path)| {
            trash_path
                .strip_prefix(outer_trash)
                .ok()
                .is_some_and(|rest| {
                    outer_path.join(rest) == Path::new(&item.path) && Path::new(&item.path).exists()
                })
        });
        if with_folder {
            restored.push(item.path.clone());
            continue;
        }
        match restore_one(trash, item, guard) {
            Ok(original) => {
                back.push((trash_path.to_path_buf(), original.clone()));
                restored.push(original.display().to_string());
            }
            Err(error) => refused.push((item.path.clone(), error)),
        }
    }
    let failed = answer(refused, restored.is_empty())?;
    Ok(LocalDaemonResponse::KernelTrashRestored { restored, failed })
}

/// Items that could not be done: reported next to the ones that were, or as the request's own error
/// when none was (a single item's refusal is then its own message).
fn answer(
    refused: Vec<(String, DaemonError)>,
    nothing_done: bool,
) -> Result<Vec<KernelFileFailure>, DaemonError> {
    if nothing_done {
        let (count, first) = (refused.len(), refused.into_iter().next());
        return Err(match first {
            Some((_, error)) if count == 1 => error,
            Some((path, DaemonError::MachineRequest { code, message })) => fail(
                &code,
                format!("None of the {count} items could be done; {path}: {message}"),
            ),
            Some((_, other)) => other,
            None => fail("file_path_invalid", "Nothing was named."),
        });
    }
    Ok(refused
        .into_iter()
        .map(|(path, error)| KernelFileFailure {
            path,
            message: match error {
                DaemonError::MachineRequest { message, .. } => message,
                other => other.to_string(),
            },
        })
        .collect())
}

fn trash_one(
    trash: &platform::Trash,
    path: &Path,
    guard: &StateGuard,
) -> Result<PathBuf, DaemonError> {
    std::fs::symlink_metadata(path).map_err(|error| io_failure(path, &error))?;
    if path.parent().is_none()
        || std::env::var_os("HOME").is_some_and(|home| Path::new(&home).starts_with(path))
    {
        return Err(fail(
            "file_path_invalid",
            format!(
                "{} holds your home folder; Chariox does not move it to the Trash.",
                path.display()
            ),
        ));
    }
    guard
        .check_replace(path)
        .map_err(|message| fail("file_state_protected", message))?;
    if trash.holds(path) {
        return Err(fail(
            "file_trash_failed",
            format!("{} is in the Trash already.", path.display()),
        ));
    }
    trash.move_to(path)
}

fn restore_one(
    trash: &platform::Trash,
    item: &TrashedKernelFile,
    guard: &StateGuard,
) -> Result<PathBuf, DaemonError> {
    let wanted = resolve(&item.path, guard)?;
    trash.restore(Path::new(&item.trash_path), &wanted)
}
