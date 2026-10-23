//! MP-08 / MP-10 / MP-11: instruction reads stay beneath pinned directory handles.
//! No path component is followed after source admission, including during races.
use super::*;
use std::{
    ffi::{CStr, CString, OsString},
    fs::{File, Metadata, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::Component,
};
const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 64;

pub(super) fn open_directory(path: &Path) -> Result<File, DaemonError> {
    if !path.is_absolute() {
        return Err(failure("skill source root must be absolute"));
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|_| failure("cannot open skill source root"))?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => directory = open_child(&directory, name, true)?,
            _ => return Err(failure("skill source path must be canonical and contained")),
        }
    }
    Ok(directory)
}
pub(super) fn open_beneath(directory: &File, relative: &Path) -> Result<File, DaemonError> {
    let mut current = directory
        .try_clone()
        .map_err(|_| failure("cannot pin skill source"))?;
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(failure("skill source escapes the admitted workspace"));
        };
        current = open_child(&current, name, true)?;
    }
    Ok(current)
}
fn open_child(parent: &File, name: &std::ffi::OsStr, directory: bool) -> Result<File, DaemonError> {
    let name =
        CString::new(name.as_bytes()).map_err(|_| failure("invalid skill source component"))?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: parent owns its descriptor, name is terminated, and File takes
    // ownership only after a successful openat. Neither symlinks nor FIFOs block.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(failure(
            "skill source changed or contains a symlink; retry with regular contained files",
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn same_state(before: &Metadata, after: &Metadata) -> bool {
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.mode() == after.mode()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}
pub(super) fn collect(directory: &File) -> Result<Vec<CharioxSkillPackageFile>, DaemonError> {
    let instructions = open_child(directory, std::ffi::OsStr::new("SKILL.md"), false)?;
    if !instructions
        .metadata()
        .map_err(|_| failure("cannot inspect SKILL.md"))?
        .is_file()
    {
        return Err(failure("skill source requires a regular SKILL.md"));
    }
    let mut files = vec![];
    let mut bytes = 0;
    let mut entries = 0;
    collect_at(
        directory,
        Path::new(""),
        &mut files,
        &mut bytes,
        &mut entries,
        0,
    )?;
    Ok(files)
}
fn collect_at(
    directory: &File,
    relative: &Path,
    files: &mut Vec<CharioxSkillPackageFile>,
    total: &mut u64,
    entries: &mut usize,
    depth: usize,
) -> Result<(), DaemonError> {
    if depth > MAX_DEPTH {
        return Err(failure("skill package directory nesting exceeds 64"));
    }
    let before = directory
        .metadata()
        .map_err(|_| failure("cannot inspect skill source directory"))?;
    for name in names(directory)? {
        *entries += 1;
        if *entries > MAX_ENTRIES {
            return Err(failure("skill package directory entry limit exceeded"));
        }
        if SKILL_PACKAGE_IGNORED_DIRS
            .iter()
            .any(|ignored| name == *ignored)
        {
            continue;
        }
        let relative = relative.join(&name);
        let path = relative
            .to_str()
            .ok_or_else(|| failure("skill paths must be UTF-8"))?;
        validate_package_relative_path(path)?;
        let mut input = open_child(directory, &name, false)?;
        let initial = input
            .metadata()
            .map_err(|_| failure("cannot inspect skill source"))?;
        if initial.is_dir() {
            collect_at(&input, &relative, files, total, entries, depth + 1)?;
            continue;
        }
        if !initial.is_file() {
            return Err(failure("skill packages require regular files"));
        }
        if files.len() >= MAX_SKILL_PACKAGE_FILES {
            return Err(failure("skill package exceeds maximum file count (512)"));
        }
        if initial.len() > MAX_SKILL_PACKAGE_BYTES.saturating_sub(*total) {
            return Err(failure("skill package exceeds maximum byte size (10 MiB)"));
        }
        let mut bytes = vec![];
        Read::by_ref(&mut input)
            .take(MAX_SKILL_PACKAGE_BYTES.saturating_sub(*total) + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("cannot read skill source"))?;
        let after = input
            .metadata()
            .map_err(|_| failure("cannot inspect skill source"))?;
        if !same_state(&initial, &after) {
            return Err(failure("skill source changed while reading; preview again"));
        }
        if crate::extension_package_security::contains_private_key(&bytes) {
            return Err(failure("skill package contains private key material"));
        }
        *total += bytes.len() as u64;
        if *total > MAX_SKILL_PACKAGE_BYTES {
            return Err(failure("skill package exceeds maximum byte size (10 MiB)"));
        }
        files.push(CharioxSkillPackageFile {
            executable: initial.mode() & 0o111 != 0,
            path: path.into(),
            sha256: sha256_hex(&bytes),
            content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    let after = directory
        .metadata()
        .map_err(|_| failure("cannot inspect skill source directory"))?;
    if !same_state(&before, &after) {
        return Err(failure("skill source changed while reading; preview again"));
    }
    Ok(())
}
struct DirectoryStream(*mut libc::DIR);
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0) };
    }
}
fn names(directory: &File) -> Result<Vec<OsString>, DaemonError> {
    let descriptor = open_child(directory, std::ffi::OsStr::new("."), true)?.into_raw_fd();
    let stream = unsafe { libc::fdopendir(descriptor) };
    if stream.is_null() {
        unsafe { libc::close(descriptor) };
        return Err(failure("cannot list skill directory"));
    }
    let stream = DirectoryStream(stream);
    let mut names = vec![];
    loop {
        set_errno(0);
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            if errno() != 0 {
                return Err(failure("cannot list skill directory"));
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        names.push(OsString::from_vec(name.to_vec()));
        if names.len() > MAX_ENTRIES {
            return Err(failure("skill package directory entry limit exceeded"));
        }
    }
    names.sort();
    Ok(names)
}
#[cfg(any(target_os = "linux", target_os = "android"))]
fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}
#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_errno(value: i32) {
    unsafe { *libc::__errno_location() = value }
}
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn errno() -> i32 {
    unsafe { *libc::__error() }
}
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn set_errno(value: i32) {
    unsafe { *libc::__error() = value }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "skillsk-pinned-source-{:032x}",
                rand::random::<u128>()
            ));
            fs::create_dir_all(root.join("selected")).unwrap();
            fs::create_dir_all(root.join("outside")).unwrap();
            for name in ["selected", "outside"] {
                fs::write(root.join(name).join("SKILL.md"),format!("---\nname: review\ndescription: reviewed source\n---\n{name} instructions\n")).unwrap();
            }
            Self(fs::canonicalize(root).unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn skill_pinned_source_survives_ancestor_replacement_without_reading_replacement() {
        let f = Fixture::new();
        let selected = f.0.join("selected");
        let pinned = open_directory(&selected).unwrap();
        let before = collect(&pinned).unwrap();
        fs::rename(&selected, f.0.join("parked")).unwrap();
        std::os::unix::fs::symlink(f.0.join("outside"), &selected).unwrap();
        let after = collect(&pinned).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            collect(&pinned).unwrap(),
            before,
            "directory enumeration does not consume the pinned handle offset"
        );
        assert!(open_directory(&selected).is_err());
    }
    #[test]
    fn skill_pinned_source_rejects_nested_symlinks_before_reading_files() {
        let f = Fixture::new();
        let selected = f.0.join("selected");
        let pinned = open_directory(&selected).unwrap();
        std::os::unix::fs::symlink(f.0.join("outside"), selected.join("references")).unwrap();
        assert!(collect(&pinned).is_err());
    }
    #[test]
    fn skill_agent_folder_admission_binds_relative_paths_to_its_workspace() {
        let f = Fixture::new();
        let package =
            super::super::package_skill_directory_beneath(&f.0, Path::new("selected")).unwrap();
        assert!(package.metadata.path.starts_with(f.0.join("selected")));
        assert!(super::super::package_skill_directory_beneath(
            &f.0.join("selected"),
            &f.0.join("outside")
        )
        .is_err());
        std::os::unix::fs::symlink(f.0.join("outside"), f.0.join("selected/escape")).unwrap();
        assert!(super::super::package_skill_directory_beneath(
            &f.0.join("selected"),
            Path::new("escape")
        )
        .is_err());
    }
}
