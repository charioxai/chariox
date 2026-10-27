use std::path::{Component, Path, PathBuf};

use super::DaemonConfig;
use crate::error::DaemonError;

impl DaemonConfig {
    pub(super) fn validate_publication_control_state_root(&self) -> Result<(), DaemonError> {
        let Some(root) = &self.publication_control_state_root else {
            return Ok(());
        };
        let invalid = || DaemonError::InvalidConfig {
            field: "publication_control_state_root",
            message: "must be an absolute directory separate from private kernel configuration",
        };
        if Path::new(&self.daemon_id).file_name() != Some(std::ffi::OsStr::new(&self.daemon_id))
            || self.daemon_id.contains('\\')
        {
            return Err(DaemonError::InvalidConfig {
                field: "publication_control_state_root",
                message: "publication kernel identity must be a single path component",
            });
        }
        if !root.is_absolute()
            || root.parent().is_none()
            || root
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(invalid());
        }
        match std::fs::symlink_metadata(root) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(invalid());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(invalid());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid()),
        }
        let private = self.user_config_path.parent().ok_or_else(invalid)?;
        if !private.is_absolute() {
            return Err(invalid());
        }
        let root = resolve_existing_ancestor(root).map_err(|_| invalid())?;
        let private = resolve_existing_ancestor(private).map_err(|_| invalid())?;
        if root.starts_with(&private) || private.starts_with(&root) {
            return Err(invalid());
        }
        Ok(())
    }
}

// A new named volume need not exist yet. Resolve its existing ancestors so
// aliases such as /var -> /private/var cannot evade the separation check.
fn resolve_existing_ancestor(path: &Path) -> std::io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // An existing dangling symlink is not a missing directory that can
            // safely be appended to its parent's canonical path.
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(error);
            }
            let Some(parent) = path.parent() else {
                return Err(error);
            };
            let Some(name) = path.file_name() else {
                return Err(error);
            };
            Ok(resolve_existing_ancestor(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}
