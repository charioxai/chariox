use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::contract::hex_digest;
use super::contract::{
    EnvironmentContract, EnvironmentIdentity, LockfileDigest, RepositoryManifest,
};
use super::policy::{
    normalize_relative_path, ProjectEnvironmentError, MANIFEST_RELATIVE_PATH, MAX_LOCKFILE_BYTES,
    MAX_MANIFEST_BYTES,
};

/// Read and validate the repository's declarative environment manifest.
///
/// Discovery only reads bounded data files. It does not walk the repository,
/// spawn a process, interpret instructions, or run validation probes.
pub fn discover(
    repository_root: impl AsRef<Path>,
    identity: EnvironmentIdentity,
) -> Result<EnvironmentContract, ProjectEnvironmentError> {
    let repository_root = repository_root.as_ref();
    ensure_repository_root(repository_root)?;

    let manifest_bytes =
        read_bounded_file(repository_root, MANIFEST_RELATIVE_PATH, MAX_MANIFEST_BYTES)?;
    let manifest = RepositoryManifest::from_toml_bytes(&manifest_bytes)?;
    for probe in &manifest.validation {
        ensure_path_safety_if_present(repository_root, &probe.path)?;
    }

    let mut lockfile_digests = Vec::with_capacity(manifest.lockfiles.len());
    for path in &manifest.lockfiles {
        if path == MANIFEST_RELATIVE_PATH {
            return Err(ProjectEnvironmentError::InvalidPath {
                path: path.clone(),
                reason: "the repository manifest cannot also be a lockfile".to_owned(),
            });
        }
        let bytes = read_bounded_file(repository_root, path, MAX_LOCKFILE_BYTES)?;
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
    root: &Path,
    relative_path: &str,
    limit: usize,
) -> Result<Vec<u8>, ProjectEnvironmentError> {
    let path = checked_existing_path(root, relative_path)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|source| ProjectEnvironmentError::InspectPath {
            path: path.clone(),
            source,
        })?;
    if metadata.len() > limit as u64 {
        return Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository file bytes",
            limit,
        });
    }

    let mut file = File::open(&path).map_err(|source| ProjectEnvironmentError::ReadFile {
        path: path.clone(),
        source,
    })?;
    let mut bytes = Vec::with_capacity(metadata.len().min(limit as u64) as usize);
    file.by_ref()
        .take((limit as u64).saturating_add(1))
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
    Ok(bytes)
}

fn checked_existing_path(
    root: &Path,
    relative_path: &str,
) -> Result<PathBuf, ProjectEnvironmentError> {
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
    Ok(path)
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
