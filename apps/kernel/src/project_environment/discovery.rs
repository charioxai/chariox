use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use sha2::{Digest, Sha256};

use super::contract::hex_digest;
use super::contract::{
    EnvironmentContract, EnvironmentIdentity, LockfileDigest, RepositoryManifest,
};
use super::policy::{
    normalize_relative_path, ProjectEnvironmentError, MANIFEST_RELATIVE_PATH, MAX_LOCKFILE_BYTES,
    MAX_MANIFEST_BYTES, MAX_TOTAL_REPOSITORY_BYTES,
};

/// Read and validate the repository's declarative environment manifest.
///
/// Discovery only reads bounded data files. It does not walk the repository,
/// spawn a process, interpret instructions, or run validation probes.
pub fn discover(
    repository_root: impl AsRef<Path>,
    identity: EnvironmentIdentity,
) -> Result<EnvironmentContract, ProjectEnvironmentError> {
    let repository_root = canonical_repository_root(repository_root.as_ref())?;

    let mut total_bytes = 0;
    let manifest_bytes = read_bounded_file(
        &repository_root,
        MANIFEST_RELATIVE_PATH,
        MAX_MANIFEST_BYTES,
        MAX_TOTAL_REPOSITORY_BYTES,
        total_bytes,
    )?;
    total_bytes += manifest_bytes.len();
    let manifest = RepositoryManifest::from_toml_bytes(&manifest_bytes)?;
    for probe in &manifest.validation {
        ensure_path_safety_if_present(&repository_root.path, &probe.path)?;
    }

    let mut lockfile_digests = Vec::with_capacity(manifest.lockfiles.len());
    for path in &manifest.lockfiles {
        if path == MANIFEST_RELATIVE_PATH {
            return Err(ProjectEnvironmentError::InvalidPath {
                path: path.clone(),
                reason: "the repository manifest cannot also be a lockfile".to_owned(),
            });
        }
        let bytes = read_bounded_file(
            &repository_root,
            path,
            MAX_LOCKFILE_BYTES,
            MAX_TOTAL_REPOSITORY_BYTES,
            total_bytes,
        )?;
        total_bytes += bytes.len();
        lockfile_digests.push(LockfileDigest {
            path: path.clone(),
            digest: hex_digest(&Sha256::digest(bytes)),
        });
    }

    let identity = identity.validate_and_normalize()?;
    Ok(EnvironmentContract::assemble(
        identity,
        manifest,
        lockfile_digests,
    ))
}

pub fn discover_repository_environment(
    repository_root: impl AsRef<Path>,
    identity: EnvironmentIdentity,
) -> Result<EnvironmentContract, ProjectEnvironmentError> {
    discover(repository_root, identity)
}

pub(super) struct CanonicalRepositoryRoot {
    path: PathBuf,
    directory: File,
}

pub(super) fn canonical_repository_root(
    root: &Path,
) -> Result<CanonicalRepositoryRoot, ProjectEnvironmentError> {
    ensure_repository_root(root)?;
    let canonical =
        fs::canonicalize(root).map_err(|source| ProjectEnvironmentError::InspectPath {
            path: root.to_path_buf(),
            source,
        })?;
    ensure_repository_root(&canonical)?;
    let expected_metadata = fs::symlink_metadata(&canonical).map_err(|source| {
        ProjectEnvironmentError::InspectPath {
            path: canonical.clone(),
            source,
        }
    })?;
    let directory = open_root_directory(&canonical)?;
    let directory_metadata =
        directory
            .metadata()
            .map_err(|source| ProjectEnvironmentError::InspectPath {
                path: canonical.clone(),
                source,
            })?;
    if !same_file_identity(&expected_metadata, &directory_metadata) {
        return Err(ProjectEnvironmentError::ConcurrentReplacement { path: canonical });
    }
    Ok(CanonicalRepositoryRoot {
        path: canonical,
        directory,
    })
}

fn ensure_repository_root(root: &Path) -> Result<(), ProjectEnvironmentError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectEnvironmentError::MissingFile {
                path: root.to_path_buf(),
            }
        } else {
            ProjectEnvironmentError::InspectPath {
                path: root.to_path_buf(),
                source,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ProjectEnvironmentError::SymlinkPath {
            path: root.to_path_buf(),
        });
    }
    if !metadata.is_dir() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: root.to_path_buf(),
        });
    }
    Ok(())
}

