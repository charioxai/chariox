//! MP-08 / MP-10 / MP-11: owner-reviewed updates of one pinned repository package.
use super::{
    catalog::{failure, SkillCatalogSet},
    repository::{staging_directory, RepositoryCheckout},
    service::SkillCatalogService,
    v2::*,
};
use crate::error::DaemonError;
use std::collections::BTreeSet;

fn member(path: &str, record: &SkillRecord) -> RepositorySkillMember {
    RepositorySkillMember {
        path: path.into(),
        skill_id: record.id.clone(),
        name: record.name.clone(),
        digest: record.digest.clone(),
    }
}

#[derive(Default)]
struct RepositoryUpdateDiff {
    changed: Vec<RepositorySkillUpdate>,
    added: Vec<RepositorySkill>,
    removed: Vec<RepositorySkillMember>,
    unchanged: Vec<RepositorySkillMember>,
    skipped: Vec<RepositorySkillSkip>,
    updates: Vec<SkillCatalogAction>,
}

impl SkillCatalogService {
    pub fn resolve_repository_update(
        &self,
        package_id: &str,
    ) -> Result<SkillsV2Response, DaemonError> {
        let catalog = self.storage.snapshot()?;
        let package = catalog
            .catalogs
            .iter()
            .flat_map(|c| &c.packages)
            .find(|p| p.id == package_id)
            .ok_or_else(|| failure("package was removed; refresh Skills"))?;
        if package.custom {
            return Err(failure(
                "this is a custom package; update its members from their recorded sources",
            ));
        }
        let locator = package
            .source_locator
            .as_deref()
            .ok_or_else(|| failure("package has no verified repository source"))?;
        let old_revision = package
            .source_revision
            .as_deref()
            .ok_or_else(|| failure("package has no pinned repository commit"))?;
        let checkout = RepositoryCheckout::fetch(staging_directory(&self.storage)?, locator, None)?;
        let (skills, skipped) = checkout.skills()?;
        if skills.is_empty() {
            return Err(failure("the repository has no installable skills; remove or ungroup the old package from Skills"));
        }
        let old = RepositoryCheckout::fetch(
            staging_directory(&self.storage)?,
            locator,
            Some(old_revision),
        )?;
        let old_paths: BTreeSet<_> = old.skills()?.0.into_iter().map(|s| s.path).collect();
        validate_members(&catalog, package, &skipped)?;
        let mut diff = RepositoryUpdateDiff {
            skipped,
            ..Default::default()
        };
        for skill in &skills {
            diff.compare_member(skill, &catalog, package, &checkout, &old_paths)?;
        }
        diff.finish(&catalog, package, &checkout, &skills)
    }
}

fn validate_members(
    catalog: &SkillCatalogSet,
    package: &SkillPackageRecord,
    skipped: &[RepositorySkillSkip],
) -> Result<(), DaemonError> {
    for record in catalog
        .catalogs
        .iter()
        .flat_map(|c| &c.skills)
        .filter(|s| s.package_id.as_deref() == Some(package.id.as_str()))
    {
        if !package.upstream_members.values().any(|id| id == &record.id) {
            return Err(failure(format!("{} has no verified upstream directory; move it to a custom package before updating this repository", record.name)));
        }
    }
    for invalid in skipped {
        if package
            .upstream_members
            .get(&invalid.path)
            .and_then(|id| catalog.record(id))
            .is_some_and(|s| s.package_id.as_deref() == Some(package.id.as_str()))
        {
            return Err(failure(format!(
                "cannot update {}: {} is invalid upstream ({}); nothing was published",
                package.name, invalid.path, invalid.reason
            )));
        }
    }
    Ok(())
}

