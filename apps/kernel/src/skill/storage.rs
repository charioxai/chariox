//! MP-08 / MP-10 / MP-11: catalog publication on the existing durable journal.
//! JSON catalogs are recoverable projections of an acknowledged journal record.
use super::{
    catalog::{failure, validate_id, SkillCatalogSet},
    v2::*,
};
use crate::{durable_state::DurableKernelStateStore, error::DaemonError};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
const CATALOG_KIND: &str = "skills.v2.catalog";
const CATALOG_SUBJECT: &str = "skills-v2";

#[derive(Debug, Clone)]
pub struct SkillCatalogStorage {
    home: PathBuf,
    durable: DurableKernelStateStore,
}

impl SkillCatalogStorage {
    pub fn new(home: &Path, durable: DurableKernelStateStore) -> Result<Self, DaemonError> {
        if !home.is_absolute() {
            return Err(failure(
                "Skills v2 requires a resolved absolute CHARIOX_HOME",
            ));
        }
        let home = fs::canonicalize(home)
            .map_err(|_| failure("Chariox home must exist before loading skills"))?;
        Ok(Self { home, durable })
    }
    pub(super) fn journal(&self) -> &DurableKernelStateStore {
        &self.durable
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn snapshot(&self) -> Result<SkillCatalogSet, DaemonError> {
        let latest = self
            .durable
            .load_subject_events_by_kind(CATALOG_SUBJECT, CATALOG_KIND, 1)?;
        let catalogs = latest
            .last()
            .map(|event| {
                serde_json::from_value::<SkillCatalogSet>(event.payload.clone()).map_err(|_| {
                    failure("invalid durable Skills v2 catalog; catalog loading stopped")
                })
            })
            .transpose()?
            .unwrap_or_default();
        catalogs.validate()?;
        Ok(catalogs)
    }
    /// Rebuild projections after a crash between journal commit and file publication.
    /// Runtime readers always read the durable revision, never a stale projection.
    pub fn recover_projections(&self) -> Result<SkillCatalogSet, DaemonError> {
        let _lock = self.lock()?;
        let snapshot = self.snapshot()?;
        self.project(&snapshot)?;
        Ok(snapshot)
    }
    pub fn publish(
        &self,
        mutation: &SkillCatalogMutation,
        prepared: &BTreeMap<usize, SkillRecord>,
    ) -> Result<SkillCatalogSet, DaemonError> {
        let _lock = self.lock()?;
        let current = self.snapshot()?;
        let next = current.plan(mutation, prepared)?;
        let value =
            serde_json::to_value(&next).map_err(|_| failure("catalog could not be serialized"))?;
        self.durable
            .append_event(CATALOG_KIND, Some(CATALOG_SUBJECT.into()), value)?;
        // The journal has committed. A projection failure must not ask the caller
        // to replay an already published batch. Recovery reads the durable event.
        if self.project(&next).is_err() {
            crate::logging::warn_with_fields(
                "skills.v2.catalog",
                "catalog committed; JSON projections require recovery",
                serde_json::json!({"revision": next.revision}),
            );
        }
        Ok(next)
    }
    pub fn record_load(&self, record: &SkillLoadRecord) -> Result<(), DaemonError> {
        if record.delivered_at_ms == 0 {
            return Err(failure(
                "skill availability is not delivery acknowledgement",
            ));
        }
        let _lock = self.lock()?;
        if self.loads(&record.session_id)?.iter().any(|previous| {
            previous.skill_id == record.skill_id
                && previous.agent_id == record.agent_id
                && previous.run_id == record.run_id
                && previous.context_generation == record.context_generation
                && previous.digest == record.digest
        }) {
            return Ok(());
        }
        self.durable.append_event(
            "skills.v2.loaded",
            Some(record.session_id.clone()),
            serde_json::to_value(record).map_err(|_| failure("invalid load receipt"))?,
        )?;
        Ok(())
    }
    /// Session history is not a capped activity tail: all confirmed receipts
    /// remain available for active Manual loads, compaction and acknowledgement
    /// idempotency even after other agents generate more than one journal page.
    pub fn loads(&self, session_id: &str) -> Result<Vec<SkillLoadRecord>, DaemonError> {
        let through_sequence = self.durable.latest_event_sequence()?;
        let mut after_sequence = 0;
        let mut records = Vec::new();
        loop {
            let page = self.durable.load_subject_events_by_kind_after(
                session_id,
                "skills.v2.loaded",
                after_sequence,
                200,
            )?;
            if page.is_empty() {
                break;
            }
            for event in page {
                if event.sequence > through_sequence {
                    return Ok(records);
                }
                after_sequence = event.sequence;
                records.push(
                    serde_json::from_value(event.payload)
                        .map_err(|_| failure("invalid durable skill load receipt"))?,
                );
            }
        }
        Ok(records)
    }
    fn project(&self, snapshot: &SkillCatalogSet) -> Result<(), DaemonError> {
        for catalog in &snapshot.catalogs {
            let relative = match &catalog.scope {
                SkillScope::Chariox => PathBuf::from("state/skills"),
                SkillScope::Project { project_id } => {
                    validate_id(project_id)?;
                    PathBuf::from("state/projects").join(project_id)
                }
            };
            let directory = self.directory(&relative)?;
            let destination = directory.join(if catalog.scope == SkillScope::Chariox {
                "catalog.json"
            } else {
                "skills.json"
            });
            let encoded =
                serde_json::to_vec(catalog).map_err(|_| failure("invalid catalog projection"))?;
            crate::config::write_private_file(&destination, &encoded)
                .map_err(|_| failure("catalog projection publication failed"))?;
        }
        Ok(())
    }
    fn lock(&self) -> Result<File, DaemonError> {
        let directory = self.directory(Path::new("state/skills"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(directory.join("catalog.lock"))
            .map_err(|_| failure("cannot open catalog publication lock"))?;
        if !file
            .metadata()
            .map_err(|_| failure("cannot inspect catalog lock"))?
            .is_file()
        {
            return Err(failure("catalog lock must be a regular file"));
        }
        fs2::FileExt::lock_exclusive(&file)
            .map_err(|_| failure("cannot lock catalog publication"))?;
        Ok(file)
    }
    pub(super) fn directory(&self, relative: &Path) -> Result<PathBuf, DaemonError> {
        super::paths::owned_directory(&self.home, relative)
    }
}
