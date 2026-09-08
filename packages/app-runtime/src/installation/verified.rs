//! Verified package provenance and trust are fenced with the stage transaction.
//! Staging still does not approve capabilities, extract files, launch code or
//! activate an installation. Plain metadata APIs remain kernel/test foundations.

use super::{
    commit_generation, create_installation, current_update, identifier, load_installation,
    load_update, sql_generation, stage_release, validate_release, ActiveGeneration,
    InstallationError, InstallationRegistry, ReleaseMetadata, StageToken, UpdateRecord,
};
use crate::publisher_trust::{PublisherTrustError, TrustedPublisherSnapshot};
use chariox_app_package::VerifiedPackage;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

mod first_install;

#[derive(Debug, thiserror::Error)]
pub enum VerifiedStageError {
    #[error(transparent)]
    Installation(#[from] InstallationError),
    #[error(transparent)]
    Trust(#[from] PublisherTrustError),
    #[error("app_verified_stage_signer_mismatch")]
    SignerMismatch,
    #[error("app_verified_stage_metadata")]
    Metadata,
}
impl From<rusqlite::Error> for VerifiedStageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Installation(InstallationError::Database(error))
    }
}
type Result<T> = std::result::Result<T, VerifiedStageError>;

/// This immutable candidate can only be built from the package verifier's
/// private proof type and an enrolled snapshot. It accepts no caller metadata.
#[derive(Debug, Clone)]
pub struct VerifiedInstallCandidate {
    release: ReleaseMetadata,
    trust: TrustedPublisherSnapshot,
    review: serde_json::Value,
}
impl VerifiedInstallCandidate {
    pub fn from_verified(
        package: &VerifiedPackage<'_>,
        trust: &TrustedPublisherSnapshot,
    ) -> Result<Self> {
        let signer = package.signer();
        let enrolled = trust.publisher();
        if signer.publisher_id != enrolled.publisher_id
            || signer.key_id != enrolled.key_id
            || signer.public_key.as_bytes() != enrolled.public_key.as_bytes()
        {
            return Err(VerifiedStageError::SignerMismatch);
        }
        let manifest = package.manifest();
        let declarations = package.declarations();
        let capabilities_digest = json_digest(&json!({
            "schema": "chariox.installation-capabilities.v1",
            "capabilities": manifest.capabilities,
            "actions": declarations.actions,
            "informationSets": declarations.information_sets,
            "toolActions": declarations.tools.iter().filter_map(|tool| tool.action.as_ref().map(|action|
                json!({"tool": tool.name, "action": action}))).collect::<Vec<_>>()
        }))?;
        let catalog_digest = json_digest(&json!({
            "schema": "chariox.installation-catalog.v1", "declarations": declarations
        }))?;
        // A view can use shared/dynamic assets outside ui/. Until the serving
        // contract narrows that set, every signed payload file participates.
        // BTreeMap-backed files() supplies a deterministic path ordering.
        let assets: Vec<_> = package
            .files()
            .map(|(path, bytes)| {
                json!({
                    "path": path, "size": bytes.len(), "digest": digest(bytes)
                })
            })
            .collect();
        let view_digest = json_digest(&json!({
            "schema": "chariox.installation-view.v1", "entry": manifest.ui.entry, "assets": assets
        }))?;
        let release = ReleaseMetadata {
            app_id: manifest.app_id.clone(),
            version: manifest.version.clone(),
            publisher_id: signer.publisher_id.clone(),
            package_digest: package.package_digest().into(),
            schema_version: manifest
                .migrations
                .as_ref()
                .map_or(0, |migrations| migrations.target_version),
            capabilities_digest,
            catalog_digest,
            view_digest,
        };
        validate_release(&release)?;
        let review = json!({
            "appId": release.app_id, "version": release.version,
            "publisherId": signer.publisher_id, "keyId": signer.key_id,
            "keyFingerprint": digest(signer.public_key.as_bytes()), "trustRevision": trust.revision().to_string(),
            "packageDigest": release.package_digest, "capabilitiesDigest": release.capabilities_digest,
            "capabilities": manifest.capabilities,
            "actions": declarations.actions.iter().map(|a| json!({"name":a.name,"criticalValidation":a.critical_validation,"effectRoutes":a.effect_routes})).collect::<Vec<_>>(),
            "declaredInformationSets": declarations.information_sets.iter().map(|i| json!({"name":i.name,"purpose":i.purpose,"sourceScope":i.source_scope})).collect::<Vec<_>>(),
            "informationSetConsent": "not_granted"
        });
        Ok(Self {
            release,
            trust: trust.clone(),
            review,
        })
    }
    /// Human review data derived from the exact verified package. Descriptions
    /// are display data; declared information sets are explicitly not consent.
    pub fn review_metadata(&self) -> &serde_json::Value {
        &self.review
    }
    pub fn release_metadata(&self) -> &ReleaseMetadata {
        &self.release
    }
}

