//! The macOS Trash: an item goes to the system trash through `NSFileManager trashItemAtURL`, which
//! picks the right folder for the volume and keeps the name unique. It runs in `osascript` (JavaScript
//! for Automation), so the kernel needs no Objective-C bindings and no Finder permission.
// Built on every system so its path rules are tested here; only macOS runs the system call.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use std::path::{Path, PathBuf};

use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

use crate::error::DaemonError;
use crate::runtime::bounded_process::BoundedCommandExt as _;
use crate::runtime::kernel_files::{fail, io_failure};

/// Prints the path the item has in the trash, or fails with the system's own reason.
const TRASH_SCRIPT: &str = r#"function run(argv) {
  ObjC.import("Foundation");
  const resulting = Ref();
  const failure = Ref();
  const url = $.NSURL.fileURLWithPath(argv[0]);
  if (!$.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError(url, resulting, failure)) {
    throw new Error(ObjC.unwrap(failure[0].localizedDescription));
  }
  return ObjC.unwrap(resulting[0].path);
}"#;

const OSASCRIPT: &str = "/usr/bin/osascript";

pub(super) struct Trash {
    home: PathBuf,
    uid: u32,
    osascript: PathBuf,
}

impl Trash {
    pub(super) fn from_env() -> Result<Self, DaemonError> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            fail(
                "file_path_invalid",
                "This kernel has no home directory, so it has no Trash.",
            )
        })?;
        // SAFETY: geteuid has no preconditions.
        Ok(Self::new(home, unsafe { libc::geteuid() }))
    }

    pub(super) fn new(home: PathBuf, uid: u32) -> Self {
        Self::running(home, uid, OSASCRIPT.into())
    }

    /// The system call goes through `osascript`; a test names its own stand-in program.
    fn running(home: PathBuf, uid: u32, osascript: PathBuf) -> Self {
        Self {
            home,
            uid,
            osascript,
        }
    }

    /// Whether `path` lies in the user's trash or the trash of another volume.
    pub(super) fn holds(&self, path: &Path) -> bool {
        path.starts_with(self.home.join(".Trash")) || self.volume_trash(path)
    }

    fn volume_trash(&self, path: &Path) -> bool {
        let uid = self.uid.to_string();
        let parts: Vec<_> = path.components().map(|part| part.as_os_str()).collect();
        parts
            .windows(2)
            .any(|pair| pair[0] == ".Trashes" && pair[1] == uid.as_str())
    }

    pub(super) fn move_to(&self, path: &Path) -> Result<PathBuf, DaemonError> {
        let output = crate::process_spawn::command(&self.osascript)
            .args(["-l", "JavaScript", "-e", TRASH_SCRIPT])
            .arg(path)
            .bounded_output()
            .map_err(|error| {
                fail(
                    "file_trash_failed",
                    format!("The system Trash could not be reached: {error}."),
                )
            })?;
        if !output.status.success() {
            return Err(fail(
                "file_trash_failed",
                format!(
                    "{} could not be moved to the Trash: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        Ok(trashed_path(&output.stdout))
    }

    /// The system keeps no record Chariox can read, so the item goes back where the client says it was.
    pub(super) fn restore(&self, trash_path: &Path, wanted: &Path) -> Result<PathBuf, DaemonError> {
        if !self.holds(trash_path) {
            return Err(fail(
                "file_not_in_trash",
                format!("{} is not an item in the Trash.", trash_path.display()),
            ));
        }
        std::fs::symlink_metadata(trash_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                fail(
                    "file_not_in_trash",
                    format!("{} is no longer in the Trash.", wanted.display()),
                )
            } else {
                io_failure(trash_path, &error)
            }
        })?;
        if let Some(parent) = wanted.parent() {
            std::fs::create_dir_all(parent).map_err(|error| io_failure(parent, &error))?;
        }
        rename_without_replacing(trash_path, wanted)?;
        Ok(wanted.to_path_buf())
    }
}

/// The path `osascript` printed: the name exactly as the system gave it, minus the one newline the
/// script's output ends with. A name may begin or end with a space, so nothing else is trimmed.
fn trashed_path(stdout: &[u8]) -> PathBuf {
    let path = stdout.strip_suffix(b"\n").unwrap_or(stdout);
    PathBuf::from(std::ffi::OsString::from_vec(path.to_vec()))
}

/// Renames `from` to `to`, failing when anything is at `to`: the restore of an item whose path was
/// taken meanwhile must not replace what is there.
fn rename_without_replacing(from: &Path, to: &Path) -> Result<(), DaemonError> {
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
    if matches!(error.raw_os_error(), Some(libc::EEXIST | libc::ENOTEMPTY)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trashed_path_keeps_every_character_of_the_name() {
        let home = PathBuf::from("/Users/me");
        // The name starts and ends with spaces, so Undo must look for exactly that item.
        assert_eq!(
            trashed_path(b"/Users/me/.Trash/ report .txt  \n"),
            home.join(".Trash/ report .txt  ")
        );
        assert_eq!(
            trashed_path(b"/Users/me/.Trash/no newline"),
            home.join(".Trash/no newline")
        );
        assert_eq!(
            trashed_path(b"/Users/me/.Trash/two\n\n"),
            home.join(".Trash/two\n"),
            "only the one newline the script adds is removed"
        );
    }

    /// The real `move_to` (the subprocess, its exit status and its output) against a stand-in for
    /// `osascript` that prints what the system script prints: the new path of the item and a newline.
    fn move_with(printing: &str, path: &Path) -> Result<PathBuf, DaemonError> {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = crate::test_temp_dir::TestTempDir::new("osascript");
        let program = dir.join("osascript");
        std::fs::write(&program, format!("#!/bin/sh\n{printing}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        Trash::running(PathBuf::from("/Users/me"), 501, program).move_to(path)
    }

    #[test]
    fn the_move_returns_the_trashed_name_exactly_and_the_system_reason_when_it_fails() {
        let home = PathBuf::from("/Users/me");
        // A name that starts and ends with a space, and one that ends with a newline.
        let spaced = move_with(
            r#"printf '%s\n' "/Users/me/.Trash/ report  ""#,
            Path::new("/Users/me/work/ report  "),
        )
        .unwrap();
        assert_eq!(spaced, home.join(".Trash/ report  "));
        let newline = move_with(
            r#"printf '%s\n\n' "/Users/me/.Trash/two""#,
            Path::new("/Users/me/work/two\n"),
        )
        .unwrap();
        assert_eq!(newline, home.join(".Trash/two\n"));
        let refused = move_with(
            r#"echo "The file couldn't be moved" >&2; exit 1"#,
            Path::new("/Users/me/work/a"),
        )
        .unwrap_err();
        assert!(
            matches!(refused, DaemonError::MachineRequest { ref code, ref message }
                if code == "file_trash_failed" && message.contains("couldn't be moved")),
            "{refused:?}"
        );
    }

    #[test]
    fn the_macos_trash_knows_its_folders_and_puts_back_only_what_is_in_them() {
        let home = PathBuf::from("/Users/me");
        let trash = Trash::new(home.clone(), 501);
        assert!(trash.holds(&home.join(".Trash/a.txt")));
        assert!(trash.holds(Path::new("/Volumes/Data/.Trashes/501/a.txt")));
        assert!(!trash.holds(Path::new("/Volumes/Data/.Trashes/502/a.txt")));
        assert!(!trash.holds(&home.join("Documents/a.txt")));
        let outside = trash.restore(&home.join("Documents/a.txt"), &home.join("a.txt"));
        assert!(
            matches!(outside, Err(DaemonError::MachineRequest { code, .. }) if code == "file_not_in_trash")
        );
    }
}
