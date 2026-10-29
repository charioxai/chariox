//! The normal local pack/validate commands also build portable reference Apps.
//! Keep the same held file identities and publisher enrollment rules as v1.
use std::{collections::BTreeSet, path::Path};

use serde_json::{json, Value};

use super::{fs, keys, read_bundle_open, PublisherFile};
use crate::{
    json as package_json,
    v2::{self, ManifestV2, PolicyV2, RequiredFeature, SupportedWorld},
    ErrorCode, Limits, PackageError, Result, TrustedPublisher,
};

const CONTROL_ENTRIES: usize = 3;

fn policy(publisher: TrustedPublisher, limits: &Limits) -> PolicyV2 {
    let mut policy = PolicyV2::new(
        BTreeSet::from([
            RequiredFeature::WasmComponent,
            RequiredFeature::UiActions,
            RequiredFeature::AgentPanel,
            RequiredFeature::Connections,
            RequiredFeature::DirectedEvents,
        ]),
        vec![
            SupportedWorld {
                name: "chariox:app/app".into(),
                major: 1,
                minor: 0,
            },
            SupportedWorld {
                name: "chariox:app/migration".into(),
                major: 1,
                minor: 0,
            },
        ],
        vec![publisher],
    );
    policy.limits = limits.clone();
    policy
}

pub(super) fn is_manifest(path: &Path, limits: &Limits) -> Result<bool> {
    let bytes = fs::read(path, limits.max_metadata_bytes, false)?;
    let value: Value = package_json::read_json(
        &bytes,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        false,
    )?;
    Ok(value["schema"] == v2::APP_SCHEMA_V2)
}

pub(super) fn pack_directory(
    bundle: &Path,
    manifest: &Path,
    private_key: &Path,
    output: &Path,
    limits: &Limits,
) -> Result<Value> {
    let bytes = fs::read(manifest, limits.max_metadata_bytes, false)?;
    let manifest: ManifestV2 = package_json::read_json(
        &bytes,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        false,
    )?;
    manifest.validate(limits)?;
    let bundle = fs::Directory::open(bundle)?;
    let signing = keys::read_signing_key(private_key)?;
    let destination = fs::Destination::open(output, false)?;
    fs::outside_bundle(&bundle, &signing.directory)?;
    fs::outside_bundle(&bundle, &destination.directory)?;
    if manifest.publisher.key_id != keys::key_id(&signing.key.verifying_key()) {
        return Err(PackageError::new(
            ErrorCode::InvalidDeveloperKey,
            "manifest publisher key ID does not match the private signing key",
        ));
    }
    let files = read_bundle_open(&bundle, limits, Some(fs::identity(&signing.file)?))?;
    let bytes = v2::pack(&manifest, &files, &signing.key, &[], limits)?;
    let policy = policy(
        TrustedPublisher {
            publisher_id: manifest.publisher.id.clone(),
            key_id: manifest.publisher.key_id.clone(),
            public_key: signing.key.verifying_key(),
        },
        limits,
    );
    let verified = v2::verify(&bytes, &policy)?;
    let report = json!({"status":"packed-locally", "manifest":verified.manifest(),
        "packageDigest":verified.digests().content, "fileCount":files.len()});
    fs::outside_bundle(&bundle, &destination.directory)?;
    destination.prepare(&bytes)?.publish()?;
    Ok(report)
}

pub(super) fn validate_archive(
    path: &Path,
    publisher: &PublisherFile,
    limits: &Limits,
) -> Result<Option<Value>> {
    let bytes = fs::read(path, limits.max_archive_bytes, false)?;
    let entries = crate::archive::parse(&bytes, limits)?;
    let first = entries
        .first()
        .ok_or_else(|| PackageError::new(ErrorCode::InvalidManifest, "package is empty"))?;
    let value: Value = package_json::read_json(
        first.data,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        false,
    )?;
    if value["schema"] != v2::APP_SCHEMA_V2 {
        return Ok(None);
    }
    let verified = v2::verify(&bytes, &policy(publisher.trusted_publisher()?, limits))?;
    Ok(Some(
        json!({"status":"verified-against-explicit-publisher-file", "manifest":verified.manifest(),
            "packageDigest":verified.digests().content, "fileCount":entries.len().saturating_sub(CONTROL_ENTRIES + verified.reviews().len())}),
    ))
}
