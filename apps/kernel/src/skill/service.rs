//! MP-08 / MP-10 / MP-11: verified immutable install preparation and pinned turns.
//! Callers must retain the normal home mutation authorization around publication.
use super::{
    catalog::{failure, safe_locator, SkillCatalogSet},
    frontmatter::parse_frontmatter,
    package::{instruction_body, package_skill_directory},
    repository::{staging_directory, RepositoryCheckout, RepositoryCheckouts},
    resolver::*,
    selection::explicit_skill_names,
    source_access::SkillSourceAccess,
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

/// MP-08 / MP-10 / MP-11: an execution-side inventory resolves this root;
/// it is not deserializable or accepted as client/model path authority.
#[derive(Debug, Clone)]
pub(crate) struct VerifiedNativeSkillSource {
    pub selection: SkillSelection,
    pub root: PathBuf,
    pub digest: String,
    pub source_machine: Option<String>,
}

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
    /// Lists what a public repository carries at its current commit. Nothing is
    /// installed; the owner's later install pins the returned revision.
    pub fn resolve_repository(&self, url: &str) -> Result<SkillsV2Response, DaemonError> {
        let checkout = RepositoryCheckout::fetch(staging_directory(&self.storage)?, url, None)?;
        let (skills, skipped) = checkout.skills()?;
        if skills.is_empty() {
            return Err(failure("the repository has no installable SKILL.md"));
        }
        Ok(SkillsV2Response::Repository {
            locator: checkout.locator.clone(),
            revision: checkout.revision.clone(),
            skills,
            skipped,
        })
    }
    /// Prepare sources without publishing names, grants, catalog records or scripts.
    pub fn prepare(
        &self,
        mutation: &SkillCatalogMutation,
    ) -> Result<BTreeMap<usize, (SkillRecord, CharioxSkillPackage)>, DaemonError> {
        self.prepare_with_native_sources(mutation, &BTreeMap::new())
    }
    pub(crate) fn prepare_with_native_sources(
        &self,
        mutation: &SkillCatalogMutation,
        native_sources: &BTreeMap<usize, VerifiedNativeSkillSource>,
    ) -> Result<BTreeMap<usize, (SkillRecord, CharioxSkillPackage)>, DaemonError> {
        self.prepare_with_source_access(mutation, native_sources, SkillSourceAccess::Owner)
    }
    pub(crate) fn prepare_with_source_access(
        &self,
        mutation: &SkillCatalogMutation,
        native_sources: &BTreeMap<usize, VerifiedNativeSkillSource>,
        access: SkillSourceAccess<'_>,
    ) -> Result<BTreeMap<usize, (SkillRecord, CharioxSkillPackage)>, DaemonError> {
        let current = self.storage.snapshot()?;
        if current.revision != mutation.expected_revision {
            return Err(failure("catalog changed; refresh before reviewing sources"));
        }
        let mut prepared = BTreeMap::new();
        let mut checkouts = RepositoryCheckouts::default();
        for (index, action) in mutation.actions.iter().enumerate() {
            let source = match action {
                SkillCatalogAction::Install { source, .. }
                | SkillCatalogAction::Update { source, .. } => source,
                _ => continue,
            };
            let mut source_revision = None;
            let (package, locator) = if let SkillInstallSource::Repository {
                url,
                revision,
                path,
            } = source
            {
                if !matches!(access, SkillSourceAccess::Owner) {
                    return Err(failure(
                        "repository changes are owner-initiated; ask the owner to review them from Skills",
                    ));
                }
                let checkout = checkouts.get(&self.storage, url, revision)?;
                source_revision = Some(checkout.revision.clone());
                (checkout.package(path)?, Some(checkout.locator.clone()))
            } else if let SkillInstallSource::Native { selection } = source {
                let verified=native_sources.get(&index)
                    .filter(|verified|&verified.selection==selection)
                    .ok_or_else(||failure("provider import requires a current execution-local source; refresh the focused provider"))?;
                let package = package_skill_directory(&verified.root)?;
                if package.version_hash != verified.digest {
                    return Err(failure(
                        "native source changed after review; refresh and reselect",
                    ));
                }
                (
                    package,
                    Some(safe_locator(&verified.root.to_string_lossy())?),
                )
            } else {
                if native_sources.contains_key(&index) {
                    return Err(failure("native source does not match its reviewed action"));
                }
                self.prepare_source(source, access)?
            };
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
                        SkillInstallSource::Repository { .. } => "repository",
                        SkillInstallSource::Native { .. } => "provider_import",
                    }
                    .into(),
                    source_machine: native_sources
                        .get(&index)
                        .and_then(|source| source.source_machine.clone()),
                    locator,
                    // Only a source adapter that actually resolves a repository
                    // revision may attest one; a caller-supplied label is not proof.
                    source_revision,
                    provider: match source {
                        SkillInstallSource::Native {
                            selection: SkillSelection::Native { provider, .. },
                        } => Some(provider.clone()),
                        _ => None,
                    },
                    account_profile: match source {
                        SkillInstallSource::Native {
                            selection:
                                SkillSelection::Native {
                                    account_profile, ..
                                },
                        } => Some(account_profile.clone()),
                        _ => None,
                    },
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
            let base = self.storage.directory(Path::new("skills/packages"))?;
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
    /// Reads never reserve an immutable digest directory. A missing/corrupt
    /// package can be repaired through the normal reviewed update path.
    pub(crate) fn verified_package_root(
        &self,
        record: &SkillRecord,
    ) -> Result<PathBuf, DaemonError> {
        super::paths::existing_owned_directory(
            self.storage.home(),
            &Path::new("skills/packages")
                .join(&record.id)
                .join(&record.digest),
        )
        .map_err(|_| {
            failure("installed skill package is missing or invalid; reinstall before submitting")
        })
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Turn pinning takes the whole admitted turn input as plain values; a request type would rewrite every catalog test call"
    )]
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
        self.pinned_turn_from_snapshot(
            &self.storage.snapshot()?,
            project_id,
            agent_id,
            session_id,
            prompt,
            selections,
            native,
            context_capacity,
            budget_percent,
            remaining_body_tokens,
            context_generation,
            None,
        )
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Turn pinning takes the whole admitted turn input as plain values; a request type would rewrite every catalog test call"
    )]
    pub(crate) fn pinned_turn_from_snapshot(
        &self,
        catalog: &SkillCatalogSet,
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
        live_run_id: Option<&str>,
    ) -> Result<SkillTurnContext, DaemonError> {
        catalog.validate()?;
        let revision = catalog.revision;
        let rows = projections(catalog, project_id, native);
        let mut selected = BTreeSet::new();
        let mut native_selections = Vec::new();
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
        // A typed chip owns its printed token's source. Validate those identities
        // first; only unbound textual names use ordinary Chariox precedence.
        for name in explicit_skill_names(prompt) {
            if selections.iter().any(|selection| match selection {
                SkillSelection::Chariox { name: bound, .. }
                | SkillSelection::Native { name: bound, .. } => bound.eq_ignore_ascii_case(&name),
            }) {
                continue;
            }
            if rows
                .iter()
                .any(|row| row.skill.name.eq_ignore_ascii_case(&name))
            {
                selected.insert(resolve_name(&rows, &name)?.skill.id.clone());
            } else {
                let matches: Vec<_> = native.iter().filter(|entry| entry.exposure == NativeSkillExposure::Confirmed).filter(|entry| matches!(&entry.selection, SkillSelection::Native { name: candidate, .. } if candidate.eq_ignore_ascii_case(&name))).collect();
                match matches.as_slice() {
                    [] => continue, // Unresolved ordinary dollar text is not an invocation.
                    [entry] => native_selections.push(entry.selection.clone()),
                    _ => {
                        return Err(failure(
                            "ambiguous skill name; open Skills and select a current entry",
                        ))
                    }
                }
            }
        }
        let explicit: Vec<_> = selected.iter().cloned().collect();
        let mut reasons: BTreeMap<String, SkillLoadReason> = selected
            .into_iter()
            .map(|id| (id, SkillLoadReason::Explicit))
            .collect();
        let previous_loads = self
            .storage
            .active_loads(session_id, project_id, revision)?;
        // The provider run still holds what it was given earlier; only a new run,
        // a compaction, a profile switch or a changed body loses it. A skill whose
        // current body the live run already holds is skipped whatever older
        // records of it exist on previous runs or bodies.
        let held: BTreeSet<&str> = previous_loads
            .iter()
            .filter(|load| load.agent_id == agent_id && live_run_id == Some(load.run_id.as_str()))
            .filter(|load| {
                rows.iter()
                    .any(|row| row.skill.id == load.skill_id && row.skill.digest == load.digest)
            })
            .map(|load| load.skill_id.as_str())
            .collect();
        for previous in previous_loads.iter().rev().cloned() {
            if held.contains(previous.skill_id.as_str()) {
                continue;
            }
            if previous.agent_id == agent_id
                && rows.iter().any(|r| {
                    r.skill.id == previous.skill_id && r.effective_state != SkillState::Off
                        && (r.effective_state != SkillState::Manual
                            || previous.reason == SkillLoadReason::Explicit
                            || explicit.contains(&r.skill.id))
                        && (explicit.contains(&r.skill.id) || !selections.iter().any(|selection| matches!(selection,
                            SkillSelection::Native { name, .. } if name.eq_ignore_ascii_case(&r.skill.name)
                        )))
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
            // A native copy that cannot acknowledge its own load cannot own the
            // one body of this turn: Chariox delivers its equal copy instead.
            let mut delivery = row.delivery.clone();
            if let SkillDelivery::ProvidedBy { entry_id, .. } = &row.delivery {
                let entry = native.iter().find(|entry| matches!(&entry.selection, SkillSelection::Native { entry_id: id, .. } if id == entry_id)).ok_or_else(|| failure("native duplicate inventory changed; refresh Skills"))?;
                if entry.supports_load_ack {
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
                delivery = SkillDelivery::Chariox;
            }
            let root = self.verified_package_root(&row.skill)?;
            let current = package_skill_directory(&root)?;
            if current.version_hash != row.skill.digest {
                return Err(failure("selected immutable skill package failed verification; reinstall before submitting"));
            }
            // Deliver the bytes that were hashed, not a separate pathname read
            // that could observe another file between verification and delivery.
            let body = instruction_body(&current)?;
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
                delivery,
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
        access: SkillSourceAccess<'_>,
    ) -> Result<(CharioxSkillPackage, Option<String>), DaemonError> {
        match source {
            SkillInstallSource::Folder { path } => {
                let package=match access {
                    SkillSourceAccess::Owner=>package_skill_directory(Path::new(path))?,
                    SkillSourceAccess::Agent{workspace:Some(workspace)}=>super::package::package_skill_directory_beneath(workspace,Path::new(path))?,
                    SkillSourceAccess::Agent{workspace:None}=>return Err(failure("remote agent folder sources must be read on the execution kernel; use content or a public URL until package transfer completes")),
                };
                let locator = package
                    .metadata
                    .path
                    .parent()
                    .ok_or_else(|| failure("skill source root is unavailable"))?;
                let locator = Some(safe_locator(&locator.to_string_lossy())?);
                Ok((package, locator))
            }
            SkillInstallSource::Content { skill_md } => self.content_package(skill_md, None),
            SkillInstallSource::Url {
                url,
                pinned_revision,
            } => {
                if pinned_revision.is_some() {
                    return Err(failure("a downloaded SKILL.md cannot attest a revision; import the whole repository instead (Skills > Import a public repository, or /skills repo <url>)"));
                }
                use std::io::Read;
                let parsed = url::Url::parse(url).map_err(|_| failure("invalid skill URL"))?;
                if !matches!(parsed.scheme(), "https" | "http") {
                    return Err(failure("skill URL must use http or https"));
                }
                let response = access
                    .download_agent(url)?
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
            SkillInstallSource::Repository { .. } => Err(failure(
                "repository sources are prepared together with their reviewed checkout",
            )),
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
        let staging = self.storage.directory(
            &Path::new("state/skill-staging")
                .join(format!("stage-{:032x}", rand::random::<u128>())),
        )?;
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
