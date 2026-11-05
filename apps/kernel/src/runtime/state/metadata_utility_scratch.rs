//! A private, empty working directory for a metadata-only utility run, which must not see any source.
use crate::error::DaemonError;
use crate::project_environment::environment_error;
use std::path::{Path, PathBuf};

pub(super) struct MetadataUtilityScratch(pub(super) PathBuf);

impl MetadataUtilityScratch {
    pub(super) fn new(root: &Path) -> Result<Self, DaemonError> {
        std::fs::create_dir_all(root)
            .map_err(|_| environment_error("the utility state root is unavailable"))?;
        let path = root.join(format!("metadata-utility-{}", rand::random::<u64>()));
        std::fs::create_dir(&path)
            .map_err(|_| environment_error("the utility scratch directory is unavailable"))?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(
                |_| environment_error("the utility scratch directory could not be secured"),
            )?;
        }
        Ok(Self(path))
    }
}

impl Drop for MetadataUtilityScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
