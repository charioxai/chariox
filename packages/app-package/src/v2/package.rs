use std::collections::{BTreeMap, BTreeSet};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    manifest::ManifestV2,
    ui_actions::{read_ui_actions, UiActionDeclaration},
    world::{check_world, SupportedWorld, WorldId},
    RequiredFeature, APP_CONTRACT_VERSION_V2, INTEGRITY_SCHEMA_V2, SIGNATURE_SCHEMA_V2,
};
use crate::{
    archive, check_payload, declarations, digest, json, manifest::valid_identifier, read_inventory,
    read_signature, signing_bytes_for, verify_signature, Declarations, ErrorCode, Inventory,
    InventoryFile, Limits, PackageError, PublisherSignature, Result, TrustedPublisher,
    RESOURCE_POLICY,
};

/// Reviewer attestations live at `signatures/reviews/<reviewer-key-id>.json`.
pub const REVIEW_DIR: &str = "signatures/reviews/";
pub const REVIEW_SCHEMA: &str = "chariox.package-review.v1";
pub const MAX_REVIEWS: usize = 8;
pub const MAX_REVIEW_BYTES: usize = 16 * 1024;

/// A reserved slot, not a service. A valid file is parsed and handed back as
/// *unverified*; nothing here grants trust, and a package without reviews is
/// valid on every delivery route. The attestation is stored, not interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewEntry {
    pub schema: String,
    pub key_id: String,
    pub algorithm: String,
    pub attestation: Value,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedReview {
    pub key_id: String,
    pub entry: ReviewEntry,
}

/// The three digests of §4.1. Review entries change only `delivered`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDigests {
    /// Over the signing subject: canonical manifest and inventory, which lists
    /// every payload digest. This is what the publisher signature covers.
    pub content: String,
    /// Over the canonical publisher-only archive: control entries, signature
    /// and payload, with every recognized review entry removed.
    pub publisher_archive: String,
    /// Over the exact delivered bytes (what TUF binds).
    pub delivered: String,
}

/// What the kernel supports. The trusted keys come from the enrolled store; a
/// key inside a package never adds itself.
#[derive(Debug, Clone)]
pub struct PolicyV2 {
    pub kernel_features: BTreeSet<RequiredFeature>,
    pub worlds: Vec<SupportedWorld>,
    pub resource_policies: Vec<String>,
    pub trusted_publishers: Vec<TrustedPublisher>,
    pub limits: Limits,
}

impl PolicyV2 {
    pub fn new(
        kernel_features: BTreeSet<RequiredFeature>,
        worlds: Vec<SupportedWorld>,
        trusted_publishers: Vec<TrustedPublisher>,
    ) -> Self {
        Self {
            kernel_features,
            worlds,
            resource_policies: vec![RESOURCE_POLICY.to_owned()],
            trusted_publishers,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug)]
pub struct VerifiedPackageV2<'a> {
    manifest: ManifestV2,
    signer: TrustedPublisher,
    declarations: Declarations,
    ui_actions: Vec<UiActionDeclaration>,
    digests: PackageDigests,
    reviews: Vec<UnverifiedReview>,
    files: BTreeMap<String, &'a [u8]>,
}

impl<'a> VerifiedPackageV2<'a> {
    pub fn manifest(&self) -> &ManifestV2 {
        &self.manifest
    }
    /// The exact enrolled key that verified the signature.
    pub fn signer(&self) -> &TrustedPublisher {
        &self.signer
    }
    pub fn declarations(&self) -> &Declarations {
        &self.declarations
    }
    pub fn ui_actions(&self) -> &[UiActionDeclaration] {
        &self.ui_actions
    }
    pub fn digests(&self) -> &PackageDigests {
        &self.digests
    }
    pub fn reviews(&self) -> &[UnverifiedReview] {
        &self.reviews
    }
    pub fn file(&self, path: &str) -> Option<&'a [u8]> {
        self.files.get(path).copied()
    }
}

