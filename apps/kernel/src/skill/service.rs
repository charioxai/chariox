//! MP-08 / MP-10 / MP-11: verified immutable install preparation and pinned turns.
//! Callers must retain the normal home mutation authorization around publication.
use super::{
    catalog::{failure, safe_locator, SkillCatalogSet},
    frontmatter::parse_frontmatter,
    package::package_skill_directory,
    resolver::*,
    selection::explicit_skill_names,
    storage::SkillCatalogStorage,
    v2::*,
    CharioxSkillPackage,
};
use crate::error::DaemonError;
use base64::Engine;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct SkillCatalogService {
    pub storage: SkillCatalogStorage,
}

impl SkillCatalogService {
    pub fn new(storage: SkillCatalogStorage) -> Self {
        Self { storage }
    }
    pub fn projections(
        &self,
        project_id: Option<&str>,
        native: &[NativeSkillEntry],
    ) -> Result<(u64, Vec<SkillProjection>), DaemonError> {
        let snapshot = self.storage.snapshot()?;
        Ok((
            snapshot.revision,
            projections(&snapshot, project_id, native),
        ))
    }
    /// Prepare sources without publishing names, grants, catalog records or scripts.
    pub fn prepare(
        &self,
        mutation: &SkillCatalogMutation,
    ) -> Result<BTreeMap<usize, (SkillRecord, CharioxSkillPackage)>, DaemonError> {
        let current = self.storage.snapshot()?;
        if current.revision != mutation.expected_revision {
            return Err(failure("catalog changed; refresh before reviewing sources"));
        }
        let mut prepared = BTreeMap::new();
        for (index, action) in mutation.actions.iter().enumerate() {
            let source = match action {
                SkillCatalogAction::Install { source, .. }
                | SkillCatalogAction::Update { source, .. } => source,
                _ => continue,
            };
            let (package, locator) = self.prepare_source(source)?;
            let body = package
                .files
                .iter()
                .find(|f| f.path == "SKILL.md")
                .ok_or_else(|| failure("package has no SKILL.md"))?;
            let body = base64::engine::general_purpose::STANDARD
                .decode(&body.content_base64)
                .map_err(|_| failure("invalid skill body encoding"))?;
            let frontmatter = parse_frontmatter(
                std::str::from_utf8(&body).map_err(|_| failure("SKILL.md must be UTF-8"))?,
            )?;
            let now = crate::session::unix_epoch_ms();
            let record = SkillRecord {
                id: format!("skill-{:032x}", rand::random::<u128>()),
                name: frontmatter.name,
                scope: mutation.scope.clone(),
                package_id: None,
                description: frontmatter.description,
                short_description: frontmatter.short_description,
                default_state: frontmatter.default_state,
                state_override: None,
                digest: package.version_hash.clone(),
                declared_version: frontmatter.declared_version,
                files: package
                    .files
                    .iter()
                    .map(|f| {
                        Ok(SkillFileManifest {
                            path: f.path.clone(),
                            sha256: f.sha256.clone(),
                            bytes: base64::engine::general_purpose::STANDARD
                                .decode(&f.content_base64)
                                .map_err(|_| failure("invalid package file"))?
                                .len() as u64,
                            executable: f.executable,
                        })
                    })
                    .collect::<Result<_, DaemonError>>()?,
                provenance: SkillProvenance {
                    kind: match source {
                        SkillInstallSource::Folder { .. } => "folder",
                        SkillInstallSource::Content { .. } => "content",
                        SkillInstallSource::Url { .. } => "url",
                        SkillInstallSource::Native { .. } => "provider_import",
                    }
                    .into(),
                    source_machine: None,
                    locator,
                    source_revision: match source {
                        SkillInstallSource::Url {
                            pinned_revision, ..
                        } => pinned_revision.clone(),
                        _ => None,
                    },
                    provider: None,
                    account_profile: None,
                    installed_at_ms: now,
                    updated_at_ms: now,
                },
                compatibility_notes: frontmatter.compatibility_notes,
            };
            prepared.insert(index, (record, package));
        }
        let records = prepared.iter().map(|(i, (r, _))| (*i, r.clone())).collect();
        current.plan(mutation, &records)?;
        Ok(prepared)
    }
    /// This API is deliberately separate from authorization. Runtime tools and UI
    /// commands must use the same reviewed request and home RuntimeInteraction.
    pub fn publish_authorized(
        &self,
        mutation: &SkillCatalogMutation,
        prepared: &BTreeMap<usize, (SkillRecord, CharioxSkillPackage)>,
    ) -> Result<SkillCatalogSet, DaemonError> {
        let records: BTreeMap<_, _> = prepared.iter().map(|(i, (r, _))| (*i, r.clone())).collect();
        let planned = self.storage.snapshot()?.plan(mutation, &records)?;
        for (index, (_, payload)) in prepared {
            let action = &mutation.actions[*index];
            let id = match action {
                SkillCatalogAction::Install {
                    collision: SkillCollisionAction::Replace { skill_id },
                    ..
                }
                | SkillCatalogAction::Update { skill_id, .. } => skill_id,
                _ => &records[index].id,
            };
            let record = planned
                .record(id)
                .ok_or_else(|| failure("prepared skill is absent from reviewed batch"))?;
            let base = self.storage.home().join("skills/packages");
            // IDs and complete digests own immutable bytes; source names never
            // choose an installation path or a provider account directory.
            super::package::materialize_skill_package_for_id(&base, &record.id, payload)?;
        }
        self.storage.publish(mutation, &records)
    }
    pub fn package_root(&self, record: &SkillRecord) -> PathBuf {
        self.storage
            .home()
            .join("skills/packages")
            .join(&record.id)
            .join(&record.digest)
    }
    pub fn pinned_turn(
        &self,
        project_id: Option<&str>,
        agent_id: &str,
        session_id: &str,
        prompt: &str,
        selections: &[SkillSelection],
        native: &[NativeSkillEntry],
        context_capacity: Option<u64>,
        budget_percent: f64,
        remaining_body_tokens: u64,
        context_generation: &str,
    ) -> Result<SkillTurnContext, DaemonError> {
        let (revision, rows) = self.projections(project_id, native)?;
        let mut selected = BTreeSet::new();
        let mut native_selections = Vec::new();
        for name in explicit_skill_names(prompt) {
            if rows
                .iter()
                .any(|row| row.skill.name.eq_ignore_ascii_case(&name))
            {
                selected.insert(resolve_name(&rows, &name)?.skill.id.clone());
            } else {
                let matches: Vec<_> = native.iter().filter(|entry| entry.exposure == NativeSkillExposure::Confirmed).filter(|entry| matches!(&entry.selection, SkillSelection::Native { name: candidate, .. } if candidate.eq_ignore_ascii_case(&name))).collect();
                if matches.len() != 1 {
                    return Err(failure(
                        "unknown or ambiguous skill name; open Skills and select a current entry",
                    ));
                }
                native_selections.push(matches[0].selection.clone());
            }
        }
        for selection in selections {
            match selection {
                SkillSelection::Chariox {
                    skill_id,
                    name,
                    catalog_revision,
                } => {
                    if *catalog_revision != revision {
                        return Err(failure(
                            "selected skill catalog changed; refresh and reselect",
                        ));
                    }
                    let row = resolve_name(&rows, name)?;
                    if &row.skill.id != skill_id {
                        return Err(failure("selected skill was replaced; reselect"));
                    }
                    selected.insert(skill_id.clone());
                }
                SkillSelection::Native { .. } => {
                    if !native.iter().any(|entry| {
                        &entry.selection == selection
                            && entry.exposure == NativeSkillExposure::Confirmed
                    }) {
                        return Err(failure(
                            "native selection is stale or unverified; refresh and reselect",
                        ));
                    }
                    if !native_selections.contains(selection) {
                        native_selections.push(selection.clone());
                    }
                }
            }
        }
        let explicit: Vec<_> = selected.iter().cloned().collect();
        let mut reasons: BTreeMap<String, SkillLoadReason> = selected
            .into_iter()
            .map(|id| (id, SkillLoadReason::Explicit))
            .collect();
        for previous in self.storage.loads(session_id)? {
            if previous.agent_id == agent_id
                && rows.iter().any(|r| {
                    r.skill.id == previous.skill_id && r.effective_state != SkillState::Off
                })
            {
                reasons.entry(previous.skill_id).or_insert(previous.reason);
            }
        }
        let mut loaded = Vec::new();
        let mut total_bytes = 0u64;
        for (id, reason) in reasons {
            // A previous confirmed Manual load remains active in that session.
            let previous_explicit = if reason == SkillLoadReason::Explicit {
                vec![id.clone()]
            } else {
                explicit.clone()
            };
            let row = admit_load(&rows, &id, &previous_explicit)?;
            if let SkillDelivery::ProvidedBy { entry_id, .. } = &row.delivery {
                let entry = native.iter().find(|entry| matches!(&entry.selection, SkillSelection::Native { entry_id: id, .. } if id == entry_id)).ok_or_else(|| failure("native duplicate inventory changed; refresh Skills"))?;
                if !entry.supports_load_ack {
                    return Err(failure("provider-native skill has no supported load acknowledgement seam; refresh Skills or disable the native copy before using Chariox delivery"));
                }
                if !native_selections.contains(&entry.selection) {
                    native_selections.push(entry.selection.clone());
                }
                // Native delivery is responsible for the one acknowledged body.
                loaded.push(SkillResolvedBody {
                    skill_id: row.skill.id.clone(),
                    name: row.skill.name.clone(),
                    digest: row.skill.digest.clone(),
                    root: String::new(),
                    body: String::new(),
                    files: vec![],
                    reason,
                    delivery: row.delivery.clone(),
                });
                continue;
            }
            let root = self.package_root(&row.skill);
            let body = fs::read_to_string(root.join("SKILL.md")).map_err(|_| {
                failure("selected skill package is missing; reinstall it before submitting")
            })?;
            let current = package_skill_directory(&root)?;
            if current.version_hash != row.skill.digest {
                return Err(failure("selected immutable skill package failed verification; reinstall before submitting"));
            }
            let wrapper = format!(
                "\nSkill {} ({}) at {}:\n{}\n",
                row.skill.name,
                row.skill.digest,
                root.display(),
                body
            );
            total_bytes = total_bytes.saturating_add(wrapper.len() as u64);
            if total_bytes > remaining_body_tokens {
                return Err(failure("selected skill bodies exceed remaining turn budget; select fewer or smaller skills"));
            }
            loaded.push(SkillResolvedBody {
                skill_id: row.skill.id.clone(),
                name: row.skill.name.clone(),
                digest: row.skill.digest.clone(),
                root: root.to_string_lossy().into(),
                body,
                files: row.skill.files.clone(),
                reason,
                delivery: row.delivery.clone(),
            });
        }
        let metadata = metadata_context(&rows, context_capacity, budget_percent)?;
        Ok(SkillTurnContext {
            catalog_revision: revision,
            context_generation: context_generation.into(),
            metadata: metadata.text,
            metadata_tokens_upper_bound: metadata.tokens_upper_bound,
            metadata_budget_tokens: metadata.budget_tokens,
            omitted_auto_count: metadata.omitted_count as u32,
            loaded,
            native_selections,
        })
    }
    fn prepare_source(
        &self,
        source: &SkillInstallSource,
    ) -> Result<(CharioxSkillPackage, Option<String>), DaemonError> {
        match source {
            SkillInstallSource::Folder { path } => Ok((
                package_skill_directory(Path::new(path))?,
                Some(safe_locator(path)?),
            )),
            SkillInstallSource::Content { skill_md } => self.content_package(skill_md, None),
            SkillInstallSource::Url { url, .. } => {
                use std::io::Read;
                let parsed = url::Url::parse(url).map_err(|_| failure("invalid skill URL"))?;
                if !matches!(parsed.scheme(), "https" | "http") {
                    return Err(failure("skill URL must use http or https"));
                }
                let response = ureq::AgentBuilder::new()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .get(url)
                    .call()
                    .map_err(|_| failure("skill source download failed"))?;
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .take(512 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| failure("skill source download failed"))?;
                if bytes.len() > 512 * 1024 {
                    return Err(failure("downloaded SKILL.md must be at most 512 KiB"));
                }
                self.content_package(
                    std::str::from_utf8(&bytes)
                        .map_err(|_| failure("downloaded skill must be UTF-8"))?,
                    Some(safe_locator(url)?),
                )
            }
            SkillInstallSource::Native { .. } => Err(failure(
                "provider import requires a confirmed execution-local inventory source",
            )),
        }
    }
    fn content_package(
        &self,
        body: &str,
        locator: Option<String>,
    ) -> Result<(CharioxSkillPackage, Option<String>), DaemonError> {
        parse_frontmatter(body)?;
        if body.len() > 512 * 1024 {
            return Err(failure("SKILL.md must be at most 512 KiB"));
        }
        let staging = self
            .storage
            .home()
            .join("state/skill-staging")
            .join(format!("stage-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&staging).map_err(|_| failure("cannot prepare skill source"))?;
        let result = (|| {
            crate::config::write_private_file(&staging.join("SKILL.md"), body.as_bytes())
                .map_err(|_| failure("cannot prepare skill content"))?;
            package_skill_directory(&staging)
        })();
        let cleanup = fs::remove_dir_all(&staging);
        if cleanup.is_err() && result.is_ok() {
            return Err(failure("skill staging cleanup failed"));
        }
        result.map(|package| (package, locator))
    }
}