/// Durable signer identity retained alongside one exact stage/generation.
/// Private fields prevent callers from forging a binding or replacing its
/// package digest. Activation must recheck this binding before commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTrustBinding {
    token: StageToken,
    owner_id: String,
    package_digest: String,
    publisher_id: String,
    key_id: String,
    trust_revision: u64,
    public_key_fingerprint: String,
}
impl StageTrustBinding {
    pub fn token(&self) -> &StageToken {
        &self.token
    }
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }
    pub fn package_digest(&self) -> &str {
        &self.package_digest
    }
    pub fn publisher_id(&self) -> &str {
        &self.publisher_id
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn trust_revision(&self) -> u64 {
        self.trust_revision
    }
    pub fn public_key_fingerprint(&self) -> &str {
        &self.public_key_fingerprint
    }

    /// Call in the SAME writer transaction as the pending stage's activation.
    /// The existing prepared/approval checks still apply. Revocation, a later
    /// enrollment revision, aborted/replaced stages or another owner fail.
    pub fn require_current(
        &self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
        current_trust: &TrustedPublisherSnapshot,
    ) -> Result<()> {
        self.require_binding(transaction, trusted_owner)?;
        current_update(transaction, &self.token)?;
        self.require_signer(transaction, trusted_owner, current_trust)?;
        Ok(())
    }

    /// Admission for an already active worker/catalog. Keep this in the SAME
    /// transaction as each broker read/mutation: an unchanged App generation
    /// does not imply its publisher key is still enrolled. The supervisor must
    /// separately cancel/fence in-flight work when any of these facts changes.
    pub fn require_active(
        &self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
        current_trust: &TrustedPublisherSnapshot,
    ) -> Result<ActiveGeneration> {
        self.require_binding(transaction, trusted_owner)?;
        self.require_signer(transaction, trusted_owner, current_trust)?;
        let installation = load_installation(transaction, &self.token.installation_id)?;
        if installation.generation != self.token.generation {
            return Err(InstallationError::Conflict.into());
        }
        if installation.admission_paused {
            return Err(InstallationError::AdmissionPaused.into());
        }
        let active = installation.active.ok_or(InstallationError::Inactive)?;
        if active.generation != self.token.generation
            || active.release.package_digest != self.package_digest
            || active.release.publisher_id != self.publisher_id
        {
            return Err(InstallationError::Conflict.into());
        }
        Ok(active)
    }

    fn require_binding(&self, transaction: &Transaction<'_>, trusted_owner: &str) -> Result<()> {
        let current = load_binding(transaction, trusted_owner, &self.token)?;
        if current != *self {
            return Err(InstallationError::Conflict.into());
        }
        Ok(())
    }

    fn require_signer(
        &self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
        current_trust: &TrustedPublisherSnapshot,
    ) -> Result<()> {
        current_trust.require_current(transaction, trusted_owner)?;
        let publisher = current_trust.publisher();
        if self.publisher_id != publisher.publisher_id
            || self.key_id != publisher.key_id
            || self.trust_revision != current_trust.revision()
            || self.public_key_fingerprint != digest(publisher.public_key.as_bytes())
        {
            return Err(PublisherTrustError::Conflict.into());
        }
        Ok(())
    }
}

