//! Moving files and folders to the machine's own trash, and back (Files "Move to Trash" and Undo).
//!
//! Nothing here deletes for good: an item goes to the trash the machine's own file manager uses
//! (freedesktop.org Trash specification on Linux, the system trash on macOS), so the owner can also
//! empty or restore it there. `restore` puts an item back at its original path and never overwrites.
use std::path::{Component, Path, PathBuf};

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
#[cfg(target_os = "macos")]
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
        match plain(raw, guard) {
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
        // Inside a folder that moved: the item is reported once it is found in the moved folder. One
        // that is not there (never existed, or reached through a link) is tried on its own.
        let inside = trashed.iter().find_map(|moved| {
            let rest = path.strip_prefix(&moved.path).ok()?;
            found_inside(Path::new(&moved.trash_path), rest)
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
                    outer_path.join(rest) == Path::new(&item.path)
                        && std::fs::symlink_metadata(&item.path).is_ok()
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

/// A path as the Files client and the agent tools name it: absolute, and with no `..` that could make the
/// item it names differ from the one it reads as (a lone `.` inside a path is dropped by `Path` itself).
fn plain(raw: &str, guard: &StateGuard) -> Result<PathBuf, DaemonError> {
    let path = resolve(raw, guard)?;
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(fail(
            "file_path_invalid",
            format!("\"{raw}\" contains ..; name the full path of the file or folder."),
        ));
    }
    Ok(path)
}

/// Where `rest` of a folder now at `moved` is in the Trash, when it is there: every folder on the way is
/// a real folder (not a link out of it) and the last part exists.
fn found_inside(moved: &Path, rest: &Path) -> Option<PathBuf> {
    let is_folder = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    let mut at = moved.to_path_buf();
    let parts: Vec<_> = rest.components().collect();
    for (index, part) in parts.iter().enumerate() {
        if !is_folder(&at) {
            return None;
        }
        at.push(part);
        if index + 1 == parts.len() {
            return std::fs::symlink_metadata(&at).ok().map(|_| at);
        }
    }
    None
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
    let wanted = plain(&item.path, guard)?;
    // The platform reads where an item is from the position of its path in a trash folder, so a path
    // that is not plain could name something else than it reads as.
    let trash_path = Path::new(&item.trash_path);
    if !trash_path.is_absolute()
        || trash_path
            .components()
            .any(|part| part == Component::ParentDir)
    {
        return Err(fail(
            "file_not_in_trash",
            format!("{} is not an item in the Trash.", item.trash_path),
        ));
    }
    trash.restore(trash_path, &wanted)
}