impl RepositoryUpdateDiff {
    fn compare_member(
        &mut self,
        skill: &RepositorySkill,
        catalog: &SkillCatalogSet,
        package: &SkillPackageRecord,
        checkout: &RepositoryCheckout,
        old_paths: &BTreeSet<String>,
    ) -> Result<(), DaemonError> {
        let previous = package
            .upstream_members
            .get(&skill.path)
            .and_then(|id| catalog.record(id));
        if let Some(previous) = previous {
            if previous.package_id.as_deref() != Some(package.id.as_str()) {
                self.skipped.push(RepositorySkillSkip { path: skill.path.clone(), reason: "moved out of this package; its grouping and installed revision are retained".into() });
                return Ok(());
            }
            let digest = checkout.package(&skill.path)?.version_hash;
            let needs_update = previous.digest != digest
                || previous.provenance.kind != "repository"
                || previous.provenance.source_revision.as_deref()
                    != Some(checkout.revision.as_str())
                || previous.provenance.locator.as_deref() != Some(checkout.locator.as_str());
            let previous = member(&skill.path, previous);
            if previous.digest == digest {
                self.unchanged.push(previous.clone())
            } else {
                self.changed.push(RepositorySkillUpdate {
                    member: previous.clone(),
                    digest,
                })
            }
            if needs_update {
                self.updates.push(SkillCatalogAction::Update {
                    skill_id: previous.skill_id,
                    expected_digest: previous.digest,
                    source: SkillInstallSource::Repository {
                        url: checkout.locator.clone(),
                        revision: checkout.revision.clone(),
                        path: skill.path.clone(),
                    },
                });
            }
        } else if old_paths.contains(&skill.path) {
            self.skipped.push(RepositorySkillSkip {
                path: skill.path.clone(),
                reason: "left unselected at the installed commit".into(),
            });
        } else if let Some(owner) = catalog.catalogs.iter().flat_map(|c| &c.packages).find(|p| {
            p.id != package.id
                && p.source_locator == package.source_locator
                && p.upstream_members
                    .get(&skill.path)
                    .and_then(|id| catalog.record(id))
                    .is_some_and(|record| {
                        record.provenance.kind == "repository"
                            && record.provenance.locator == package.source_locator
                            && (record.scope == package.scope
                                || matches!(record.scope, SkillScope::Chariox)
                                || matches!(package.scope, SkillScope::Chariox))
                    })
        }) {
            self.skipped.push(RepositorySkillSkip {
                path: skill.path.clone(),
                reason: format!("already installed from this directory by package {}; its grouping and installed revision are retained", owner.name),
            });
        } else {
            self.added.push(skill.clone());
            self.updates.push(SkillCatalogAction::Install {
                source: SkillInstallSource::Repository {
                    url: checkout.locator.clone(),
                    revision: checkout.revision.clone(),
                    path: skill.path.clone(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: Some(package.id.clone()),
            });
        }
        Ok(())
    }

    fn finish(
        mut self,
        catalog: &SkillCatalogSet,
        package: &SkillPackageRecord,
        checkout: &RepositoryCheckout,
        skills: &[RepositorySkill],
    ) -> Result<SkillsV2Response, DaemonError> {
        let paths: BTreeSet<_> = skills.iter().map(|s| s.path.as_str()).collect();
        let mut actions = Vec::new();
        for (path, id) in &package.upstream_members {
            if paths.contains(path.as_str()) {
                continue;
            }
            if let Some(record) = catalog
                .record(id)
                .filter(|s| s.package_id.as_deref() == Some(package.id.as_str()))
            {
                self.removed.push(member(path, record));
                actions.push(SkillCatalogAction::Remove {
                    skill_id: id.clone(),
                });
            }
        }
        if self.updates.is_empty() && !self.removed.is_empty() {
            return Err(failure("all remaining members were removed upstream; remove or ungroup this package from Skills"));
        }
        // Removal first permits an upstream directory move to keep its old name;
        // the entire reviewed batch still succeeds or fails as one publication.
        actions.extend(self.updates);
        Ok(SkillsV2Response::RepositoryUpdate {
            package_id: package.id.clone(),
            locator: checkout.locator.clone(),
            from_revision: package
                .source_revision
                .clone()
                .expect("validated before fetch"),
            revision: checkout.revision.clone(),
            changed: self.changed,
            added: self.added,
            removed: self.removed,
            unchanged: self.unchanged,
            skipped: self.skipped,
            mutation: SkillCatalogMutation {
                scope: package.scope.clone(),
                expected_revision: catalog.revision,
                actions,
            },
        })
    }
}
