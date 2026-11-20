//! The freedesktop.org Trash specification: `files/` holds the items, `info/<name>.trashinfo` says
//! where each came from. The home trash serves its own filesystem; an item on another one goes to that
//! filesystem's `$topdir/.Trash/$uid` (when the administrator made `.Trash`: a real folder with the
//! sticky bit) or else `$topdir/.Trash-$uid`, so a move is always a rename and never a copy.
//!
//! Every trash folder used is the user's alone: owned by the user, closed to everyone else, not a
//! link. Folders are opened one handle at a time (`Dir`), so what was checked is what is used. A
//! location that fails a check is passed over for the next allowed one; when none is left the move
//! fails and nothing moved.
use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Component, Path, PathBuf};

use super::dir::Dir;
use crate::error::DaemonError;
use crate::runtime::kernel_files::{fail, io_failure};

const STICKY: u32 = 0o1000;
/// The longest name a file has on the filesystems of the specification's trash.
const NAME_MAX: usize = 255;
const RECORD_SUFFIX: &str = ".trashinfo";

type DeviceOf = Box<dyn Fn(&Path) -> std::io::Result<u64> + Send + Sync>;

pub(super) struct Trash {
    home_trash: PathBuf,
    uid: u32,
    /// The filesystem a path is on; a test says which paths are another one.
    device_of: DeviceOf,
}

/// The trash folder a path lies in, read from the path's shape.
enum Root<'a> {
    /// The home trash.
    Home(&'a Path),
    /// `$topdir/.Trash-$uid`.
    Own { root: &'a Path, topdir: &'a Path },
    /// `$topdir/.Trash/$uid`.
    Shared { root: &'a Path, topdir: &'a Path },
}

/// An item in a trash folder: the entry of `files/` that has the record, and the path inside that entry.
struct Located<'a> {
    root: Root<'a>,
    entry: &'a OsStr,
    inside: Vec<&'a OsStr>,
}

/// One trash folder, opened and checked, and the mount point its `Path=` lines are relative to.
struct Folder {
    dir: Dir,
    root: PathBuf,
    topdir: Option<PathBuf>,
}

