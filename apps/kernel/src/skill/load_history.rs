//! MP-08 / MP-10 / MP-11: immutable delivery history and revision-bound active loads.
use super::{
    catalog::{failure, SkillCatalogSet},
    storage::SkillCatalogStorage,
    v2::*,
};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredSkillLoad {
    pub record: SkillLoadRecord,
    pub catalog_revision: u64,
    pub project_id: Option<String>,
}
impl SkillCatalogStorage {
    #[cfg(test)]
    pub fn record_load(&self, record: &SkillLoadRecord) -> Result<(), DaemonError> {
        self.record_load_at_revision(record, None, self.snapshot()?.revision)
    }
    pub(crate) fn record_load_at_revision(
        &self,
        record: &SkillLoadRecord,
        project_id: Option<&str>,
        catalog_revision: u64,
    ) -> Result<(), DaemonError> {
        if record.delivered_at_ms == 0 {
            return Err(failure(
                "skill availability is not delivery acknowledgement",
            ));
        }
        let _lock = self.lock()?;
        // Verify that this admitted revision still exists. A stale acknowledgement
        // remains truthful history, but cannot revive authority revoked since admission.
        self.snapshot_at_revision(catalog_revision)?;
        if self.loads(&record.session_id)?.iter().any(|previous| {
            previous.skill_id == record.skill_id
                && previous.agent_id == record.agent_id
                && previous.run_id == record.run_id
                && previous.context_generation == record.context_generation
                && previous.digest == record.digest
        }) {
            return Ok(());
        }
        self.journal().append_event(
            "skills.v2.loaded",
            Some(record.session_id.clone()),
            serde_json::to_value(StoredSkillLoad {
                record: record.clone(),
                catalog_revision,
                project_id: project_id.map(str::to_owned),
            })
            .map_err(|_| failure("invalid load receipt"))?,
        )?;
        Ok(())
    }
    fn load_bindings(&self, session_id: &str) -> Result<Vec<StoredSkillLoad>, DaemonError> {
        super::home_journal::verify(self)?;
        let ceiling = self.journal().latest_event_sequence()?;
        let mut after = 0;
        let mut records = Vec::new();
        loop {
            let page = self.journal().load_subject_events_by_kind_after(
                session_id,
                "skills.v2.loaded",
                after,
                200,
            )?;
            if page.is_empty() {
                break;
            }
            for event in page {
                if event.sequence > ceiling {
                    return Ok(records);
                }
                after = event.sequence;
                let bound: StoredSkillLoad = serde_json::from_value(event.payload)
                    .map_err(|_| failure("invalid durable skill load binding"))?;
                if bound.record.session_id != session_id || bound.record.delivered_at_ms == 0 {
                    return Err(failure("skill load binding does not match this session"));
                }
                records.push(bound);
            }
        }
        Ok(records)
    }
    pub fn loads(&self, session_id: &str) -> Result<Vec<SkillLoadRecord>, DaemonError> {
        Ok(self
            .load_bindings(session_id)?
            .into_iter()
            .map(|bound| bound.record)
            .collect())
    }
    pub(crate) fn active_loads(
        &self,
        session_id: &str,
        project_id: Option<&str>,
        through_revision: u64,
    ) -> Result<Vec<SkillLoadRecord>, DaemonError> {
        self.snapshot_at_revision(through_revision)?;
        let mut candidates = self
            .load_bindings(session_id)?
            .into_iter()
            .filter(|bound| {
                bound.project_id.as_deref() == project_id
                    && bound.catalog_revision <= through_revision
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(vec![]);
        }
        let first = candidates
            .iter()
            .map(|bound| bound.catalog_revision)
            .min()
            .unwrap();
        let mut after = 0;
        let mut last = 0;
        loop {
            let page = self.journal().load_subject_events_by_kind_after(
                "skills-v2",
                "skills.v2.catalog",
                after,
                200,
            )?;
            if page.is_empty() {
                break;
            }
            for event in page {
                after = event.sequence;
                let catalog: SkillCatalogSet = serde_json::from_value(event.payload)
                    .map_err(|_| failure("invalid durable skill revocation history"))?;
                if catalog.revision > through_revision {
                    if last != through_revision {
                        return Err(failure("skill revocation history is incomplete"));
                    }
                    return Ok(candidates.into_iter().map(|b| b.record).collect());
                }
                if catalog.revision <= first {
                    last = catalog.revision;
                    continue;
                }
                if catalog.revision != last + 1 {
                    return Err(failure("skill revocation history is incomplete"));
                }
                last = catalog.revision;
                catalog.validate()?;
                let rows = super::resolver::projections(&catalog, project_id, &[]);
                candidates.retain(|bound| {
                    catalog.revision <= bound.catalog_revision
                        || rows.iter().any(|row| {
                            row.skill.id == bound.record.skill_id
                                && row.effective_state != SkillState::Off
                        })
                });
            }
        }
        if last < through_revision {
            return Err(failure("skill revocation history is incomplete"));
        }
        Ok(candidates.into_iter().map(|b| b.record).collect())
    }
}
