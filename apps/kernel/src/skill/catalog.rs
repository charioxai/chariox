//! MP-08 / MP-10 / MP-11: deterministic catalog transactions shared by every client.
use super::v2::*;
use crate::error::DaemonError;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogSet {
    pub revision: u64,
    pub catalogs: Vec<SkillCatalog>,
}

impl Default for SkillCatalogSet {
    fn default() -> Self {
        Self {
            revision: 0,
            catalogs: vec![empty_catalog(SkillScope::Chariox, 0)],
        }
    }
}

pub fn empty_catalog(scope: SkillScope, revision: u64) -> SkillCatalog {
    SkillCatalog {
        schema: SKILL_CATALOG_SCHEMA,
        revision,
        scope,
        skills: vec![],
        packages: vec![],
        chariox_overrides: BTreeMap::new(),
    }
}

pub(super) fn failure(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.catalog",
        message: message.into(),
    }
}

pub fn collision_key(name: &str) -> Result<String, DaemonError> {
    crate::mcp::validate_registry_name(name, "skill name")?;
    if name.len() > 128 {
        return Err(failure("skill names must be at most 128 bytes"));
    }
    Ok(name.to_ascii_lowercase())
}

pub fn validate_id(id: &str) -> Result<(), DaemonError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(failure("invalid skill, package or Project ID"));
    }
    Ok(())
}

pub fn validate_scope(scope: &SkillScope) -> Result<(), DaemonError> {
    if let SkillScope::Project { project_id } = scope {
        validate_id(project_id)?;
    }
    Ok(())
}

impl SkillCatalogSet {
    pub fn catalog(&self, scope: &SkillScope) -> Option<&SkillCatalog> {
        self.catalogs.iter().find(|c| &c.scope == scope)
    }
    pub fn record(&self, id: &str) -> Option<&SkillRecord> {
        self.catalogs
            .iter()
            .flat_map(|c| &c.skills)
            .find(|s| s.id == id)
    }
    fn catalog_mut(&mut self, scope: &SkillScope) -> &mut SkillCatalog {
        if !self.catalogs.iter().any(|c| &c.scope == scope) {
            self.catalogs
                .push(empty_catalog(scope.clone(), self.revision));
        }
        self.catalogs
            .iter_mut()
            .find(|c| &c.scope == scope)
            .expect("catalog was inserted")
    }
    fn record_mut(&mut self, id: &str) -> Result<&mut SkillRecord, DaemonError> {
        self.catalogs
            .iter_mut()
            .flat_map(|c| &mut c.skills)
            .find(|s| s.id == id)
            .ok_or_else(|| failure("skill was removed; refresh Skills"))
    }
    pub fn validate(&self) -> Result<(), DaemonError> {
        if self.catalog(&SkillScope::Chariox).is_none() {
            return Err(failure("Chariox catalog is missing"));
        }
        let mut scopes = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut package_ids = BTreeSet::new();
        let mut names = Vec::new();
        for catalog in &self.catalogs {
            validate_scope(&catalog.scope)?;
            if catalog.schema != SKILL_CATALOG_SCHEMA
                || catalog.revision != self.revision
                || !scopes.insert(&catalog.scope)
            {
                return Err(failure(
                    "invalid catalog schema, revision or duplicate scope",
                ));
            }
            for package in &catalog.packages {
                validate_id(&package.id)?;
                if package.name.trim().is_empty()
                    || package.name.len() > 256
                    || package.scope != catalog.scope
                    || !package_ids.insert(&package.id)
                {
                    return Err(failure("invalid package identity, name or scope"));
                }
            }
            for skill in &catalog.skills {
                validate_id(&skill.id)?;
                let key = collision_key(&skill.name)?;
                if !ids.insert(&skill.id)
                    || skill.scope != catalog.scope
                    || skill.default_state == SkillState::Off
                {
                    return Err(failure(
                        "invalid skill identity, scope or invocation default",
                    ));
                }
                if let Some(package_id) = &skill.package_id {
                    if !catalog.packages.iter().any(|p| &p.id == package_id) {
                        return Err(failure("package membership must stay in the skill's scope"));
                    }
                }
                names.push((&catalog.scope, key));
            }
        }
        for (i, (scope, name)) in names.iter().enumerate() {
            if names[..i].iter().any(|(other_scope, other_name)| {
                other_name == name
                    && (scope == other_scope
                        || matches!(scope, SkillScope::Chariox)
                        || matches!(other_scope, SkillScope::Chariox))
            }) {
                return Err(failure(format!(
                    "skill name `{name}` is occupied; choose Rename or Replace"
                )));
            }
        }
        let global_ids: BTreeSet<&str> = self
            .catalog(&SkillScope::Chariox)
            .into_iter()
            .flat_map(|c| &c.skills)
            .map(|s| s.id.as_str())
            .collect();
        for catalog in &self.catalogs {
            if catalog
                .chariox_overrides
                .keys()
                .any(|id| !global_ids.contains(id.as_str()))
                || (catalog.scope == SkillScope::Chariox && !catalog.chariox_overrides.is_empty())
            {
                return Err(failure(
                    "Project overrides must refer to an installed Chariox skill",
                ));
            }
        }
        Ok(())
    }