fn read_bounded_file(
    root: &CanonicalRepositoryRoot,
    relative_path: &str,
    limit: usize,
    aggregate_limit: usize,
    bytes_already_read: usize,
) -> Result<Vec<u8>, ProjectEnvironmentError> {
    verify_root_identity(root)?;
    let (path, expected_metadata) = checked_existing_path(&root.path, relative_path)?;
    let mut opened = open_declared_file(root, relative_path, &path)?;
    let metadata = verify_opened_file_against(root, &path, &opened, Some(&expected_metadata))?;
    if metadata.len() > limit as u64 {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository file bytes",
            limit,
        });
    }
    let aggregate_remaining = aggregate_limit.saturating_sub(bytes_already_read);
    if metadata.len() > aggregate_remaining as u64 {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository aggregate bytes",
            limit: aggregate_limit,
        });
    }

    let read_limit = limit.min(aggregate_remaining);
    let mut bytes = Vec::with_capacity(metadata.len().min(read_limit as u64) as usize);
    opened
        .file
        .by_ref()
        .take((read_limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| ProjectEnvironmentError::ReadFile {
            path: path.clone(),
            source,
        })?;
    if bytes.len() > limit {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository file bytes",
            limit,
        });
    }
    if bytes.len() > aggregate_remaining {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository aggregate bytes",
            limit: aggregate_limit,
        });
    }
    verify_opened_file_against(root, &path, &opened, Some(&expected_metadata))?;
    verify_root_identity(root)?;
    Ok(bytes)
}

fn verify_root_identity(root: &CanonicalRepositoryRoot) -> Result<(), ProjectEnvironmentError> {
    let path_metadata = fs::symlink_metadata(&root.path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectEnvironmentError::MissingFile {
                path: root.path.clone(),
            }
        } else {
            ProjectEnvironmentError::InspectPath {
                path: root.path.clone(),
                source,
            }
        }
    })?;
    if path_metadata.file_type().is_symlink() {
        return Err(ProjectEnvironmentError::SymlinkPath {
            path: root.path.clone(),
        });
    }
    if !path_metadata.is_dir() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: root.path.clone(),
        });
    }
    let directory_metadata =
        root.directory
            .metadata()
            .map_err(|source| ProjectEnvironmentError::InspectPath {
                path: root.path.clone(),
                source,
            })?;
    if !same_file_identity(&path_metadata, &directory_metadata) {
        return Err(ProjectEnvironmentError::ConcurrentReplacement {
            path: root.path.clone(),
        });
    }
    Ok(())
}

fn ensure_canonical_path_inside(
    root: &CanonicalRepositoryRoot,
    path: &Path,
) -> Result<(), ProjectEnvironmentError> {
    let canonical = fs::canonicalize(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectEnvironmentError::MissingFile {
                path: path.to_path_buf(),
            }
        } else {
            ProjectEnvironmentError::InspectPath {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if !canonical.starts_with(&root.path) {
        return Err(ProjectEnvironmentError::PathOutsideRoot { path: canonical });
    }
    Ok(())
}

pub(super) fn verify_opened_file(
    root: &CanonicalRepositoryRoot,
    path: &Path,
    opened: &OpenedDeclaredFile,
) -> Result<Metadata, ProjectEnvironmentError> {
    verify_opened_file_against(root, path, opened, None)
}

fn verify_opened_file_against(
    root: &CanonicalRepositoryRoot,
    path: &Path,
    opened: &OpenedDeclaredFile,
    expected_metadata: Option<&Metadata>,
) -> Result<Metadata, ProjectEnvironmentError> {
    verify_root_identity(root)?;
    verify_opened_parent(root, opened)?;
    ensure_canonical_path_inside(root, path)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectEnvironmentError::MissingFile {
                path: path.to_path_buf(),
            }
        } else {
            ProjectEnvironmentError::InspectPath {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    if path_metadata.file_type().is_symlink() {
        return Err(ProjectEnvironmentError::SymlinkPath {
            path: path.to_path_buf(),
        });
    }
    if !path_metadata.is_file() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: path.to_path_buf(),
        });
    }
    if expected_metadata.is_some_and(|expected| !same_file_identity(expected, &path_metadata)) {
        return Err(ProjectEnvironmentError::ConcurrentReplacement {
            path: path.to_path_buf(),
        });
    }
    let opened_metadata =
        opened
            .file
            .metadata()
            .map_err(|source| ProjectEnvironmentError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
    if !opened_metadata.is_file() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: path.to_path_buf(),
        });
    }
    if !same_file_identity(&path_metadata, &opened_metadata) {
        return Err(ProjectEnvironmentError::ConcurrentReplacement {
            path: path.to_path_buf(),
        });
    }
    Ok(opened_metadata)
}

