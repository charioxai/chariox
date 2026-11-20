//! MP-08 / MP-10 / MP-11: home-owned source reviews; approval never refetches bytes.
//! Runtime callers retain ordinary owner + ask/yolo authorization around publication.
use super::{
    catalog::{failure, safe_locator, validate_id, SkillCatalogSet},
    package::{materialize_skill_package_for_id, package_skill_directory},
    service::SkillCatalogService,
    v2::*,
    CharioxSkillPackage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[path = "review_retention.rs"]
mod retention;

const REVIEW_KIND: &str = "skills.v2.review";
const REVIEW_TTL_MS: u64 = 30 * 60 * 1000;
struct PendingReview(Option<std::path::PathBuf>);
impl Drop for PendingReview {
    fn drop(&mut self) {
        if let Some(root) = self.0.take() {
            let _ = fs::remove_dir_all(root);
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReview {
    receipt: SkillReviewReceipt,
    caller_user_id: String,
    // Raw URLs, query credentials and duplicate instruction bodies do not enter
    // the journal. The original request is bound by its canonical hash instead.
    publication: SkillCatalogMutation,
    prepared: BTreeMap<usize, SkillRecord>,
}

fn mutation_hash(mutation: &SkillCatalogMutation) -> Result<String, crate::error::DaemonError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(mutation)
                .map_err(|_| failure("cannot encode skill mutation review"))?
        )
    ))
}
fn publication_mutation(
    mutation: &SkillCatalogMutation,
) -> Result<SkillCatalogMutation, crate::error::DaemonError> {
    let mut publication = mutation.clone();
    for action in &mut publication.actions {
        match action {
            // A repository's identity is what makes its package source-backed at
            // publication; the locator is stripped of credentials and query like
            // every stored locator, and nothing is fetched again.
            SkillCatalogAction::Install {
                source: SkillInstallSource::Repository { url, .. },
                ..
            }
            | SkillCatalogAction::Update {
                source: SkillInstallSource::Repository { url, .. },
                ..
            } => *url = safe_locator(url)?,
            SkillCatalogAction::Install { source, .. }
            | SkillCatalogAction::Update { source, .. } => {
                *source = SkillInstallSource::Content {
                    skill_md: "reviewed staged package".into(),
                };
            }
            _ => {}
        }
    }
    Ok(publication)
}
impl SkillCatalogService {
    pub fn preview_review(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
    ) -> Result<SkillsV2Response, crate::error::DaemonError> {
        self.preview_review_with_native_sources(caller_user_id, mutation, &BTreeMap::new())
    }
    pub(crate) fn preview_review_with_native_sources(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
        native_sources: &BTreeMap<usize, super::service::VerifiedNativeSkillSource>,
    ) -> Result<SkillsV2Response, crate::error::DaemonError> {
        self.preview_review_with_source_access(
            caller_user_id,
            mutation,
            native_sources,
            super::source_access::SkillSourceAccess::Owner,
        )
    }
    pub(crate) fn preview_review_with_source_access(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
        native_sources: &BTreeMap<usize, super::service::VerifiedNativeSkillSource>,
        access: super::source_access::SkillSourceAccess<'_>,
    ) -> Result<SkillsV2Response, crate::error::DaemonError> {
        if caller_user_id.is_empty() {
            return Err(failure(
                "a home owner identity is required for skill review",
            ));
        }
        retention::sweep(self, crate::session::unix_epoch_ms())?;
        let prepared = self.prepare_with_source_access(mutation, native_sources, access)?;
        let records: BTreeMap<_, _> = prepared
            .iter()
            .map(|(index, (record, _))| (*index, record.clone()))
            .collect();
        let current = self.storage.snapshot()?;
        let planned = current.plan(mutation, &records)?;
        let receipt_id = format!("review-{:032x}", rand::random::<u128>());
        let root = self
            .storage
            .directory(&Path::new("state/skill-reviews").join(&receipt_id))?;
        let mut pending = PendingReview(Some(root.clone()));
        let mut sources = Vec::new();
        for (index, (record, payload)) in &prepared {
            materialize_skill_package_for_id(&root, &format!("member-{index}"), payload)?;
            let effective_id = match &mutation.actions[*index] {
                SkillCatalogAction::Install {
                    collision: SkillCollisionAction::Replace { skill_id },
                    ..
                }
                | SkillCatalogAction::Update { skill_id, .. } => skill_id,
                _ => &record.id,
            };
            sources.push(SkillReviewedSource {
                action_index: u32::try_from(*index)
                    .map_err(|_| failure("skill review batch is too large"))?,
                skill_id: effective_id.clone(),
                digest: record.digest.clone(),
                source_revision: record.provenance.source_revision.clone(),
            });
        }
        let receipt = SkillReviewReceipt {
            id: receipt_id,
            expires_at_ms: crate::session::unix_epoch_ms().saturating_add(REVIEW_TTL_MS),
            mutation_sha256: mutation_hash(mutation)?,
            sources,
        };
        let stored = StoredReview {
            receipt: receipt.clone(),
            caller_user_id: caller_user_id.into(),
            publication: publication_mutation(mutation)?,
            prepared: records,
        };
        self.storage.journal().append_event(
            REVIEW_KIND,
            Some(receipt.id.clone()),
            serde_json::to_value(&stored).map_err(|_| failure("cannot encode skill review"))?,
        )?;
        pending.0 = None;
        Ok(SkillsV2Response::Preview {
            expected_revision: current.revision,
            affected: review_affected(&current, &planned, mutation),
            collisions: vec![],
            review_receipt: receipt,
        })
    }
    /// Read the same reviewed set for an existing home RuntimeInteraction. The
    /// runtime must authorize before calling publish_reviewed; no grant is made.
    pub fn reviewed_changes(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
        receipt_id: &str,
    ) -> Result<(SkillReviewReceipt, Vec<SkillRecord>), crate::error::DaemonError> {
        let stored = self.checked_review(caller_user_id, mutation, receipt_id)?;
        let current = self.storage.snapshot()?;
        let next = current.plan(&stored.publication, &stored.prepared)?;
        Ok((
            stored.receipt,
            review_affected(&current, &next, &stored.publication),
        ))
    }
    pub fn publish_reviewed(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
        receipt_id: &str,
    ) -> Result<SkillCatalogSet, crate::error::DaemonError> {
        let stored = self.checked_review(caller_user_id, mutation, receipt_id)?;
        let root = super::paths::existing_owned_directory(
            self.storage.home(),
            &Path::new("state/skill-reviews").join(receipt_id),
        )?;
        let mut prepared = BTreeMap::<usize, (SkillRecord, CharioxSkillPackage)>::new();
        for (index, record) in &stored.prepared {
            let source = super::paths::existing_owned_directory(
                self.storage.home(),
                &Path::new("state/skill-reviews")
                    .join(receipt_id)
                    .join(format!("member-{index}"))
                    .join(&record.digest),
            )?;
            let package = package_skill_directory(&source)?;
            if package.version_hash != record.digest {
                return Err(failure(
                    "reviewed source failed verification; preview again",
                ));
            }
            prepared.insert(*index, (record.clone(), package));
        }
        // Catalog publication still rechecks the durable revision under its
        // existing collision lock. Concurrent/replayed approval cannot overwrite.
        let published = self.publish_authorized(&stored.publication, &prepared)?;
        // Failure to clean a private review must not invite replay of a committed
        // mutation. Its revision has advanced, so the receipt can no longer apply.
        if fs::remove_dir_all(root).is_err() {
            crate::logging::warn_with_fields(
                "skills.v2.review",
                "published skill review requires staging cleanup",
                serde_json::json!({"revision":published.revision}),
            );
        }
        Ok(published)
    }
    fn checked_review(
        &self,
        caller_user_id: &str,
        mutation: &SkillCatalogMutation,
        receipt_id: &str,
    ) -> Result<StoredReview, crate::error::DaemonError> {
        validate_id(receipt_id)?;
        retention::sweep(self, crate::session::unix_epoch_ms())?;
        let events =
            self.storage
                .journal()
                .load_subject_events_by_kind(receipt_id, REVIEW_KIND, 1)?;
        let event = events
            .last()
            .ok_or_else(|| failure("skill review is missing; preview again"))?;
        let stored: StoredReview = serde_json::from_value(event.payload.clone())
            .map_err(|_| failure("invalid durable skill review"))?;
        if stored.caller_user_id != caller_user_id
            || stored.receipt.id != receipt_id
            || stored.receipt.mutation_sha256 != mutation_hash(mutation)?
            || stored.publication.scope != mutation.scope
            || stored.publication.expected_revision != mutation.expected_revision
        {
            return Err(failure(
                "skill review does not match this home owner and mutation; preview again",
            ));
        }
        if stored.receipt.expires_at_ms <= crate::session::unix_epoch_ms() {
            return Err(failure("skill review expired; preview again"));
        }
        if self.storage.snapshot()?.revision != mutation.expected_revision {
            return Err(failure("catalog changed after review; preview again"));
        }
        Ok(stored)
    }
}
fn review_affected(
    before: &SkillCatalogSet,
    after: &SkillCatalogSet,
    mutation: &SkillCatalogMutation,
) -> Vec<SkillRecord> {
    let old: BTreeMap<_, _> = before
        .catalogs
        .iter()
        .flat_map(|c| &c.skills)
        .map(|r| (r.id.clone(), r))
        .collect();
    let new: BTreeMap<_, _> = after
        .catalogs
        .iter()
        .flat_map(|c| &c.skills)
        .map(|r| (r.id.clone(), r))
        .collect();
    let mut ids: BTreeSet<_> = old
        .keys()
        .chain(new.keys())
        .filter(|id| old.get(*id) != new.get(*id))
        .cloned()
        .collect();
    for action in &mutation.actions {
        match action {
            SkillCatalogAction::Remove { skill_id }
            | SkillCatalogAction::SetState { skill_id, .. }
            | SkillCatalogAction::Move { skill_id, .. } => {
                ids.insert(skill_id.clone());
            }
            SkillCatalogAction::SetPackageState { package_id, .. }
            | SkillCatalogAction::RemovePackage { package_id, .. }
            | SkillCatalogAction::RenamePackage { package_id, .. } => {
                ids.extend(
                    old.values()
                        .chain(new.values())
                        .filter(|r| r.package_id.as_ref() == Some(package_id))
                        .map(|r| r.id.clone()),
                );
            }
            _ => {}
        }
    }
    ids.into_iter()
        .filter_map(|id| new.get(&id).or_else(|| old.get(&id)).map(|r| (*r).clone()))
        .collect()
}