impl InstallationRegistry<'_> {
    pub fn create_and_stage_verified(
        &mut self,
        installation_id: &str,
        trusted_owner: &str,
        candidate: &VerifiedInstallCandidate,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        candidate
            .trust
            .require_current(&transaction, trusted_owner)?;
        create_installation(
            &transaction,
            installation_id,
            &candidate.release.app_id,
            trusted_owner,
        )?;
        let record = stage_release(
            &transaction,
            installation_id,
            0,
            candidate.release.clone(),
            now_ms,
        )?;
        save_binding(&transaction, trusted_owner, &record, &candidate.trust)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn stage_verified(
        &mut self,
        installation_id: &str,
        trusted_owner: &str,
        expected_generation: u64,
        candidate: &VerifiedInstallCandidate,
        now_ms: u64,
    ) -> Result<UpdateRecord> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        candidate
            .trust
            .require_current(&transaction, trusted_owner)?;
        let installation = load_installation(&transaction, installation_id)?;
        if installation.owner_id != trusted_owner {
            return Err(InstallationError::NotFound.into());
        }
        let record = stage_release(
            &transaction,
            installation_id,
            expected_generation,
            candidate.release.clone(),
            now_ms,
        )?;
        save_binding(&transaction, trusted_owner, &record, &candidate.trust)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Activates a prepared, approved generation only while its exact retained
    /// signer enrollment is current. Revocation or a later re-enrollment cannot
    /// be repaired by substituting a fresh snapshot for an older staged binding.
    /// All trust checks and the existing activation mutation share one writer
    /// transaction. External migration/worker preparation remain caller duties.
    pub fn commit_verified(
        &mut self,
        token: &StageToken,
        trusted_owner: &str,
        current_trust: &TrustedPublisherSnapshot,
        now_ms: u64,
    ) -> Result<ActiveGeneration> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let supervised: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_installation_supervised_stages WHERE installation_id=?1 AND generation=?2)",
            params![token.installation_id, sql_generation(token.generation)?], |row| row.get(0),
        )?;
        if supervised {
            return Err(InstallationError::Invalid("supervised activation required").into());
        }
        let binding = load_binding(&transaction, trusted_owner, token)?;
        binding.require_current(&transaction, trusted_owner, current_trust)?;
        let active = commit_generation(&transaction, token, now_ms)?;
        transaction.commit()?;
        Ok(active)
    }

    pub fn staged_trust(
        &self,
        trusted_owner: &str,
        token: &StageToken,
    ) -> Result<StageTrustBinding> {
        load_binding(self.connection, trusted_owner, token)
    }

    /// Resolve retained provenance for the active release, including after its
    /// update journal was pruned. Call require_active in the admission transaction.
    pub fn active_trust(
        &self,
        trusted_owner: &str,
        installation_id: &str,
    ) -> Result<StageTrustBinding> {
        identifier(trusted_owner)?;
        let installation = load_installation(self.connection, installation_id)?;
        if installation.owner_id != trusted_owner {
            return Err(InstallationError::NotFound.into());
        }
        let active = installation.active.ok_or(InstallationError::NotFound)?;
        let base_generation: i64 = self
            .connection
            .query_row(
                "SELECT base_generation FROM app_installation_stage_trust
             WHERE installation_id=?1 AND generation=?2 AND owner_id=?3",
                params![
                    installation_id,
                    sql_generation(active.generation)?,
                    trusted_owner
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(InstallationError::NotFound)?;
        let token = StageToken {
            installation_id: installation_id.into(),
            base_generation: u64::try_from(base_generation)
                .map_err(|_| InstallationError::Conflict)?,
            generation: active.generation,
        };
        load_binding(self.connection, trusted_owner, &token)
    }
}

fn save_binding(
    transaction: &Transaction<'_>,
    owner: &str,
    record: &UpdateRecord,
    trust: &TrustedPublisherSnapshot,
) -> Result<()> {
    let publisher = trust.publisher();
    transaction.execute("INSERT INTO app_installation_stage_trust
        (installation_id,generation,base_generation,owner_id,package_digest,publisher_id,key_id,trust_revision,public_key_fingerprint)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![record.token.installation_id, sql_generation(record.token.generation)?, sql_generation(record.token.base_generation)?,
            owner, record.release.package_digest, publisher.publisher_id, publisher.key_id, sql_generation(trust.revision())?,
            digest(publisher.public_key.as_bytes())])?;
    Ok(())
}

fn load_binding(
    connection: &rusqlite::Connection,
    owner: &str,
    token: &StageToken,
) -> Result<StageTrustBinding> {
    identifier(owner)?;
    let installation = load_installation(connection, &token.installation_id)?;
    if installation.owner_id != owner {
        return Err(InstallationError::NotFound.into());
    }
    type Fields = (i64, String, String, String, String, i64, String);
    let fields: Fields = connection.query_row("SELECT base_generation,owner_id,package_digest,publisher_id,key_id,trust_revision,public_key_fingerprint
        FROM app_installation_stage_trust WHERE installation_id=?1 AND generation=?2 AND owner_id=?3",
        params![token.installation_id, sql_generation(token.generation)?, owner],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))
        .optional()?.ok_or(InstallationError::NotFound)?;
    let release = match load_update(connection, &token.installation_id, token.generation) {
        Ok(record) if record.token == *token => record.release,
        Ok(_) => return Err(InstallationError::Conflict.into()),
        Err(InstallationError::NotFound) => {
            installation
                .active
                .filter(|active| active.generation == token.generation)
                .ok_or(InstallationError::NotFound)?
                .release
        }
        Err(error) => return Err(error.into()),
    };
    if fields.0 < 0
        || fields.0 as u64 != token.base_generation
        || fields.5 <= 0
        || release.package_digest != fields.2
        || release.publisher_id != fields.3
        || fields.6.len() != 71
        || !fields.6.starts_with("sha256:")
        || !fields.6.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(InstallationError::Conflict.into());
    }
    Ok(StageTrustBinding {
        token: token.clone(),
        owner_id: fields.1,
        package_digest: fields.2,
        publisher_id: fields.3,
        key_id: fields.4,
        trust_revision: fields.5 as u64,
        public_key_fingerprint: fields.6,
    })
}

fn json_digest(value: &impl Serialize) -> Result<String> {
    let bytes =
        serde_json_canonicalizer::to_vec(value).map_err(|_| VerifiedStageError::Metadata)?;
    Ok(digest(&bytes))
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