fn verify_opened_parent(
    root: &CanonicalRepositoryRoot,
    opened: &OpenedDeclaredFile,
) -> Result<(), ProjectEnvironmentError> {
    ensure_canonical_path_inside(root, &opened.parent_path)?;
    let parent_path_metadata = fs::symlink_metadata(&opened.parent_path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ProjectEnvironmentError::MissingFile {
                path: opened.parent_path.clone(),
            }
        } else {
            ProjectEnvironmentError::InspectPath {
                path: opened.parent_path.clone(),
                source,
            }
        }
    })?;
    if parent_path_metadata.file_type().is_symlink() {
        return Err(ProjectEnvironmentError::SymlinkPath {
            path: opened.parent_path.clone(),
        });
    }
    if !parent_path_metadata.is_dir() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: opened.parent_path.clone(),
        });
    }
    let opened_parent_metadata =
        opened
            .parent
            .metadata()
            .map_err(|source| ProjectEnvironmentError::InspectPath {
                path: opened.parent_path.clone(),
                source,
            })?;
    if !opened_parent_metadata.is_dir() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: opened.parent_path.clone(),
        });
    }
    if !same_file_identity(&parent_path_metadata, &opened_parent_metadata) {
        return Err(ProjectEnvironmentError::ConcurrentReplacement {
            path: opened.parent_path.clone(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.volume_serial_number() == right.volume_serial_number()
        && left.file_index() == right.file_index()
}

#[cfg(all(not(unix), not(windows)))]
fn same_file_identity(left: &Metadata, right: &Metadata) -> bool {
    left.is_dir() == right.is_dir()
        && left.is_file() == right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
}

fn map_open_error(path: &Path, source: io::Error) -> ProjectEnvironmentError {
    if source.kind() == io::ErrorKind::NotFound {
        ProjectEnvironmentError::MissingFile {
            path: path.to_path_buf(),
        }
    } else {
        #[cfg(unix)]
        if source.raw_os_error() == Some(libc::ELOOP) {
            return ProjectEnvironmentError::SymlinkPath {
                path: path.to_path_buf(),
            };
        }
        ProjectEnvironmentError::InspectPath {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(unix)]
fn open_root_directory(root: &Path) -> Result<File, ProjectEnvironmentError> {
    use std::os::unix::ffi::OsStrExt;

    let root = CString::new(root.as_os_str().as_bytes()).map_err(|_| {
        ProjectEnvironmentError::InvalidPath {
            path: root.to_string_lossy().into_owned(),
            reason: "path contains a NUL byte".to_owned(),
        }
    })?;
    let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY;
    let fd = unsafe { libc::open(root.as_ptr(), flags) };
    if fd < 0 {
        return Err(map_open_error(
            Path::new("repository root"),
            io::Error::last_os_error(),
        ));
    }
    let directory = unsafe { File::from_raw_fd(fd) };
    let metadata = directory
        .metadata()
        .map_err(|source| ProjectEnvironmentError::InspectPath {
            path: PathBuf::from("repository root"),
            source,
        })?;
    if !metadata.is_dir() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: PathBuf::from("repository root"),
        });
    }
    Ok(directory)
}

#[cfg(unix)]
fn open_at(
    parent: &File,
    component: &str,
    path: &Path,
    directory: bool,
) -> Result<File, ProjectEnvironmentError> {
    let component = CString::new(component).map_err(|_| ProjectEnvironmentError::InvalidPath {
        path: path.to_string_lossy().into_owned(),
        reason: "path contains a NUL byte".to_owned(),
    })?;
    let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    if directory {
        flags |= libc::O_DIRECTORY;
    } else {
        flags |= libc::O_NONBLOCK;
    }
    let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags) };
    if fd < 0 {
        return Err(map_open_error(path, io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
pub(super) struct OpenedDeclaredFile {
    file: File,
    parent: File,
    parent_path: PathBuf,
}

#[cfg(unix)]
pub(super) fn open_declared_file(
    root: &CanonicalRepositoryRoot,
    relative_path: &str,
    path: &Path,
) -> Result<OpenedDeclaredFile, ProjectEnvironmentError> {
    verify_root_identity(root)?;
    let relative_path = normalize_relative_path("repository path", relative_path)?;
    let components = relative_path.split('/').collect::<Vec<_>>();
    let mut parent =
        root.directory
            .try_clone()
            .map_err(|source| ProjectEnvironmentError::InspectPath {
                path: root.path.clone(),
                source,
            })?;
    for component in &components[..components.len() - 1] {
        let child = open_at(&parent, component, path, true)?;
        let metadata = child
            .metadata()
            .map_err(|source| ProjectEnvironmentError::InspectPath {
                path: path.to_path_buf(),
                source,
            })?;
        if !metadata.is_dir() {
            return Err(ProjectEnvironmentError::UnsafeFile {
                path: path.to_path_buf(),
            });
        }
        parent = child;
    }
    let file = open_at(&parent, components[components.len() - 1], path, false)?;
    let metadata = file
        .metadata()
        .map_err(|source| ProjectEnvironmentError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(ProjectEnvironmentError::UnsafeFile {
            path: path.to_path_buf(),
        });
    }
    let parent_path = root.path.join(components[..components.len() - 1].join("/"));
    Ok(OpenedDeclaredFile {
        file,
        parent,
        parent_path,
    })
}

#[cfg(not(unix))]
pub(super) struct OpenedDeclaredFile {
    file: File,
    parent: File,
    parent_path: PathBuf,
}

#[cfg(not(unix))]
fn open_root_directory(_root: &Path) -> Result<File, ProjectEnvironmentError> {
    Err(ProjectEnvironmentError::UnsupportedFilesystem)
}

#[cfg(not(unix))]
pub(super) fn open_declared_file(
    _root: &CanonicalRepositoryRoot,
    _relative_path: &str,
    _path: &Path,
) -> Result<OpenedDeclaredFile, ProjectEnvironmentError> {
    Err(ProjectEnvironmentError::UnsupportedFilesystem)
}

fn checked_existing_path(
    root: &Path,
    relative_path: &str,
) -> Result<(PathBuf, Metadata), ProjectEnvironmentError> {
    let relative_path = normalize_relative_path("repository path", relative_path)?;
    let components = relative_path.split('/').collect::<Vec<_>>();
    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                ProjectEnvironmentError::MissingFile { path: path.clone() }
            } else {
                ProjectEnvironmentError::InspectPath {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ProjectEnvironmentError::SymlinkPath { path: path.clone() });
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(ProjectEnvironmentError::UnsafeFile { path: path.clone() });
        }
    }
    let metadata =
        fs::symlink_metadata(&path).map_err(|source| ProjectEnvironmentError::InspectPath {
            path: path.clone(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(ProjectEnvironmentError::UnsafeFile { path });
    }
    Ok((path, metadata))
}

fn ensure_path_safety_if_present(
    root: &Path,
    relative_path: &str,
) -> Result<(), ProjectEnvironmentError> {
    let relative_path = normalize_relative_path("validation path", relative_path)?;
    let components = relative_path.split('/').collect::<Vec<_>>();
    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        path.push(component);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ProjectEnvironmentError::InspectPath {
                    path: path.clone(),
                    source,
                })
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(ProjectEnvironmentError::SymlinkPath { path: path.clone() });
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(ProjectEnvironmentError::UnsafeFile { path });
        }
    }
    Ok(())
}
