//! The macOS Trash: an item goes to the system trash through `NSFileManager trashItemAtURL`, which
//! picks the right folder for the volume and keeps the name unique. It runs in `osascript` (JavaScript
//! for Automation), so the kernel needs no Objective-C bindings and no Finder permission.
// Built on every system so its path rules are tested here; only macOS runs the system call.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use std::path::{Path, PathBuf};

use super::Located;
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

pub(super) struct Trash {
    home: PathBuf,
    uid: u32,
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
        Self { home, uid }
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
        let output = crate::process_spawn::command("/usr/bin/osascript")
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
        Ok(PathBuf::from(
            String::from_utf8_lossy(&output.stdout).trim(),
        ))
    }

    /// The system keeps no record Chariox can read, so the item goes back where the client says it was.
    pub(super) fn locate(
        &self,
        trash_path: &Path,
        original: &Path,
    ) -> Result<Located, DaemonError> {
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
                    format!("{} is no longer in the Trash.", original.display()),
                )
            } else {
                io_failure(trash_path, &error)
            }
        })?;
        Ok(Located {
            trash_path: trash_path.to_path_buf(),
            original: original.to_path_buf(),
            record: None,
        })
    }
}
