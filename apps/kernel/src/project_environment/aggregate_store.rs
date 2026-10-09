//! MP-08: Private durable lineage anchors for the P01 read-through aggregate.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};

/// MP-08: P01 persists only identity anchors, leaving every legacy byte untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentIdentityAnchors {
    project_id: String,
    lineage: EnvironmentLineage,
    folder_ids: BTreeMap<String, String>,
}

impl ProjectEnvironmentStore {
    pub fn snapshot(
        &self,
        project: &crate::session::RuntimeProject,
    ) -> Result<ProjectEnvironment, DaemonError> {
        self.snapshot_locked(project)
    }

    pub(crate) fn snapshot_locked(
        &self,
        project: &crate::session::RuntimeProject,
    ) -> Result<ProjectEnvironment, DaemonError> {
        let identity_lock = self.identity_lock(project.id())?;
        let path = self.path(project.id()).with_extension("identity.json");
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut anchors: EnvironmentIdentityAnchors = match options.open(&path) {
            Ok(file) => {
                if !file
                    .metadata()
                    .map_err(|_| environment_error("identity unavailable"))?
                    .is_file()
                {
                    return Err(environment_error("identity must be a regular file"));
                }
                let mut bytes = vec![];
                file.take(64 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| environment_error("identity read failed"))?;
                if bytes.len() > 64 * 1024 {
                    return Err(environment_error("identity exceeds bounds"));
                }
                serde_json::from_slice(&bytes)
                    .map_err(|_| environment_error("invalid environment identity"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                EnvironmentIdentityAnchors {
                    project_id: project.id().into(),
                    lineage: EnvironmentLineage {
                        project_id: format!("{:032x}", rand::random::<u128>()),
                        environment_id: format!("{:032x}", rand::random::<u128>()),
                    },
                    folder_ids: BTreeMap::new(),
                }
            }
            Err(_) => return Err(environment_error("environment identity unavailable")),
        };
        if anchors.project_id != project.id()
            || anchors.lineage.project_id.len() != 32
            || anchors.lineage.environment_id.len() != 32
        {
            return Err(environment_error("environment identity binding mismatch"));
        }
        let previous = anchors.clone();
        for workspace in project.workspace_ids() {
            anchors
                .folder_ids
                .entry(workspace.clone())
                .or_insert_with(|| format!("{:032x}", rand::random::<u128>()));
        }
        if previous != anchors || !path.exists() {
            let temporary = self
                .root
                .join(format!(".identity-{:032x}", rand::random::<u128>()));
            let result = (|| {
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options
                    .open(&temporary)
                    .map_err(|_| environment_error("identity write failed"))?;
                file.write_all(
                    &serde_json::to_vec(&anchors)
                        .map_err(|_| environment_error("identity encoding failed"))?,
                )
                .map_err(|_| environment_error("identity write failed"))?;
                file.sync_all()
                    .map_err(|_| environment_error("identity sync failed"))?;
                fs::rename(&temporary, &path)
                    .map_err(|_| environment_error("identity commit failed"))?;
                File::open(&self.root)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| environment_error("identity directory sync failed"))
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result?;
        }
        drop(identity_lock);
        let legacy = self.load(project.id())?;
        let mut snapshot = project_environment_snapshot(
            project,
            anchors.lineage,
            &anchors.folder_ids,
            legacy.as_ref(),
        );
        self.attach_detection(&mut snapshot)?;
        self.attach_revision(&mut snapshot)?;
        Ok(snapshot)
    }
}
