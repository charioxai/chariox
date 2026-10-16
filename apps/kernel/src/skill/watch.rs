//! MP-08 / MP-10 / MP-11: disposable watch cursors over the existing home journal.
//! A cursor is a scoped read model, never catalog or delivery authority.
use super::{
    catalog::{empty_catalog, failure},
    storage::SkillCatalogStorage,
    v2::*,
};
use crate::error::DaemonError;
use std::collections::BTreeSet;

#[derive(Clone, Default)]
pub(crate) struct SkillWatchCursor {
    project_id: Option<String>,
    catalogs: Option<Vec<SkillCatalog>>,
    loaded_after: u64,
}
impl SkillCatalogStorage {
    pub(crate) fn watch_changes(
        &self,
        session_id: &str,
        project_id: &str,
        mut cursor: SkillWatchCursor,
    ) -> Result<(SkillWatchCursor, Vec<SkillCatalogEvent>), DaemonError> {
        if cursor.project_id.as_deref() != Some(project_id) {
            cursor = SkillWatchCursor::default();
            cursor.project_id = Some(project_id.into());
        }
        // Establish the load cursor before reading catalogs. A concurrent load
        // is then picked up below; reconnect reads historical loads with Loads.
        if cursor.catalogs.is_none() {
            cursor.loaded_after = self.journal().latest_event_sequence()?;
        }
        let current = self.snapshot()?;
        let scopes = [
            SkillScope::Chariox,
            SkillScope::Project {
                project_id: project_id.into(),
            },
        ];
        let catalogs: Vec<_> = scopes
            .iter()
            .map(|scope| {
                current
                    .catalog(scope)
                    .cloned()
                    .unwrap_or_else(|| empty_catalog(scope.clone(), current.revision))
            })
            .collect();
        let mut events = Vec::new();
        for catalog in &catalogs {
            let previous = cursor
                .catalogs
                .as_ref()
                .and_then(|catalogs| catalogs.iter().find(|old| old.scope == catalog.scope));
            if let Some(event) = catalog_change(previous, catalog, current.revision) {
                events.push(event);
            }
        }
        cursor.catalogs = Some(catalogs);
        // Bound one feed iteration; subsequent iterations continue the same
        // durable sequence, so large batches cannot drop confirmed receipts.
        for event in self.journal().load_subject_events_by_kind_after(
            session_id,
            "skills.v2.loaded",
            cursor.loaded_after,
            200,
        )? {
            let record: SkillLoadRecord = serde_json::from_value(event.payload)
                .map_err(|_| failure("invalid durable skill delivery event"))?;
            if record.session_id != session_id || record.delivered_at_ms == 0 {
                return Err(failure("skill delivery event does not match this session"));
            }
            cursor.loaded_after = event.sequence;
            events.push(SkillCatalogEvent::Loaded { record });
        }
        Ok((cursor, events))
    }
}

fn catalog_change(
    previous: Option<&SkillCatalog>,
    current: &SkillCatalog,
    revision: u64,
) -> Option<SkillCatalogEvent> {
    let mut comparable = current.clone();
    comparable.revision = 0;
    if previous.is_some_and(|old| {
        let mut old = old.clone();
        old.revision = 0;
        old == comparable
    }) {
        // The home-wide expected revision also changes for another Project.
        // Invalidate that number without exposing its members or scope.
        return (current.scope == SkillScope::Chariox
            && previous.is_some_and(|old| old.revision != revision))
        .then(|| SkillCatalogEvent::Changed {
            revision,
            scope: SkillScope::Chariox,
            changed_skill_ids: vec![],
            removed_skill_ids: vec![],
        });
    }
    let mut changed: BTreeSet<_> = current
        .skills
        .iter()
        .filter(|record| {
            previous.and_then(|old| old.skills.iter().find(|old| old.id == record.id))
                != Some(*record)
        })
        .map(|record| record.id.clone())
        .collect();
    let removed = previous
        .into_iter()
        .flat_map(|old| &old.skills)
        .filter(|old| !current.skills.iter().any(|record| record.id == old.id))
        .map(|old| old.id.clone())
        .collect();
    // Project overrides change the effective Chariox rows without modifying
    // those records. Include both added/changed and removed override IDs.
    for id in current.chariox_overrides.keys().chain(
        previous
            .into_iter()
            .flat_map(|old| old.chariox_overrides.keys()),
    ) {
        if current.chariox_overrides.get(id)
            != previous.and_then(|old| old.chariox_overrides.get(id))
        {
            changed.insert(id.clone());
        }
    }
    Some(SkillCatalogEvent::Changed {
        revision,
        scope: current.scope.clone(),
        changed_skill_ids: changed.into_iter().collect(),
        removed_skill_ids: removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_watch_global_revision_does_not_leak_other_project_changes() {
        let old = empty_catalog(SkillScope::Chariox, 1);
        let mut current = old.clone();
        current.revision = 2;
        assert!(
            matches!(catalog_change(Some(&old), &current, 2), Some(SkillCatalogEvent::Changed { scope:SkillScope::Chariox, changed_skill_ids, removed_skill_ids, .. }) if changed_skill_ids.is_empty() && removed_skill_ids.is_empty())
        );
    }
    #[test]
    fn skill_watch_reports_reset_project_overrides_and_package_only_changes() {
        let mut old = empty_catalog(
            SkillScope::Project {
                project_id: "project".into(),
            },
            1,
        );
        old.chariox_overrides
            .insert("global-skill".into(), SkillState::Off);
        let mut current = old.clone();
        current.revision = 2;
        current.chariox_overrides.clear();
        assert!(
            matches!(catalog_change(Some(&old), &current, 2), Some(SkillCatalogEvent::Changed { changed_skill_ids, .. }) if changed_skill_ids == ["global-skill"])
        );
        current.packages.push(SkillPackageRecord {
            id: "package".into(),
            name: "Custom".into(),
            scope: current.scope.clone(),
            custom: true,
            source_locator: None,
            source_revision: None,
            upstream_members: Default::default(),
        });
        assert!(
            catalog_change(Some(&empty_catalog(current.scope.clone(), 1)), &current, 2).is_some()
        );
    }
}