fn real_device(path: &Path) -> std::io::Result<u64> {
    std::fs::metadata(path).map(|metadata| std::os::unix::fs::MetadataExt::dev(&metadata))
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
        Self::on_devices(home_trash, uid, Box::new(real_device))
    }

    pub(super) fn on_devices(home_trash: PathBuf, uid: u32, device_of: DeviceOf) -> Self {
        Self {
            home_trash,
            uid,
            device_of,
        }
    }

    /// Whether `path` lies in one of the trash folders this machine keeps: the home trash, or a `.Trash`
    /// or `.Trash-$uid` right at the top of a filesystem. A user's own folder with such a name elsewhere
    /// is not a trash.
    pub(super) fn holds(&self, path: &Path) -> bool {
        path.starts_with(&self.home_trash)
            || path.ancestors().any(|at| {
                let is_trash_name = at.file_name().is_some_and(|name| {
                    name == ".Trash" || name.as_bytes() == self.own_name().as_bytes()
                });
                is_trash_name && at.parent().is_some_and(|topdir| self.is_top(topdir))
            })
    }

    fn own_name(&self) -> OsString {
        OsString::from(format!(".Trash-{}", self.uid))
    }

    /// Whether `dir` is the top of its filesystem, the folder its trash lies in.
    fn is_top(&self, dir: &Path) -> bool {
        match dir.parent() {
            None => true,
            Some(above) => match ((self.device_of)(dir), (self.device_of)(above)) {
                (Ok(here), Ok(there)) => here != there,
                _ => false,
            },
        }
    }

    /// Where `path` lies in a trash of this user, from the path's shape alone: the home trash when the
    /// path is in it, else the outermost `.Trash-$uid` or `.Trash/$uid` at the top of a filesystem; then
    /// `files/`, the entry, and the path inside the entry. Names below the trash folder are never
    /// matched, so the items in it may have any names (`files`, `info`, `.Trash-$uid`, ...).
    fn locate<'a>(&self, path: &'a Path) -> Option<Located<'a>> {
        let root = match path.ancestors().find(|at| *at == self.home_trash) {
            Some(home) => Root::Home(home),
            None => {
                let mut outer_first: Vec<&Path> = path.ancestors().collect();
                outer_first.reverse();
                outer_first.into_iter().find_map(|at| self.trash_root(at))?
            }
        };
        let root_path = match root {
            Root::Home(root) | Root::Own { root, .. } | Root::Shared { root, .. } => root,
        };
        let mut parts = path
            .strip_prefix(root_path)
            .ok()?
            .components()
            .map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            });
        if parts.next()?? != "files" {
            return None;
        }
        let entry = parts.next()??;
        let inside = parts.collect::<Option<Vec<_>>>()?;
        Some(Located {
            root,
            entry,
            inside,
        })
    }

    /// `at` as a trash folder of this user at the top of a filesystem, when it is one by its place.
    fn trash_root<'a>(&self, at: &'a Path) -> Option<Root<'a>> {
        let (name, parent) = (at.file_name()?, at.parent()?);
        let root = if name.as_bytes() == self.own_name().as_bytes() {
            Root::Own {
                root: at,
                topdir: parent,
            }
        } else if name.as_bytes() == self.uid.to_string().as_bytes()
            && parent.file_name() == Some(".Trash".as_ref())
        {
            Root::Shared {
                root: at,
                topdir: parent.parent()?,
            }
        } else {
            return None;
        };
        match root {
            Root::Own { topdir, .. } | Root::Shared { topdir, .. } if self.is_top(topdir) => {
                Some(root)
            }
            _ => None,
        }
    }

    pub(super) fn move_to(&self, path: &Path) -> Result<PathBuf, DaemonError> {
        let name = path
            .file_name()
            .ok_or_else(|| fail("file_path_invalid", "A Trash item needs a name."))?;
        let parent = std::fs::canonicalize(path.parent().unwrap_or(path))
            .map_err(|error| io_failure(path, &error))?;
        let parent_dir = Dir::open(&parent).map_err(|error| io_failure(&parent, &error))?;
        let device = (self.device_of)(&parent).map_err(|error| io_failure(&parent, &error))?;
        let mut problems = Vec::new();
        if let Some(folder) = self.pick(&parent, device, &mut problems) {
            return self.put(&parent_dir, &parent, path, name, folder);
        }
        Err(fail(
            "file_trash_failed",
            format!(
                "{} could not be moved to a Trash that only you can use ({}). Nothing was moved.",
                path.display(),
                problems.join("; ")
            ),
        ))
    }

    /// The first trash folder that may take an item of `parent` (on `device`), in the order of the
    /// specification: the home trash when it is on that filesystem, then `$topdir/.Trash/$uid`, then
    /// `$topdir/.Trash-$uid`. A location that fails a check adds its reason to `problems`.
    fn pick(&self, parent: &Path, device: u64, problems: &mut Vec<String>) -> Option<Folder> {
        match self.home_folder(device) {
            Ok(Some(folder)) => return Some(folder),
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
        let mut topdir = parent;
        while let Some(above) = topdir.parent() {
            if (self.device_of)(above).is_ok_and(|other| other == device) {
                topdir = above;
            } else {
                break;
            }
        }
        let top = match Dir::open(topdir) {
            Ok(top) if (self.device_of)(topdir).is_ok_and(|other| other == device) => top,
            _ => {
                problems.push(format!("{} cannot be opened", topdir.display()));
                return None;
            }
        };
        match self.shared_folder(&top, topdir) {
            Ok(Some(folder)) => return Some(folder),
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
        match self.own_folder(&top, topdir) {
            Ok(folder) => Some(folder),
            Err(problem) => {
                problems.push(problem);
                None
            }
        }
    }

    /// The home trash, made if needed, when it is on `device`.
    fn home_folder(&self, device: u64) -> Result<Option<Folder>, String> {
        let problem = |what: &str, error: std::io::Error| {
            format!("{} {what}: {error}", self.home_trash.display())
        };
        let (Some(data_home), Some(name)) = (self.home_trash.parent(), self.home_trash.file_name())
        else {
            return Err(format!("{} has no folder", self.home_trash.display()));
        };
        std::fs::create_dir_all(data_home).map_err(|error| problem("cannot be made", error))?;
        let data_home = std::fs::canonicalize(data_home)
            .map_err(|error| problem("cannot be reached", error))?;
        let parent = Dir::open(&data_home).map_err(|error| problem("cannot be reached", error))?;
        parent
            .make_at(name, 0o700)
            .map_err(|error| problem("cannot be made", error))?;
        let dir = parent
            .open_at(name)
            .map_err(|error| problem("is a link or not a folder", error))?;
        if !dir
            .is_private_to(self.uid)
            .map_err(|error| problem("cannot be checked", error))?
        {
            return Err(format!(
                "{} belongs to someone else",
                self.home_trash.display()
            ));
        }
        if (self.device_of)(&data_home.join(name)).is_ok_and(|other| other != device) {
            return Ok(None);
        }
        Ok(Some(Folder {
            dir,
            root: self.home_trash.clone(),
            topdir: None,
        }))
    }

    /// `$topdir/.Trash/$uid`, when the administrator made `.Trash` a real folder with the sticky bit.
    fn shared_folder(&self, top: &Dir, topdir: &Path) -> Result<Option<Folder>, String> {
        let shared_name = OsStr::new(".Trash");
        let shared_path = topdir.join(shared_name);
        let shared = match top.open_at(shared_name) {
            Ok(shared) => shared,
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => return Ok(None),
            Err(_) => {
                return Err(format!(
                    "{} is a link or not a folder",
                    shared_path.display()
                ))
            }
        };
        let sticky = shared.stat().is_ok_and(|stat| stat.st_mode & STICKY != 0);
        if !sticky {
            return Err(format!(
                "{} does not have the sticky bit",
                shared_path.display()
            ));
        }
        let uid_name = OsString::from(self.uid.to_string());
        let own_path = shared_path.join(&uid_name);
        let problem = |what: &str| format!("{} {what}", own_path.display());
        shared
            .make_at(&uid_name, 0o700)
            .map_err(|_| problem("cannot be made"))?;
        let dir = shared
            .open_at(&uid_name)
            .map_err(|_| problem("is a link or not a folder"))?;
        if !dir.is_private_to(self.uid).unwrap_or(false) {
            return Err(problem("belongs to someone else"));
        }
        Ok(Some(Folder {
            dir,
            root: own_path,
            topdir: Some(topdir.to_path_buf()),
        }))
    }

    /// `$topdir/.Trash-$uid`, made if needed.
    fn own_folder(&self, top: &Dir, topdir: &Path) -> Result<Folder, String> {
        let name = self.own_name();
        let path = topdir.join(&name);
        let problem = |what: &str| format!("{} {what}", path.display());
        top.make_at(&name, 0o700)
            .map_err(|_| problem("cannot be made"))?;
        let dir = top
            .open_at(&name)
            .map_err(|_| problem("is a link or not a folder"))?;
        if !dir.is_private_to(self.uid).unwrap_or(false) {
            return Err(problem("belongs to someone else"));
        }
        Ok(Folder {
            dir,
            root: path,
            topdir: Some(topdir.to_path_buf()),
        })
    }

    /// Moves `name` of `parent_dir` into `folder` under a free name and records where it came from.
    fn put(
        &self,
        parent_dir: &Dir,
        parent: &Path,
        path: &Path,
        name: &OsStr,
        folder: Folder,
    ) -> Result<PathBuf, DaemonError> {
        let failed = |what: &str, error: std::io::Error| {
            fail(
                "file_trash_failed",
                format!(
                    "{} could not be moved to the Trash: {what}: {error}.",
                    path.display()
                ),
            )
        };
        parent_dir
            .stat_at(name)
            .map_err(|error| io_failure(path, &error))?;
        let parts = |part: &str| -> Result<Dir, DaemonError> {
            folder
                .dir
                .make_at(OsStr::new(part), 0o700)
                .map_err(|e| failed("the Trash folder is not usable", e))?;
            let dir = folder
                .dir
                .open_at(OsStr::new(part))
                .map_err(|e| failed("the Trash folder is not usable", e))?;
            match dir.is_private_to(self.uid) {
                Ok(true) => Ok(dir),
                _ => Err(fail(
                    "file_trash_failed",
                    format!(
                        "{} is not yours alone. Nothing was moved.",
                        folder.root.join(part).display()
                    ),
                )),
            }
        };
        let (files, info) = (parts("files")?, parts("info")?);
        let recorded = match &folder.topdir {
            Some(topdir) => parent.strip_prefix(topdir).unwrap_or(parent).join(name),
            None => path.to_path_buf(),
        };
        for attempt in 0u32.. {
            let candidate = trash_name(name, attempt);
            if files.stat_at(&candidate).is_ok() {
                continue;
            }
            let mut record = candidate.clone();
            record.push(RECORD_SUFFIX);
            let mut file = match info.create_at(&record) {
                Ok(file) => file,
                Err(error) if error.raw_os_error() == Some(libc::EEXIST) => continue,
                Err(error) => return Err(failed("the Trash record cannot be made", error)),
            };
            let text = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                encode(recorded.as_os_str().as_bytes()),
                chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
            );
            if let Err(error) = file.write_all(text.as_bytes()) {
                let _ = info.unlink_at(&record);
                return Err(failed("the Trash record cannot be written", error));
            }
            return match parent_dir.move_into(name, &files, &candidate) {
                Ok(()) => Ok(folder.root.join("files").join(&candidate)),
                Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                    let _ = info.unlink_at(&record);
                    continue;
                }
                Err(error) => {
                    let _ = info.unlink_at(&record);
                    Err(failed("the move failed", error))
                }
            };
        }
        unreachable!("an unbounded range ends only by returning")
    }

    /// Puts the item at `trash_path` back at `wanted`, when the Trash records it as that. Returns where it is.
    pub(super) fn restore(&self, trash_path: &Path, wanted: &Path) -> Result<PathBuf, DaemonError> {
        let not_trash = |what: &str| {
            fail(
                "file_not_in_trash",
                format!("{} {what}.", trash_path.display()),
            )
        };
        // The item is an entry of `files/`, or something inside one: the record belongs to the entry,
        // and the path inside it is added to the recorded place.
        let Located {
            root,
            entry: name,
            inside,
        } = self
            .locate(trash_path)
            .ok_or_else(|| not_trash("is not an item in the Trash"))?;
        let (folder, topdir) = self
            .open_root(&root)
            .ok_or_else(|| not_trash("is not an item in a Trash folder that only you can use"))?;
        let (files, info) = match (
            folder.open_at("files".as_ref()),
            folder.open_at("info".as_ref()),
        ) {
            (Ok(files), Ok(info)) => (files, info),
            _ => return Err(not_trash("is not an item in the Trash")),
        };
        let gone = || {
            fail(
                "file_not_in_trash",
                format!("{} is no longer in the Trash.", wanted.display()),
            )
        };
        if files.stat_at(name).is_err() {
            return Err(gone());
        }
        let mut record = name.to_os_string();
        record.push(RECORD_SUFFIX);
        let recorded = info
            .read_at(&record, self.uid)
            .ok()
            .and_then(|text| original_of(&text, topdir.as_deref()))
            .ok_or_else(|| {
                fail(
                    "file_not_in_trash",
                    format!(
                        "The Trash has no readable record of where {} came from.",
                        trash_path.display()
                    ),
                )
            })?;
        let recorded = inside.iter().fold(recorded, |path, part| path.join(part));
        // The folder that holds the item to move, and the item's name in it.
        let mut holder = None;
        if !inside.is_empty() {
            let mut dir = files.open_at(name).map_err(|_| gone())?;
            for part in &inside[..inside.len() - 1] {
                dir = dir.open_at(part).map_err(|_| gone())?;
            }
            if dir.stat_at(inside[inside.len() - 1]).is_err() {
                return Err(gone());
            }
            holder = Some(dir);
        }
        let (from, from_name) = match (&holder, inside.last()) {
            (Some(dir), Some(last)) => (dir, *last),
            _ => (&files, name),
        };
        if !same_place(&recorded, wanted) {
            return Err(fail(
                "file_not_in_trash",
                format!(
                    "The Trash keeps {} as {}, not {}.",
                    trash_path.display(),
                    recorded.display(),
                    wanted.display()
                ),
            ));
        }
        let (parent, original_name) = match (wanted.parent(), wanted.file_name()) {
            (Some(parent), Some(name)) => (parent, name),
            _ => return Err(fail("file_path_invalid", "A restored item needs a name.")),
        };
        std::fs::create_dir_all(parent).map_err(|error| io_failure(parent, &error))?;
        let parent_path =
            std::fs::canonicalize(parent).map_err(|error| io_failure(parent, &error))?;
        let parent_dir = Dir::open(&parent_path).map_err(|error| io_failure(parent, &error))?;
        from.move_into(from_name, &parent_dir, original_name).map_err(|error| {
            if matches!(error.raw_os_error(), Some(libc::EEXIST | libc::ENOTEMPTY)) {
                fail(
                    "file_restore_conflict",
                    format!(
                        "{} is taken, so the item stays in the Trash. Move or rename what is there, then restore it again.",
                        wanted.display()
                    ),
                )
            } else {
                io_failure(wanted, &error)
            }
        })?;
        if inside.is_empty() {
            let _ = info.unlink_at(&record);
        }
        Ok(wanted.to_path_buf())
    }

    /// The trash folder `root`, opened and checked as a trash of this user, and its mount point;
    /// `None` when it fails a check.
    fn open_root(&self, root: &Root) -> Option<(Dir, Option<PathBuf>)> {
        let private = |dir: Dir| dir.is_private_to(self.uid).ok()?.then_some(dir);
        let open = |path: &Path| Dir::open(&std::fs::canonicalize(path).ok()?).ok();
        match *root {
            Root::Home(path) => {
                let dir = open(path.parent()?)?.open_at(path.file_name()?).ok()?;
                private(dir).map(|dir| (dir, None))
            }
            Root::Own { root, topdir } => {
                let dir = open(topdir)?.open_at(root.file_name()?).ok()?;
                private(dir).map(|dir| (dir, Some(topdir.to_path_buf())))
            }
            Root::Shared { root, topdir } => {
                let shared = open(topdir)?.open_at(".Trash".as_ref()).ok()?;
                if shared.stat().ok()?.st_mode & STICKY == 0 {
                    return None;
                }
                let dir = shared.open_at(root.file_name()?).ok()?;
                private(dir).map(|dir| (dir, Some(topdir.to_path_buf())))
            }
        }
    }
}

