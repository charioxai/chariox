//! Unsigned development transport. Only the kernel supplies the signing key;
//! the resulting package goes through the ordinary verifier and installer.
use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use serde_json::Value;

use crate::{archive, json, v2, ErrorCode, Limits, Manifest, PackageError, Publisher, Result};

pub fn pack_dev_bundle(
    manifest: &Value,
    files: &BTreeMap<String, Vec<u8>>,
    limits: &Limits,
) -> Result<Vec<u8>> {
    validate_manifest(manifest, limits)?;
    let manifest = json::canonical(manifest)?;
    if manifest.len() > limits.max_metadata_bytes {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "dev manifest exceeds byte limit",
        ));
    }
    let mut entries = vec![
        (archive::CONTROL_PATHS[0], manifest.as_slice()),
        (archive::CONTROL_PATHS[1], &[][..]),
        (archive::CONTROL_PATHS[2], &[][..]),
    ];
    entries.extend(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    archive::encode(&entries, limits)
}

/// No claimed publisher in an unsigned bundle can become signing authority.
/// The kernel chooses both the reserved dev principal and its local key.
pub fn sign_dev_bundle(bytes: &[u8], key: &SigningKey, limits: &Limits) -> Result<Vec<u8>> {
    let entries = archive::parse(bytes, limits)?;
    if !entries[1].data.is_empty() || !entries[2].data.is_empty() {
        return Err(PackageError::new(
            ErrorCode::InvalidSignature,
            "dev push requires an unsigned bundle",
        ));
    }
    let mut manifest: Value = json::read_json(
        entries[0].data,
        limits.max_metadata_bytes,
        limits,
        ErrorCode::InvalidManifest,
        true,
    )?;
    validate_manifest(&manifest, limits)?;
    manifest["publisher"] = serde_json::to_value(Publisher {
        id: v2::DEV_PUBLISHER_ID.into(),
        name: "Local development".into(),
        key_id: "kernel-dev".into(),
    })
    .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "dev publisher encoding failed"))?;
    let files = entries[3..]
        .iter()
        .map(|entry| (entry.path.clone(), entry.data.to_vec()))
        .collect();
    if manifest["schema"] == v2::APP_SCHEMA_V2 {
        let manifest: v2::ManifestV2 = serde_json::from_value(manifest).map_err(|_| {
            PackageError::new(ErrorCode::InvalidManifest, "invalid v2 dev manifest")
        })?;
        v2::pack(&manifest, &files, key, &[], limits)
    } else {
        let manifest: Manifest = serde_json::from_value(manifest)
            .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid dev manifest"))?;
        crate::pack(&manifest, &files, key, limits)
    }
}

fn validate_manifest(manifest: &Value, limits: &Limits) -> Result<()> {
    if manifest["schema"] == v2::APP_SCHEMA_V2 {
        serde_json::from_value::<v2::ManifestV2>(manifest.clone())
            .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid v2 dev manifest"))?
            .validate(limits)
    } else {
        serde_json::from_value::<Manifest>(manifest.clone())
            .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid dev manifest"))?
            .validate(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TrustedPublisher, VerificationPolicy};
    use serde_json::json;

    #[test]
    fn unsigned_bundle_uses_only_the_kernel_identity_and_cannot_verify_as_a_release() {
        let manifest = json!({"schema":"chariox.app.v1", "appId":"com.example.todo", "version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"publisher","name":"Publisher"},
            "sdkVersion":"0.8.0", "appContractVersion":1,"minKernelProtocol":500,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"capabilities":{}});
        let files = BTreeMap::from([
            (
                "runtime/main.js".into(),
                b"export default function register() {}".to_vec(),
            ),
            (
                "ui/index.html".into(),
                b"<!doctype html><title>Dev</title>".to_vec(),
            ),
        ]);
        let key = SigningKey::from_bytes(&[37; 32]);
        let limits = Limits::default();
        let unsigned = pack_dev_bundle(&manifest, &files, &limits).unwrap();
        let policy = VerificationPolicy::new(
            500,
            vec![TrustedPublisher {
                publisher_id: "local_dev".into(),
                key_id: "kernel-dev".into(),
                public_key: key.verifying_key(),
            }],
        );
        assert!(crate::verify(&unsigned, &policy).is_err());
        let signed = sign_dev_bundle(&unsigned, &key, &limits).unwrap();
        let package = crate::verify(&signed, &policy).unwrap();
        assert_eq!(package.manifest().publisher.id, "local_dev");
        assert_eq!(
            package.manifest().app_id,
            manifest["appId"].as_str().unwrap()
        );
        assert_eq!(package.files().count(), files.len());
        assert!(sign_dev_bundle(&signed, &key, &limits).is_err());
        let other = SigningKey::from_bytes(&[38; 32]);
        let other_policy = VerificationPolicy::new(
            500,
            vec![TrustedPublisher {
                publisher_id: "local_dev".into(),
                key_id: "kernel-dev".into(),
                public_key: other.verifying_key(),
            }],
        );
        assert!(crate::verify(&signed, &other_policy).is_err());
    }

    #[test]
    fn v2_dev_transport_keeps_the_frozen_package_contract() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/vectors/v2/package.json")).unwrap();
        let shared = &vectors["shared"];
        let files = shared["files"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, value)| (name.clone(), value.as_str().unwrap().as_bytes().to_vec()))
            .collect();
        let key = SigningKey::from_bytes(&[37; 32]);
        let unsigned = pack_dev_bundle(&shared["manifest"], &files, &Limits::default()).unwrap();
        let signed = sign_dev_bundle(&unsigned, &key, &Limits::default()).unwrap();
        let mut policy = v2::PolicyV2::new(
            std::collections::BTreeSet::from([
                v2::RequiredFeature::WasmComponent,
                v2::RequiredFeature::UiActions,
            ]),
            vec![v2::SupportedWorld {
                name: "chariox:app/app".into(),
                major: 1,
                minor: 0,
            }],
            vec![TrustedPublisher {
                publisher_id: "local_dev".into(),
                key_id: "kernel-dev".into(),
                public_key: key.verifying_key(),
            }],
        );
        policy.kernel_features.extend([
            v2::RequiredFeature::AgentPanel,
            v2::RequiredFeature::Connections,
            v2::RequiredFeature::DirectedEvents,
        ]);
        assert_eq!(
            v2::verify(&signed, &policy)
                .unwrap()
                .manifest()
                .publisher
                .id,
            "local_dev"
        );
    }
}
