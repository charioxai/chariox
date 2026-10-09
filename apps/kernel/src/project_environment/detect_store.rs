//! MP-08 / MP-10 / MP-11: Private proposal cache; never a saved requirement revision or readiness receipt.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDetectionCache {
    pub project_id: String,
    pub evidence_digest: String,
    pub proposals: Vec<EnvironmentProposal>,
    pub operation: EnvironmentOperation,
}
impl ProjectEnvironmentStore {
    pub fn load_detection(
        &self,
        project: &str,
    ) -> Result<Option<EnvironmentDetectionCache>, DaemonError> {
        // Private proposal caches are disposable across corruption and schema rollback.
        Ok(self.read_detection(project).ok().flatten())
    }
    fn read_detection(
        &self,
        project: &str,
    ) -> Result<Option<EnvironmentDetectionCache>, DaemonError> {
        let path = self.path(project).with_extension("detect.json");
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(environment_error("detection cache unavailable")),
        };
        if !file
            .metadata()
            .map_err(|_| environment_error("detection cache unavailable"))?
            .is_file()
        {
            return Err(environment_error("detection cache must be regular"));
        }
        let mut bytes = vec![];
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("detection cache read failed"))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(environment_error("detection cache exceeds bounds"));
        }
        let cache: EnvironmentDetectionCache = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("invalid detection cache"))?;
        if cache.project_id != project || cache.operation.local_project_id != project {
            return Err(environment_error("detection cache binding mismatch"));
        }
        Ok(Some(cache))
    }
    pub fn save_detection(&self, cache: &EnvironmentDetectionCache) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec(cache)
            .map_err(|_| environment_error("detection cache encoding failed"))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(environment_error("detection proposals exceed bounds"));
        }
        crate::config::write_private_file(
            &self.path(&cache.project_id).with_extension("detect.json"),
            &bytes,
        )
        .map_err(|_| environment_error("detection cache write failed"))
    }
    pub(crate) fn attach_detection(
        &self,
        snapshot: &mut ProjectEnvironment,
    ) -> Result<(), DaemonError> {
        if let Some(cache) = self.load_detection(&snapshot.local_project_id)? {
            let valid_folder = |r: &Requirement| match &r.scope {
                RequirementScope::Folder { folder_id } => {
                    snapshot.folders.iter().any(|f| &f.folder_id == folder_id)
                }
                RequirementScope::Project => false,
            };
            snapshot.proposals.extend(
                cache
                    .proposals
                    .into_iter()
                    .filter(|p| valid_folder(&p.requirement)),
            );
            snapshot.evidence_digest = Some(cache.evidence_digest);
            snapshot.operations = vec![cache.operation];
        }
        snapshot
            .delivered_capabilities
            .enabled_environment_operations =
            vec![EnvironmentCapability::Get, EnvironmentCapability::Detect];
        Ok(())
    }
}