/// Build deterministic bytes. `reviews` are placed by key id, whatever their
/// order, and never enter the publisher inventory or signature.
pub fn pack(
    manifest: &ManifestV2,
    files: &BTreeMap<String, Vec<u8>>,
    key: &SigningKey,
    reviews: &[ReviewEntry],
    limits: &Limits,
) -> Result<Vec<u8>> {
    if files.len() + reviews.len() > limits.max_entries.saturating_sub(3) {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "payload exceeds entry limit",
        ));
    }
    let mut payload_bytes = 0usize;
    for bytes in files.values() {
        payload_bytes = payload_bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= limits.max_archive_bytes)
            .ok_or_else(|| {
                PackageError::new(ErrorCode::ArchiveLimit, "payload exceeds byte limit")
            })?;
    }
    manifest.validate(limits)?;
    let borrowed: BTreeMap<_, _> = files
        .iter()
        .map(|(path, bytes)| (path.clone(), bytes.as_slice()))
        .collect();
    check_payload(
        &manifest.required_paths(),
        manifest.migrations.as_ref(),
        &borrowed,
        limits,
    )?;
    read_documents(manifest, &borrowed, limits)?;
    let inventory = Inventory {
        schema: INTEGRITY_SCHEMA_V2.to_owned(),
        files: files
            .iter()
            .map(|(path, bytes)| InventoryFile {
                path: path.clone(),
                size: bytes.len() as u64,
                digest: digest(bytes),
            })
            .collect(),
    };
    let manifest_value = serde_json::to_value(manifest)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "manifest encoding failed"))?;
    json::check_value(&manifest_value, limits)?;
    let signed_bytes = signing_bytes_for(SIGNATURE_SCHEMA_V2, &manifest_value, &inventory)?;
    let signature = PublisherSignature {
        key_id: manifest.publisher.key_id.clone(),
        algorithm: "ed25519".to_owned(),
        digest: digest(&signed_bytes),
        value: BASE64.encode(key.sign(&signed_bytes).to_bytes()),
    };
    let manifest_bytes = json::canonical(&manifest_value)?;
    let inventory_bytes = json::canonical(&inventory)?;
    let signature_bytes = json::canonical(&signature)?;
    for bytes in [&manifest_bytes, &inventory_bytes, &signature_bytes] {
        if bytes.len() > limits.max_metadata_bytes {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "control file exceeds byte limit",
            ));
        }
    }
    if reviews.len() > MAX_REVIEWS {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "too many review entries",
        ));
    }
    let mut sorted: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for review in reviews {
        let value = serde_json::to_value(review).map_err(|_| {
            PackageError::new(ErrorCode::InvalidSignature, "review encoding failed")
        })?;
        json::check_value(&value, limits)?;
        let bytes = json::canonical(&value)?;
        let path = format!("{REVIEW_DIR}{}.json", review.key_id);
        if bytes.len() > MAX_REVIEW_BYTES || sorted.insert(path, bytes).is_some() {
            return Err(PackageError::new(
                ErrorCode::UnexpectedEntry,
                "review entry is oversized or duplicated",
            ));
        }
        check_review(review, &review.key_id)?;
    }
    let mut entries: Vec<(&str, &[u8])> = vec![
        (archive::CONTROL_PATHS[0], &manifest_bytes),
        (archive::CONTROL_PATHS[1], &inventory_bytes),
        (archive::CONTROL_PATHS[2], &signature_bytes),
    ];
    let mut rest: BTreeMap<&str, &[u8]> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    rest.extend(sorted.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
    entries.extend(rest);
    archive::encode(&entries, limits)
}

pub fn verify<'a>(bytes: &'a [u8], policy: &PolicyV2) -> Result<VerifiedPackageV2<'a>> {
    let limits = &policy.limits;
    let entries = archive::parse(bytes, limits)?;
    let manifest_value: Value = json::read_json(
        entries[0].data,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        true,
    )?;
    let manifest: ManifestV2 = serde_json::from_value(manifest_value.clone()).map_err(|_| {
        PackageError::new(
            ErrorCode::InvalidManifest,
            "manifest does not match chariox.app.v2",
        )
    })?;
    manifest.validate(limits)?;
    let inventory = read_inventory(entries[1].data, INTEGRITY_SCHEMA_V2, limits)?;
    let signature = read_signature(entries[2].data, &manifest.publisher.key_id, limits)?;
    let trusted = policy
        .trusted_publishers
        .iter()
        .find(|trusted| {
            trusted.publisher_id == manifest.publisher.id
                && trusted.key_id == manifest.publisher.key_id
        })
        .ok_or_else(|| {
            PackageError::new(
                ErrorCode::UntrustedPublisher,
                "publisher signing key has not been enrolled",
            )
        })?;
    let signed_bytes = signing_bytes_for(SIGNATURE_SCHEMA_V2, &manifest_value, &inventory)?;
    verify_signature(trusted, &signature, &signed_bytes)?;
    check_compatibility(&manifest, policy)?;

    let mut reviews = Vec::new();
    let mut payload = Vec::new();
    for entry in &entries[3..] {
        if entry.path.starts_with("signatures/") {
            reviews.push(read_review(&entry.path, entry.data, limits)?);
        } else {
            payload.push(entry);
        }
    }
    if reviews.len() > MAX_REVIEWS {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "too many review entries",
        ));
    }
    if inventory.files.len() != payload.len() {
        return Err(PackageError::new(
            ErrorCode::UnexpectedEntry,
            "archive and signed inventory have different file counts",
        ));
    }
    let mut files = BTreeMap::new();
    for (entry, expected) in payload.iter().zip(&inventory.files) {
        if expected.path != entry.path {
            return Err(PackageError::new(
                ErrorCode::UnexpectedEntry,
                "archive path is absent from the signed inventory",
            ));
        }
        if expected.size != entry.data.len() as u64 || expected.digest != digest(entry.data) {
            return Err(PackageError::new(
                ErrorCode::IntegrityMismatch,
                "payload size or digest mismatch",
            ));
        }
        files.insert(entry.path.clone(), entry.data);
    }
    check_payload(
        &manifest.required_paths(),
        manifest.migrations.as_ref(),
        &files,
        limits,
    )?;
    let (declarations, ui_actions) = read_documents(&manifest, &files, limits)?;
    let mut publisher_entries: Vec<(&str, &[u8])> = entries[..3]
        .iter()
        .map(|entry| (entry.path.as_str(), entry.data))
        .collect();
    publisher_entries.extend(
        payload
            .iter()
            .map(|entry| (entry.path.as_str(), entry.data)),
    );
    let digests = PackageDigests {
        content: digest(&signed_bytes),
        publisher_archive: digest(&archive::encode(&publisher_entries, limits)?),
        delivered: digest(bytes),
    };
    Ok(VerifiedPackageV2 {
        manifest,
        signer: trusted.clone(),
        declarations,
        ui_actions,
        digests,
        reviews,
        files,
    })
}

