//! Slice-owned blank workspaces use the same broker publication boundary as Project copies.
use super::{
    ensure_private_real_directory, path_to_string, slice_development_error,
    slice_development_io_error, update_managed_publication_access,
};
use crate::error::DaemonError;
use crate::slice::SliceDevelopmentPublication;
use std::fs;
use std::path::Path;

const PUBLICATION: &str = "development";
const STAGING: &str = ".empty-development-staging";
const RECEIPT: &str = ".chariox-empty-workspace";
const RECEIPT_BYTES: &[u8] = b"chariox-empty-slice-workspace-v1\n";

fn publication(parent: &Path) -> Result<SliceDevelopmentPublication, DaemonError> {
    let destination = parent.join(PUBLICATION);
    let workspace = path_to_string(&destination.join("workspace"))?;
    Ok(SliceDevelopmentPublication {
        publication_id: PUBLICATION.into(),
        destination_root: path_to_string(&destination)?,
        primary_repository_path: workspace.clone(),
        repository_paths: vec![workspace],
    })
}

fn real_directory(path: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| slice_development_io_error("inspect", path, error))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || fs::canonicalize(path)
            .map_err(|error| slice_development_io_error("resolve", path, error))?
            != path
    {
        return Err(slice_development_error(
            "empty slice workspace path is not a canonical directory",
        ));
    }
    Ok(())
}

fn validate(parent: &Path) -> Result<(), DaemonError> {
    real_directory(parent)?;
    let destination = parent.join(PUBLICATION);
    real_directory(&destination)?;
    let receipt = destination.join(RECEIPT);
    let metadata = fs::symlink_metadata(&receipt)
        .map_err(|error| slice_development_io_error("inspect", &receipt, error))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != RECEIPT_BYTES.len() as u64
    {
        return Err(slice_development_error(
            "empty slice workspace ownership receipt is invalid",
        ));
    }
    let bytes =
        fs::read(&receipt).map_err(|error| slice_development_io_error("read", &receipt, error))?;
    if bytes != RECEIPT_BYTES {
        return Err(slice_development_error(
            "empty slice workspace ownership receipt is invalid",
        ));
    }
    real_directory(&destination.join("workspace"))
}

pub(super) fn materialize(
    parent: &Path,
    expected: Option<&SliceDevelopmentPublication>,
) -> Result<SliceDevelopmentPublication, DaemonError> {
    let access_action = if expected.is_some() {
        "verify"
    } else {
        "grant"
    };
    if expected.is_none() {
        ensure_private_real_directory(parent)?;
    }
    real_directory(parent)?;
    let publication = publication(parent)?;
    if expected.is_some_and(|expected| expected != &publication) {
        return Err(slice_development_error(
            "empty slice publication differs from durable state",
        ));
    }
    cleanup_staging(parent)?;
    let destination = parent.join(PUBLICATION);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && expected.is_none() => {
            // Publish the receipt and workspace together. The agent can write only workspace/.
            let staging = parent.join(STAGING);
            fs::create_dir(&staging)
                .map_err(|error| slice_development_io_error("create", &staging, error))?;
            let result = (|| {
                ensure_private_real_directory(&staging)?;
                ensure_private_real_directory(&staging.join("workspace"))?;
                fs::write(staging.join(RECEIPT), RECEIPT_BYTES).map_err(|error| {
                    slice_development_io_error("write receipt", &staging, error)
                })?;
                fs::rename(&staging, &destination)
                    .map_err(|error| slice_development_io_error("publish", &destination, error))
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            result?;
        }
        Err(error) => return Err(slice_development_io_error("recover", &destination, error)),
    }
    validate(parent)?;
    update_managed_publication_access(access_action, parent, &publication)?;
    Ok(publication)
}

pub(super) fn cleanup(parent: &Path) -> Result<(), DaemonError> {
    match fs::symlink_metadata(parent) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(slice_development_io_error("inspect", parent, error)),
        Ok(_) => {}
    }
    real_directory(parent)?;
    cleanup_staging(parent)?;
    if fs::read_dir(parent)
        .map_err(|error| slice_development_io_error("inspect", parent, error))?
        .next()
        .is_none()
    {
        return fs::remove_dir(parent)
            .map_err(|error| slice_development_io_error("remove", parent, error));
    }
    validate(parent)?;
    let publication = publication(parent)?;
    update_managed_publication_access("revoke", parent, &publication)?;
    fs::remove_dir_all(parent.join(PUBLICATION))
        .map_err(|error| slice_development_io_error("remove publication", parent, error))?;
    fs::remove_dir(parent).map_err(|error| slice_development_io_error("remove", parent, error))
}

fn cleanup_staging(parent: &Path) -> Result<(), DaemonError> {
    let staging = parent.join(STAGING);
    match fs::symlink_metadata(&staging) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(slice_development_io_error("inspect", &staging, error)),
        Ok(_) => {}
    }
    // This exact scratch directory is kernel-owned, outside the agent's workspace mount.
    // The lifecycle executor serializes operations for this slice.
    real_directory(&staging)?;
    fs::remove_dir_all(&staging)
        .map_err(|error| slice_development_io_error("remove staging", &staging, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-empty-publication-{}",
                rand::random::<u64>()
            ));
            fs::create_dir(&root).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_publication_recovers_after_publication_before_durable_ack() {
        let root = Scratch::new();
        let parent = root.0.join("slice");
        let published = materialize(&parent, None).unwrap();
        fs::write(
            Path::new(&published.primary_repository_path).join("edit"),
            "preserved",
        )
        .unwrap();
        assert_eq!(materialize(&parent, None).unwrap(), published);
        assert_eq!(
            fs::read_to_string(Path::new(&published.primary_repository_path).join("edit")).unwrap(),
            "preserved"
        );
        let staging = parent.join(STAGING);
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("partial"), "interrupted preparation").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o710)).unwrap();
        }
        assert_eq!(materialize(&parent, Some(&published)).unwrap(), published);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
                0o710
            );
        }
        assert!(!staging.exists());
        cleanup(&parent).unwrap();
        cleanup(&parent).unwrap();
    }

    #[test]
    fn empty_publication_never_recreates_lost_durable_content_or_deletes_unowned_content() {
        let root = Scratch::new();
        let parent = root.0.join("slice");
        let published = materialize(&parent, None).unwrap();
        fs::remove_dir_all(parent.join(PUBLICATION)).unwrap();
        assert!(materialize(&parent, Some(&published)).is_err());
        fs::create_dir(parent.join(PUBLICATION)).unwrap();
        let unowned = parent.join(PUBLICATION).join("keep");
        fs::write(&unowned, "unowned").unwrap();
        assert!(materialize(&parent, None).is_err());
        assert!(cleanup(&parent).is_err());
        assert_eq!(fs::read_to_string(&unowned).unwrap(), "unowned");
    }

    #[cfg(unix)]
    #[test]
    fn empty_publication_rejects_symlink_workspace_without_touching_target() {
        let root = Scratch::new();
        let parent = root.0.join("slice");
        let published = materialize(&parent, None).unwrap();
        let external = root.0.join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), "untouched").unwrap();
        fs::remove_dir(&published.primary_repository_path).unwrap();
        std::os::unix::fs::symlink(&external, &published.primary_repository_path).unwrap();
        assert!(materialize(&parent, Some(&published)).is_err());
        assert!(cleanup(&parent).is_err());
        assert_eq!(
            fs::read_to_string(external.join("keep")).unwrap(),
            "untouched"
        );
    }
}
