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
    fn unclaimed(home: &Path, durable: DurableKernelStateStore) -> Result<Self, DaemonError> {
        if !home.is_absolute() {
            return Err(failure(
                "Skills v2 requires a resolved absolute CHARIOX_HOME",
            ));
        }
        let home = fs::canonicalize(home)
            .map_err(|_| failure("Chariox home must exist before loading skills"))?;
        Ok(Self { home, durable })
    }
    pub fn new(home: &Path, durable: DurableKernelStateStore) -> Result<Self, DaemonError> {
        let storage = Self::unclaimed(home, durable)?;
        let _lock = storage.lock()?;
        super::home_journal::claim(&storage)?;
        Ok(storage)
    }
    /// The returned lock keeps global cleanup and catalog publication mutually
    /// exclusive. Other journals only retire their own state after the owning
    /// kernel completed global cleanup; they never rewrite its new workflows.
    pub(crate) fn startup_scope(
        home: &Path,
        durable: DurableKernelStateStore,
    ) -> Result<(Self, Option<File>, std::collections::BTreeSet<String>), DaemonError> {
        let storage = Self::unclaimed(home, durable)?;
        let lock = storage.lock()?;
        if let Some(names) = super::home_journal::completed_foreign_reset(&storage)? {
            return Ok((storage, None, names));
        }
        super::home_journal::claim(&storage)?;
        Ok((storage, Some(lock), Default::default()))
    }
    pub(super) fn journal(&self) -> &DurableKernelStateStore {
        &self.durable
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn snapshot(&self) -> Result<SkillCatalogSet, DaemonError> {
        super::home_journal::verify(self)?;
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
    /// A running turn keeps the revision it admitted, including when that same
    /// turn installs an update. History lives in the existing durable journal.
    pub(crate) fn snapshot_at_revision(
        &self,
        revision: u64,
    ) -> Result<SkillCatalogSet, DaemonError> {
        let current = self.snapshot()?;
        if current.revision == revision {
            return Ok(current);
        }
        if revision == 0 {
            return Ok(SkillCatalogSet::default());
        }
        if revision > current.revision {
            return Err(failure("skill turn revision is not committed"));
        }
        let mut after = 0;
        loop {
            let page = self.durable.load_subject_events_by_kind_after(
                CATALOG_SUBJECT,
                CATALOG_KIND,
                after,
                200,
            )?;
            if page.is_empty() {
                break;
            }
            for event in page {
                after = event.sequence;
                let snapshot: SkillCatalogSet = serde_json::from_value(event.payload)
                    .map_err(|_| failure("invalid durable skill turn catalog"))?;
                if snapshot.revision == revision {
                    snapshot.validate()?;
                    return Ok(snapshot);
                }
                if snapshot.revision > revision {
                    break;
                }
            }
        }
        Err(failure(
            "skill turn catalog is unavailable; start a new turn",
        ))
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
    pub(super) fn lock(&self) -> Result<File, DaemonError> {
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