fn read_documents(
    manifest: &ManifestV2,
    files: &BTreeMap<String, &[u8]>,
    limits: &Limits,
) -> Result<(Declarations, Vec<UiActionDeclaration>)> {
    let declarations = declarations::read_declarations(
        declarations::DeclarationPaths {
            tools: manifest.tools.as_deref(),
            events: manifest.events.as_deref(),
            actions: manifest.actions.as_deref(),
        },
        &manifest.capabilities,
        true,
        files,
        limits,
    )?;
    let ui_actions = match &manifest.ui_actions {
        Some(path) => read_ui_actions(files[path.as_str()], limits)?,
        None => Vec::new(),
    };
    Ok((declarations, ui_actions))
}

fn check_compatibility(manifest: &ManifestV2, policy: &PolicyV2) -> Result<()> {
    if manifest.app_contract_version != APP_CONTRACT_VERSION_V2 {
        return Err(PackageError::new(
            ErrorCode::IncompatibleContract,
            "unsupported App contract version",
        ));
    }
    if !policy.resource_policies.contains(&manifest.resource_policy) {
        return Err(PackageError::new(
            ErrorCode::IncompatibleResourcePolicy,
            "unsupported resource policy",
        ));
    }
    let world = WorldId::parse(&manifest.world)?;
    check_world(&world, &policy.worlds).map_err(|refusal| refusal.into_error(&world))?;
    let missing: Vec<_> = manifest
        .required_features
        .iter()
        .filter(|feature| !policy.kernel_features.contains(feature))
        .map(|feature| serde_json::to_string(feature).unwrap_or_default())
        .collect();
    if !missing.is_empty() {
        return Err(PackageError::new(
            ErrorCode::IncompatibleProtocol,
            format!("package requires kernel features: {}", missing.join(", ")),
        ));
    }
    Ok(())
}

fn read_review(path: &str, bytes: &[u8], limits: &Limits) -> Result<UnverifiedReview> {
    let unsupported = |message| PackageError::new(ErrorCode::IncompatibleContract, message);
    let key_id = path
        .strip_prefix(REVIEW_DIR)
        .and_then(|name| name.strip_suffix(".json"))
        .filter(|key_id| valid_identifier(key_id))
        .ok_or_else(|| unsupported("unknown signature entry format"))?;
    let value: Value = json::read_json(
        bytes,
        MAX_REVIEW_BYTES,
        limits,
        ErrorCode::InvalidSignature,
        true,
    )?;
    if value.get("schema").and_then(Value::as_str) != Some(REVIEW_SCHEMA) {
        return Err(unsupported("unknown review format"));
    }
    let entry: ReviewEntry = serde_json::from_value(value)
        .map_err(|_| PackageError::new(ErrorCode::InvalidSignature, "invalid review entry"))?;
    check_review(&entry, key_id)?;
    Ok(UnverifiedReview {
        key_id: key_id.to_owned(),
        entry,
    })
}

fn check_review(entry: &ReviewEntry, key_id: &str) -> Result<()> {
    if entry.schema != REVIEW_SCHEMA
        || entry.key_id != key_id
        || !valid_identifier(key_id)
        || entry.algorithm != "ed25519"
        || !entry.attestation.is_object()
        || entry.value.is_empty()
        || entry.value.len() > 4096
    {
        return Err(PackageError::new(
            ErrorCode::InvalidSignature,
            "invalid review entry",
        ));
    }
    Ok(())
}
