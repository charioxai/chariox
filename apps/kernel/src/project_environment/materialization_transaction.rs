//! MP-08 / MP-10 / MP-11: Import rollback respects code-layer and target ownership.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy)]
pub(super) enum MaterializationTarget {
    /// M28 owns the entire disposable checkout and its publication rollback.
    StagedDevelopment,
    /// A lease mounts existing source files; their contents must not be replaced.
    MountedSource,
}

pub(super) struct ProjectEnvironmentMaterialization {
    files: Vec<MaterializedFile>,
}
impl ProjectEnvironmentMaterialization {
    pub(super) fn prepare(
        manifest: &ProjectEnvironmentManifest,
        resolved: &ResolvedProjectEnvironment,
        roots: &BTreeMap<String, PathBuf>,
        target: MaterializationTarget,
    ) -> Result<Self, DaemonError> {
        let mut transaction = Self { files: Vec::new() };
        for ((workspace, path), value) in
            super::materialize::project_environment_files(manifest, resolved)?
        {
            let root = roots
                .get(&workspace)
                .ok_or_else(|| environment_error("materialization workspace not selected"))?;
            use super::materialize::WorkspaceFilePublication;
            let publication = match target {
                MaterializationTarget::StagedDevelopment => WorkspaceFilePublication::Replace,
                MaterializationTarget::MountedSource
                    if manifest.entries.iter().any(|entry| {
                        entry.workspace_id == workspace
                            && entry.name == path
                            && entry.kind == ProjectEnvironmentEntryKind::ConfigFile
                    }) =>
                {
                    WorkspaceFilePublication::ReuseMatching
                }
                MaterializationTarget::MountedSource => WorkspaceFilePublication::Create,
            };
            transaction
                .files
                .push(super::materialize::write_private_workspace_file_owned(
                    root,
                    &path,
                    value.as_bytes(),
                    publication,
                )?);
        }
        Ok(transaction)
    }
    pub(super) fn commit(self) {
        for file in self.files {
            file.commit();
        }
    }
}

pub(super) struct MaterializedFile {
    #[cfg(unix)]
    parent: std::fs::File,
    #[cfg(unix)]
    name: std::ffi::CString,
    #[cfg(unix)]
    identity: std::fs::Metadata,
    #[cfg(unix)]
    original_permissions: Option<(std::fs::File, std::fs::Permissions)>,
    committed: bool,
}
impl MaterializedFile {
    #[cfg(unix)]
    pub(super) fn new(
        parent: std::fs::File,
        name: std::ffi::CString,
        file: &std::fs::File,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            parent,
            name,
            identity: file
                .metadata()
                .map_err(|_| environment_error("materialized file identity unavailable"))?,
            committed: false,
            original_permissions: None,
        })
    }
    #[cfg(unix)]
    pub(super) fn preserve_permissions(&mut self, file: std::fs::File) {
        self.original_permissions = Some((file, self.identity.permissions()));
    }
    #[cfg(unix)]
    pub(super) fn sync(&self) -> Result<(), DaemonError> {
        self.parent
            .sync_all()
            .map_err(|_| environment_error("sync target config directory failed"))
    }
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for MaterializedFile {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::fs::MetadataExt,
            };
            let descriptor = unsafe {
                libc::openat(
                    self.parent.as_raw_fd(),
                    self.name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                return;
            }
            let current = unsafe { std::fs::File::from_raw_fd(descriptor) };
            let Ok(identity) = current.metadata() else {
                return;
            };
            // Do not erase a replacement file or a file edited after publication.
            if identity.dev() == self.identity.dev()
                && identity.ino() == self.identity.ino()
                && identity.len() == self.identity.len()
                && identity.mtime() == self.identity.mtime()
                && identity.mtime_nsec() == self.identity.mtime_nsec()
            {
                if let Some((file, permissions)) = &self.original_permissions {
                    use std::os::unix::fs::PermissionsExt;
                    // A subsequent target permission change belongs to the target.
                    if identity.permissions().mode() & 0o7777 == 0o600 {
                        let _ = file.set_permissions(permissions.clone());
                        let _ = file.sync_all();
                    }
                } else {
                    unsafe { libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) };
                }
                let _ = self.parent.sync_all();
            }
        }
    }
}
