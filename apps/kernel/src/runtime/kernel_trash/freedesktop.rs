//! The freedesktop.org Trash specification: `files/` holds the items, `info/<name>.trashinfo` says
//! where each came from. The home trash serves its own filesystem; an item on another one goes to that
//! filesystem's `$topdir/.Trash/$uid` (when the admin made it) or `$topdir/.Trash-$uid`, so a move is
//! always a rename and never a copy; where neither can be made the item stays and the reason says so.
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use super::Located;
use crate::error::DaemonError;
use crate::runtime::kernel_files::{fail, io_failure};

const STICKY: u32 = 0o1000;

pub(super) struct Trash {
    home_trash: PathBuf,
    uid: u32,
}

/// One trash folder and, for a per-filesystem one, the mount point its `Path=` lines are relative to.
struct Folder {
    root: PathBuf,
    topdir: Option<PathBuf>,
}

impl Trash {
    pub(super) fn from_env() -> Result<Self, DaemonError> {
        let data_home = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
            Some(path) if path.is_absolute() => path,
            _ => std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".local/share"))
                .ok_or_else(|| {
                    fail(
                        "file_path_invalid",
                        "This kernel has no home directory, so it has no Trash.",
                    )
                })?,
        };
        // SAFETY: geteuid has no preconditions.
        Ok(Self::new(data_home.join("Trash"), unsafe {
            libc::geteuid()
        }))
    }

    pub(super) fn new(home_trash: PathBuf, uid: u32) -> Self {
        Self { home_trash, uid }
    }

    /// Whether `path` lies in one of the trash folders this machine keeps.
    pub(super) fn holds(&self, path: &Path) -> bool {
        path.starts_with(&self.home_trash)
            || path.components().any(|part| {
                let name = part.as_os_str().as_bytes();
                name == b".Trash" || name == format!(".Trash-{}", self.uid).as_bytes()
            })
    }

    pub(super) fn move_to(&self, path: &Path) -> Result<PathBuf, DaemonError> {
        let parent = path.parent().unwrap_or(path);
        let device = std::fs::metadata(parent)
            .map_err(|error| io_failure(parent, &error))?
            .dev();
        let folder = self.folder_for(parent, device)?;
        let base = path.file_name().unwrap_or_default();
        let recorded = match &folder.topdir {
            Some(topdir) => path.strip_prefix(topdir).unwrap_or(path),
            None => path,
        };
        let (name, info) = reserve(&folder.root, base.as_bytes(), recorded)?;
        let target = folder.root.join("files").join(&name);
        if let Err(error) = std::fs::rename(path, &target) {
            let _ = std::fs::remove_file(&info);
            return Err(fail(
                "file_trash_failed",
                format!(
                    "{} could not be moved to the Trash: {error}.",
                    path.display()
                ),
            ));
        }
        Ok(target)
    }

    fn folder_for(&self, parent: &Path, device: u64) -> Result<Folder, DaemonError> {
        make_folder(&self.home_trash)?;
        if std::fs::metadata(&self.home_trash).is_ok_and(|home| home.dev() == device) {
            return Ok(Folder {
                root: self.home_trash.clone(),
                topdir: None,
            });
        }
        let mut topdir = parent;
        while let Some(above) = topdir.parent() {
            if std::fs::metadata(above).is_ok_and(|meta| meta.dev() == device) {
                topdir = above;
            } else {
                break;
            }
        }
        let shared = topdir.join(".Trash");
        let uid_name = self.uid.to_string();
        if let Ok(meta) = std::fs::symlink_metadata(&shared) {
            if meta.is_dir() && meta.mode() & STICKY != 0 {
                let own = shared.join(&uid_name);
                if make_folder(&own).is_ok() {
                    return Ok(Folder {
                        root: own,
                        topdir: Some(topdir.to_path_buf()),
                    });
                }
            }
        }
        let own = topdir.join(format!(".Trash-{}", self.uid));
        make_folder(&own).map_err(|_| {
            fail(
                "file_trash_failed",
                format!(
                    "{} is on a different drive than your home folder, and the Trash folder for that drive ({}) cannot be made. Nothing was moved.",
                    parent.display(),
                    own.display()
                ),
            )
        })?;
        Ok(Folder {
            root: own,
            topdir: Some(topdir.to_path_buf()),
        })
    }

    /// Checks `trash_path` is an item of one of this user's trash folders that was `original`, and says
    /// where it goes back.
    pub(super) fn locate(
        &self,
        trash_path: &Path,
        original: &Path,
    ) -> Result<Located, DaemonError> {
        let not_trash = || {
            fail(
                "file_not_in_trash",
                format!("{} is not an item in the Trash.", trash_path.display()),
            )
        };
        let items = trash_path.parent().ok_or_else(not_trash)?;
        let root = items.parent().ok_or_else(not_trash)?;
        if items.file_name() != Some("files".as_ref()) {
            return Err(not_trash());
        }
        let topdir = if root == self.home_trash {
            None
        } else if root.file_name().map(|n| n.as_bytes())
            == Some(format!(".Trash-{}", self.uid).as_bytes())
        {
            root.parent().map(Path::to_path_buf)
        } else if root.file_name().map(|n| n.as_bytes()) == Some(self.uid.to_string().as_bytes())
            && root.parent().and_then(Path::file_name) == Some(".Trash".as_ref())
        {
            root.parent().and_then(Path::parent).map(Path::to_path_buf)
        } else {
            return Err(not_trash());
        };
        let name = trash_path.file_name().ok_or_else(not_trash)?;
        let mut record_name = name.to_os_string();
        record_name.push(".trashinfo");
        let record = root.join("info").join(record_name);
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
        let recorded = read_original(&record, topdir.as_deref())?;
        if recorded != original {
            return Err(fail(
                "file_not_in_trash",
                format!(
                    "The Trash keeps {} as {}, not {}.",
                    trash_path.display(),
                    recorded.display(),
                    original.display()
                ),
            ));
        }
        Ok(Located {
            trash_path: trash_path.to_path_buf(),
            original: recorded,
            record: Some(record),
        })
    }
}