/// The name an item has in `files/`: its own, with `.N` after a collision, cut so that the record
/// `<name>.trashinfo` is still a file name. The record keeps the full original name.
fn trash_name(name: &OsStr, attempt: u32) -> OsString {
    let suffix = if attempt > 0 {
        format!(".{attempt}")
    } else {
        String::new()
    };
    let room = NAME_MAX - RECORD_SUFFIX.len() - suffix.len();
    let mut cut = match name.to_str() {
        // Cut between characters, so the name stays text and the path reported for it names it exactly.
        Some(text) => OsString::from(&text[..text.floor_char_boundary(room)]),
        // Not text to begin with: any bytes but `/` and NUL are a name here.
        None => OsStr::from_bytes(&name.as_bytes()[..name.len().min(room)]).to_os_string(),
    };
    cut.push(suffix);
    cut
}

/// Whether two paths name the same place: the same name in the same folder, links above it resolved.
fn same_place(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.file_name(), b.file_name(), a.parent(), b.parent()) {
        (Some(x), Some(y), Some(pa), Some(pb)) if x == y => {
            matches!((std::fs::canonicalize(pa), std::fs::canonicalize(pb)), (Ok(ca), Ok(cb)) if ca == cb)
        }
        _ => false,
    }
}

fn original_of(record: &str, topdir: Option<&Path>) -> Option<PathBuf> {
    let encoded = record.lines().find_map(|line| line.strip_prefix("Path="))?;
    let path = PathBuf::from(OsString::from_vec(decode(encoded)?));
    if path.is_absolute() {
        Some(path)
    } else {
        Some(topdir?.join(path))
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
