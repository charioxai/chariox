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
mod freedesktop;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(not(target_os = "macos"))]
use freedesktop as platform;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(test)]
mod tests;

/// Where a trashed item sits and where it belongs again.
struct Located {
    trash_path: PathBuf,
    original: PathBuf,
    /// The record the trash keeps beside the item (the freedesktop `.trashinfo`); removed once restored.
    record: Option<PathBuf>,
}

pub(crate) fn trash(
    request: &TrashKernelFilesRequest,
    guard: &StateGuard,
) -> Result<LocalDaemonResponse, DaemonError> {
    if request.paths.is_empty() {
        return Err(fail(
            "file_path_invalid",
            "Name at least one file or folder to move to the Trash.",
        ));
    }
    let trash = platform::Trash::from_env()?;
    let mut resolved: Vec<(String, PathBuf)> = Vec::new();
    let mut refused = Vec::new();
    for raw in &request.paths {
        match resolve(raw, guard) {
            Ok(path) if resolved.iter().any(|(_, seen)| *seen == path) => {}
            Ok(path) => resolved.push((raw.clone(), path)),
            Err(error) => refused.push((raw.clone(), error)),
        }
    }
    let mut trashed = Vec::new();
    for (raw, path) in &resolved {
        // A folder and something inside it, both named: the folder takes the item along.
        if resolved
            .iter()
            .any(|(_, other)| other != path && path.starts_with(other))
        {
            continue;
        }
        match trash_one(&trash, path, guard) {
            Ok(trash_path) => trashed.push(TrashedKernelFile {
                path: path.display().to_string(),
                trash_path: trash_path.display().to_string(),
            }),
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
    if request.items.is_empty() {
        return Err(fail(
            "file_path_invalid",
            "Name at least one item to restore from the Trash.",
        ));
    }
    let trash = platform::Trash::from_env()?;
    let mut restored = Vec::new();
    let mut refused = Vec::new();
    for item in &request.items {
        match restore_one(&trash, item, guard) {
            Ok(original) => restored.push(original.display().to_string()),
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
    let located = trash.locate(Path::new(&item.trash_path), &wanted)?;
    if let Some(parent) = located.original.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io_failure(parent, &error))?;
    }
    rename_without_replacing(&located.trash_path, &located.original)?;
    if let Some(record) = &located.record {
        let _ = std::fs::remove_file(record);
    }
    Ok(located.original)
}

/// Renames `from` to `to`, failing when anything is at `to`: the restore of an item whose path was
/// taken meanwhile must not replace what is there.
fn rename_without_replacing(from: &Path, to: &Path) -> Result<(), DaemonError> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = |path: &Path| {
        std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| fail("file_path_invalid", "A path holds a NUL byte."))
    };
    let (from_c, to_c) = (c(from)?, c(to)?);
    #[cfg(target_os = "macos")]
    // SAFETY: both arguments are NUL-terminated strings that live through the call.
    let rc = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(not(target_os = "macos"))]
    // SAFETY: both arguments are NUL-terminated strings that live through the call.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) as i32
    };
    if rc == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST) || error.raw_os_error() == Some(libc::ENOTEMPTY) {
        return Err(fail(
            "file_restore_conflict",
            format!(
                "{} is taken, so the item stays in the Trash. Move or rename what is there, then restore it again.",
                to.display()
            ),
        ));
    }
    Err(io_failure(to, &error))
}