/// `files/` and `info/` under `root`, private to the user as the specification asks.
fn make_folder(root: &Path) -> Result<(), DaemonError> {
    for part in ["files", "info"] {
        let dir = root.join(part);
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|error| io_failure(&dir, &error))?;
    }
    Ok(())
}

/// A free name in the trash and its `.trashinfo`, created together so two moves never share a name.
fn reserve(
    root: &Path,
    base: &[u8],
    recorded: &Path,
) -> Result<(std::ffi::OsString, PathBuf), DaemonError> {
    for attempt in 0u32.. {
        let mut name = base.to_vec();
        if attempt > 0 {
            name.extend_from_slice(format!(".{attempt}").as_bytes());
        }
        let name = std::ffi::OsString::from_vec(name);
        if std::fs::symlink_metadata(root.join("files").join(&name)).is_ok() {
            continue;
        }
        let mut record_name = name.clone();
        record_name.push(".trashinfo");
        let info = root.join("info").join(record_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&info)
        {
            Ok(mut file) => {
                let text = format!(
                    "[Trash Info]\nPath={}\nDeletionDate={}\n",
                    encode(recorded.as_os_str().as_bytes()),
                    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
                );
                return match file.write_all(text.as_bytes()) {
                    Ok(()) => Ok((name, info)),
                    Err(error) => {
                        let _ = std::fs::remove_file(&info);
                        Err(io_failure(&info, &error))
                    }
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_failure(&info, &error)),
        }
    }
    unreachable!("an unbounded range ends only by returning")
}

fn read_original(record: &Path, topdir: Option<&Path>) -> Result<PathBuf, DaemonError> {
    let damaged = || {
        fail(
            "file_not_in_trash",
            format!(
                "The Trash has no readable record of where {} came from.",
                record.display()
            ),
        )
    };
    let text = std::fs::read_to_string(record).map_err(|_| damaged())?;
    let encoded = text
        .lines()
        .find_map(|line| line.strip_prefix("Path="))
        .ok_or_else(damaged)?;
    let path = PathBuf::from(std::ffi::OsString::from_vec(
        decode(encoded).ok_or_else(damaged)?,
    ));
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(topdir.ok_or_else(damaged)?.join(path))
    }
}

/// Percent-encoding of the bytes of a path; `/` and the unreserved characters stay as they are.
fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    Some(out)
}