    /// Prepared records contain verified immutable bytes; all changes remain private
    /// until the sole durable writer accepts expected_revision.
    pub fn plan(
        &self,
        mutation: &SkillCatalogMutation,
        prepared: &BTreeMap<usize, SkillRecord>,
    ) -> Result<Self, DaemonError> {
        if mutation.expected_revision != self.revision {
            return Err(failure(
                "catalog changed; refresh and review the batch again",
            ));
        }
        validate_scope(&mutation.scope)?;
        if mutation.actions.is_empty() || mutation.actions.len() > 512 {
            return Err(failure("a batch must contain 1 to 512 reviewed actions"));
        }
        let mut next = self.clone();
        for (index, action) in mutation.actions.iter().enumerate() {
            match action {
                SkillCatalogAction::Install {
                    collision,
                    package_id,
                    ..
                } => {
                    let mut record = prepared
                        .get(&index)
                        .cloned()
                        .ok_or_else(|| failure("install source was not verified"))?;
                    record.scope = mutation.scope.clone();
                    record.package_id = package_id.clone();
                    match collision {
                        SkillCollisionAction::Reject => {}
                        SkillCollisionAction::Rename { name } => {
                            collision_key(name)?;
                            record.name = name.clone();
                        }
                        SkillCollisionAction::Replace { skill_id } => {
                            let existing = next
                                .record(skill_id)
                                .cloned()
                                .ok_or_else(|| failure("replacement target was removed"))?;
                            // Replace updates the existing binding, never duplicates across scopes.
                            record.id = existing.id;
                            record.scope = existing.scope;
                            record.package_id = existing.package_id;
                            record.state_override = existing.state_override;
                            record.provenance.installed_at_ms = existing.provenance.installed_at_ms;
                            *next.record_mut(skill_id)? = record;
                            continue;
                        }
                    }
                    next.catalog_mut(&mutation.scope).skills.push(record);
                }
                SkillCatalogAction::Update {
                    skill_id,
                    expected_digest,
                    ..
                } => {
                    let existing = next
                        .record(skill_id)
                        .cloned()
                        .ok_or_else(|| failure("updated skill was removed"))?;
                    if existing.scope != mutation.scope || &existing.digest != expected_digest {
                        return Err(failure(
                            "skill scope or revision changed; review update again",
                        ));
                    }
                    let mut record = prepared
                        .get(&index)
                        .cloned()
                        .ok_or_else(|| failure("update source was not verified"))?;
                    record.id = existing.id;
                    record.scope = existing.scope;
                    record.package_id = existing.package_id;
                    record.name = existing.name;
                    record.state_override = existing.state_override;
                    record.provenance.installed_at_ms = existing.provenance.installed_at_ms;
                    *next.record_mut(skill_id)? = record;
                }
                SkillCatalogAction::Remove { skill_id } => {
                    next.remove_skill(&mutation.scope, skill_id)?;
                }
                SkillCatalogAction::SetState { skill_id, state } => {
                    next.set_state(&mutation.scope, skill_id, *state)?;
                }
                SkillCatalogAction::CreatePackage { package_id, name } => {
                    validate_id(package_id)?;
                    next.catalog_mut(&mutation.scope)
                        .packages
                        .push(SkillPackageRecord {
                            id: package_id.clone(),
                            name: name.clone(),
                            scope: mutation.scope.clone(),
                            custom: true,
                            source_locator: None,
                            source_revision: None,
                            upstream_members: BTreeMap::new(),
                        });
                }
                SkillCatalogAction::RenamePackage { package_id, name } => {
                    next.package_mut(&mutation.scope, package_id)?.name = name.clone();
                }
                SkillCatalogAction::SetPackageSource {
                    package_id,
                    source_locator,
                    source_revision,
                    upstream_members,
                } => {
                    let package = next.package_mut(&mutation.scope, package_id)?;
                    package.custom = false;
                    package.source_locator = Some(safe_locator(source_locator)?);
                    package.source_revision = Some(source_revision.clone());
                    package.upstream_members = upstream_members.clone();
                }
                SkillCatalogAction::Regroup {
                    skill_id,
                    package_id,
                } => {
                    let skill = next.record_mut(skill_id)?;
                    if skill.scope != mutation.scope {
                        return Err(failure("regroup must use the skill's scope"));
                    }
                    skill.package_id = package_id.clone();
                }
                SkillCatalogAction::SetPackageState { package_id, state } => {
                    let package = next
                        .catalogs
                        .iter()
                        .flat_map(|c| &c.packages)
                        .find(|p| &p.id == package_id)
                        .ok_or_else(|| failure("package was removed"))?;
                    if package.scope != mutation.scope
                        && !(package.scope == SkillScope::Chariox
                            && matches!(mutation.scope, SkillScope::Project { .. }))
                    {
                        return Err(failure("package is outside the selected scope"));
                    }
                    let ids: Vec<String> = next
                        .catalogs
                        .iter()
                        .flat_map(|c| &c.skills)
                        .filter(|s| s.package_id.as_ref() == Some(package_id))
                        .map(|s| s.id.clone())
                        .collect();
                    for id in ids {
                        next.set_state(&mutation.scope, &id, *state)?;
                    }
                }
                SkillCatalogAction::RemovePackage {
                    package_id,
                    remove_skills,
                } => {
                    next.package_mut(&mutation.scope, package_id)?;
                    let ids: Vec<String> = next
                        .catalog(&mutation.scope)
                        .into_iter()
                        .flat_map(|c| &c.skills)
                        .filter(|s| s.package_id.as_ref() == Some(package_id))
                        .map(|s| s.id.clone())
                        .collect();
                    for id in ids {
                        if *remove_skills {
                            next.remove_skill(&mutation.scope, &id)?;
                        } else {
                            next.record_mut(&id)?.package_id = None;
                        }
                    }
                    next.catalog_mut(&mutation.scope)
                        .packages
                        .retain(|p| &p.id != package_id);
                }
                SkillCatalogAction::Move {
                    skill_id,
                    destination,
                } => {
                    validate_scope(destination)?;
                    let mut record = next
                        .record(skill_id)
                        .cloned()
                        .ok_or_else(|| failure("skill was removed"))?;
                    if record.scope != mutation.scope {
                        return Err(failure("move must name the source scope"));
                    }
                    next.remove_skill(&mutation.scope, skill_id)?;
                    record.scope = destination.clone();
                    record.package_id = None;
                    next.catalog_mut(destination).skills.push(record);
                }
            }
        }
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| failure("catalog revision exhausted"))?;
        for catalog in &mut next.catalogs {
            catalog.revision = next.revision;
        }
        next.validate()?;
        Ok(next)
    }
    fn package_mut(
        &mut self,
        scope: &SkillScope,
        id: &str,
    ) -> Result<&mut SkillPackageRecord, DaemonError> {
        self.catalog_mut(scope)
            .packages
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| failure("package was removed or is outside the selected scope"))
    }
    fn set_state(
        &mut self,
        scope: &SkillScope,
        id: &str,
        state: Option<SkillState>,
    ) -> Result<(), DaemonError> {
        let record_scope = self
            .record(id)
            .ok_or_else(|| failure("skill was removed"))?
            .scope
            .clone();
        if &record_scope == scope {
            self.record_mut(id)?.state_override = state;
        } else if record_scope == SkillScope::Chariox && matches!(scope, SkillScope::Project { .. })
        {
            let overrides = &mut self.catalog_mut(scope).chariox_overrides;
            if let Some(state) = state {
                overrides.insert(id.into(), state);
            } else {
                overrides.remove(id);
            }
        } else {
            return Err(failure("skill is outside the selected scope"));
        }
        Ok(())
    }
    fn remove_skill(&mut self, scope: &SkillScope, id: &str) -> Result<(), DaemonError> {
        if &self
            .record(id)
            .ok_or_else(|| failure("skill was removed"))?
            .scope
            != scope
        {
            return Err(failure("skill is outside the selected scope"));
        }
        self.catalog_mut(scope).skills.retain(|s| s.id != id);
        for catalog in &mut self.catalogs {
            catalog.chariox_overrides.remove(id);
            for package in &mut catalog.packages {
                package.upstream_members.retain(|_, member| member != id);
            }
        }
        Ok(())
    }
}

pub fn safe_locator(locator: &str) -> Result<String, DaemonError> {
    if let Ok(mut url) = url::Url::parse(locator) {
        if !matches!(url.scheme(), "https" | "http" | "file") {
            return Err(failure("unsupported skill source URL scheme"));
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return Ok(url.to_string());
    }
    if locator.contains('\0') || locator.len() > 4096 {
        return Err(failure("invalid skill source locator"));
    }
    Ok(locator.into())
}
