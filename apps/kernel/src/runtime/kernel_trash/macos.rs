//! The macOS Trash: an item goes to the system trash through `NSFileManager trashItemAtURL`, which
//! picks the right folder for the volume and keeps the name unique. It runs in `osascript` (JavaScript
//! for Automation), so the kernel needs no Objective-C bindings and no Finder permission.
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

type MountOf = Box<dyn Fn(&Path) -> std::io::Result<PathBuf> + Send + Sync>;

pub(super) struct Trash {
    home: PathBuf,
    uid: u32,
    osascript: PathBuf,
    /// The mount point of the volume a path is on; a test says where its volumes are mounted.
    mount_of: MountOf,
}

/// The mount point of the volume `path` is on, as the system reports it: a volume may be mounted at any
/// folder, not only in `/Volumes`.
fn mount_point(path: &Path) -> std::io::Result<PathBuf> {
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stat` has room for one `statfs`.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: statfs succeeded, so it filled `stat`, and `f_mntonname` is NUL-terminated.
    let name = unsafe { std::ffi::CStr::from_ptr(stat.assume_init_ref().f_mntonname.as_ptr()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
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
        Self::running(home, uid, OSASCRIPT.into(), Box::new(mount_point))
    }

    /// The system call goes through `osascript`, and mount points come from the system; a test names
    /// its own stand-in program and mounts.
    fn running(home: PathBuf, uid: u32, osascript: PathBuf, mount_of: MountOf) -> Self {
        Self {
            home,
            uid,
            osascript,
            mount_of,
        }
    }

    /// Whether `path` is the user's trash, the trash of a volume, or in one.
    pub(super) fn holds(&self, path: &Path) -> bool {
        self.trash_folder_of(path).is_some()
    }

    /// The trash folder `path` is or lies in: `~/.Trash`, or `.Trashes/$uid` at the mount point of the
    /// volume `path` is on, wherever it is mounted. A folder of the user's own named `.Trashes` below
    /// that mount point is not a trash.
    fn trash_folder_of(&self, path: &Path) -> Option<PathBuf> {
        let home_trash = self.home.join(".Trash");
        if path.starts_with(&home_trash) {
            return Some(home_trash);
        }
        // The volume of the folder that holds `path`, so a link in a trash is not followed to another one.
        let volume = (self.mount_of)(path.parent()?).ok()?;
        let folder = volume.join(".Trashes").join(self.uid.to_string());
        path.starts_with(&folder).then_some(folder)
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
        if self
            .trash_folder_of(trash_path)
            .is_none_or(|folder| folder == trash_path)
        {
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
    // SAFETY: both arguments are NUL-terminated strings that live through the call.
    let rc = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
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

    /// A Trash of uid 501 on a machine whose startup volume is at `/` and other volumes at `mounts`.
    fn on_machine(home: &Path, osascript: PathBuf, mounts: &[&Path]) -> Trash {
        let mounts: Vec<PathBuf> = mounts.iter().map(|mount| mount.to_path_buf()).collect();
        let mount_of = move |path: &Path| {
            Ok(mounts
                .iter()
                .filter(|mount| path.starts_with(mount))
                .max_by_key(|mount| mount.components().count())
                .cloned()
                .unwrap_or_else(|| PathBuf::from("/")))
        };
        Trash::running(home.to_path_buf(), 501, osascript, Box::new(mount_of))
    }

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
        on_machine(Path::new("/Users/me"), program, &[]).move_to(path)
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
        let mounts = [Path::new("/Volumes/Data"), Path::new("/mnt/data")];
        let trash = on_machine(&home, OSASCRIPT.into(), &mounts);
        assert!(trash.holds(&home.join(".Trash/a.txt")));
        assert!(trash.holds(Path::new("/Volumes/Data/.Trashes/501/a.txt")));
        assert!(trash.holds(Path::new("/mnt/data/.Trashes/501/a.txt")));
        assert!(!trash.holds(Path::new("/Volumes/Data/.Trashes/502/a.txt")));
        assert!(trash.holds(Path::new("/.Trashes/501/a.txt")));
        assert!(!trash.holds(&home.join("Documents/a.txt")));
        // A folder named like a trash below the mount point of its volume is not one.
        assert!(!trash.holds(&home.join("work/.Trashes/501/a.txt")));
        assert!(!trash.holds(Path::new("/Volumes/Data/work/.Trashes/501/a.txt")));
        assert!(!trash.holds(Path::new("/mnt/.Trashes/501/a.txt")));
        assert!(!trash.holds(Path::new("/Volumes/.Trashes/501/a.txt")));
        // Only an item in a trash folder is put back, never the folder itself.
        for not_an_item in [
            home.join("Documents/a.txt"),
            home.join("work/.Trashes/501/a.txt"),
            home.join(".Trash"),
            PathBuf::from("/Volumes/Data/.Trashes/501"),
            PathBuf::from("/mnt/data/.Trashes/501"),
            PathBuf::from("/mnt/.Trashes/501/a.txt"),
        ] {
            let outside = trash.restore(&not_an_item, &home.join("a.txt"));
            assert!(
                matches!(outside, Err(DaemonError::MachineRequest { ref code, ref message })
                    if code == "file_not_in_trash" && message.contains("is not an item")),
                "{}: {outside:?}",
                not_an_item.display()
            );
        }
    }

    /// Undo of an item the system moved to the trash of a volume mounted outside `/Volumes`: the
    /// exact path `move_to` gave puts the item back.
    #[test]
    fn an_item_in_the_trash_of_a_volume_mounted_anywhere_comes_back() {
        let dir = crate::test_temp_dir::TestTempDir::new("macos-volume");
        let volume = dir.join("mnt/data");
        let in_trash = volume.join(".Trashes/501/a.txt");
        std::fs::create_dir_all(in_trash.parent().unwrap()).unwrap();
        std::fs::write(&in_trash, b"a").unwrap();
        let trash = on_machine(&dir.join("home"), OSASCRIPT.into(), &[&volume]);
        let wanted = volume.join("work/a.txt");

        let back = trash.restore(&in_trash, &wanted).unwrap();

        assert_eq!(back, wanted);
        assert_eq!(std::fs::read(&wanted).unwrap(), b"a");
        assert!(!in_trash.exists());
    }
}
