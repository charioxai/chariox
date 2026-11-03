//! The kernel's durable Environment records, one file per Project.
use super::{environment_error, ProjectEnvironmentRecord};
use crate::error::DaemonError;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DIRECTORY: &str = "project-environment";
/// Schema 1 lived here; it has no read path, so the first start of this kernel deletes it.
const SCHEMA_1_DIRECTORY: &str = "project-environments";
const MAX_RECORD_BYTES: u64 = 1024 * 1024;

/// One kernel process owns its home, so one lock serializes every read-modify-write.
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct ProjectEnvironmentStore {
    root: PathBuf,
}

pub(crate) enum SaveOutcome {
    Saved(ProjectEnvironmentRecord),
    /// The record changed since the caller read it; nothing was written.
    Conflict(ProjectEnvironmentRecord),
}

pub(crate) fn remove_schema_1_state(private_state_root: &Path) -> Result<(), DaemonError> {
    match fs::remove_dir_all(private_state_root.join(SCHEMA_1_DIRECTORY)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(environment_error(
            "removing the schema 1 Project environments failed",
        )),
    }
}

impl ProjectEnvironmentStore {
    pub fn new(private_state_root: &Path) -> Self {
        Self {
            root: private_state_root.join(DIRECTORY),
        }
    }

    fn path(&self, project: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.json", Sha256::digest(project.as_bytes())))
    }

    pub fn load(&self, project: &str) -> Result<Option<ProjectEnvironmentRecord>, DaemonError> {
        let file = match File::open(self.path(project)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(environment_error("the saved Environment is unreadable")),
        };
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("the saved Environment is unreadable"))?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(environment_error("the saved Environment exceeds its bound"));
        }
        let record: ProjectEnvironmentRecord = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("the saved Environment is not valid"))?;
        record.validate().map_err(environment_error)?;
        Ok(Some(record))
    }

    /// Writes `fields` as revision `expected_revision + 1` unless the record moved on.
    pub(crate) fn save(
        &self,
        project: &str,
        expected_revision: u64,
        fields: ProjectEnvironmentRecord,
    ) -> Result<SaveOutcome, DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current_revision = match self.load(project)? {
            Some(current) if current.revision != expected_revision => {
                return Ok(SaveOutcome::Conflict(current))
            }
            Some(current) => current.revision,
            None if expected_revision != 0 => {
                return Ok(SaveOutcome::Conflict(ProjectEnvironmentRecord {
                    schema: super::record::PROJECT_ENVIRONMENT_SCHEMA,
                    ..Default::default()
                }))
            }
            None => 0,
        };
        let record = ProjectEnvironmentRecord {
            revision: current_revision + 1,
            changed_at_ms: crate::session::unix_epoch_ms(),
            ..fields
        };
        record.validate().map_err(environment_error)?;
        self.write(project, &record)?;
        Ok(SaveOutcome::Saved(record))
    }

    fn write(&self, project: &str, record: &ProjectEnvironmentRecord) -> Result<(), DaemonError> {
        fs::create_dir_all(&self.root)
            .map_err(|_| environment_error("the Environment directory is unavailable"))?;
        let bytes = serde_json::to_vec(record)
            .map_err(|_| environment_error("the Environment could not be encoded"))?;
        let temporary = self.root.join(format!(".tmp-{}", rand::random::<u64>()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, self.path(project))?;
            File::open(&self.root)?.sync_all()
        })();
        let _ = fs::remove_file(&temporary);
        result.map_err(|_| environment_error("saving the Environment failed"))
    }

    fn remove(&self, project: &str) -> Result<(), DaemonError> {
        match fs::remove_file(self.path(project)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(environment_error("removing the Environment failed")),
        }
    }

    /// Deletes the record and this Project's Secret values; the Vault must be unlocked for the values.
    pub fn remove_with_secrets(
        &self,
        project: &str,
        vault: &dyn crate::secret::CredentialVaultStore,
    ) -> Result<(), DaemonError> {
        let _guard = WRITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(record) = self.load(project)? {
            let service = super::project_environment_vault_service(project);
            for name in &record.secrets {
                vault.delete_secret(&service, name)?;
            }
        }
        self.remove(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_environment::record::PROJECT_ENVIRONMENT_SCHEMA;

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> Scratch {
        Scratch(std::env::temp_dir().join(format!(
            "chariox-environment-{:032x}",
            rand::random::<u128>()
        )))
    }
    fn fields(script: &str) -> ProjectEnvironmentRecord {
        ProjectEnvironmentRecord {
            schema: PROJECT_ENVIRONMENT_SCHEMA,
            setup_script: script.into(),
            ..Default::default()
        }
    }

    #[test]
    fn save_bumps_the_revision_and_a_stale_save_gets_the_current_record() {
        let dir = scratch();
        let store = ProjectEnvironmentStore::new(&dir.0);
        assert!(store.load("p").unwrap().is_none());
        let SaveOutcome::Saved(first) = store.save("p", 0, fields("one")).unwrap() else {
            panic!("first save must succeed")
        };
        assert_eq!(first.revision, 1);
        let SaveOutcome::Conflict(current) = store.save("p", 0, fields("two")).unwrap() else {
            panic!("a stale revision must conflict")
        };
        assert_eq!(current.setup_script, "one");
        let SaveOutcome::Saved(second) = store.save("p", 1, fields("two")).unwrap() else {
            panic!("the current revision must save")
        };
        assert_eq!(second.revision, 2);
        assert_eq!(store.load("p").unwrap().unwrap().setup_script, "two");
    }

    #[test]
    fn schema_1_state_is_deleted_without_being_read() {
        let dir = scratch();
        let old = dir.0.join(SCHEMA_1_DIRECTORY);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("x.json"), b"{}").unwrap();
        remove_schema_1_state(&dir.0).unwrap();
        assert!(!old.exists());
        remove_schema_1_state(&dir.0).unwrap();
    }
}
