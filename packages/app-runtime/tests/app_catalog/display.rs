//! The release metadata carries the signed `displayName` and icon, or nothing.
use super::*;
use chariox_app_package::{v2, VerifiedApp, VerifiedAppPackage};
use chariox_app_runtime::installation::ReleaseMetadata;

fn png() -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend(kind);
        chunk.extend(data);
        chunk.extend([0; 4]);
        chunk
    };
    out.extend(chunk(b"IHDR", &[0, 0, 0, 32, 0, 0, 0, 32, 8, 6, 0, 0, 0]));
    out.extend(chunk(b"IDAT", &[0; 4]));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn wasm_package(fixture: &Package, presentation: Value) -> v2::VerifiedPackageV2<'static> {
    let mut manifest = json!({
        "schema":"chariox.app.v2", "appId":"com.example.catalog", "version":"1.0.0",
        "publisher": fixture.manifest.publisher, "appContractVersion":2,
        "resourcePolicy":"chariox.app.resources.v1", "world":chariox_app_wit::APP_WORLD,
        "requiredFeatures":["wasm-component"],
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"test","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"},"capabilities":{}
    });
    manifest
        .as_object_mut()
        .unwrap()
        .extend(presentation.as_object().unwrap().clone());
    let manifest: v2::ManifestV2 = serde_json::from_value(manifest).unwrap();
    let files = BTreeMap::from([
        ("runtime/app.wasm".into(), super::ui_actions::component()),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>x</title>".to_vec(),
        ),
        ("ui/icon.png".into(), png()),
    ]);
    let bytes = v2::pack(&manifest, &files, &fixture.key, &[], &Limits::default()).unwrap();
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let policy = v2::PolicyV2::new(
        [v2::RequiredFeature::WasmComponent].into(),
        vec![v2::SupportedWorld {
            name: "chariox:app/app".into(),
            major: 1,
            minor: 0,
        }],
        vec![fixture.publisher()],
    );
    v2::verify(bytes, &policy).unwrap()
}

fn candidate_release(package: &impl VerifiedApp, fixture: &Package) -> ReleaseMetadata {
    let (_connection, trust) = database(fixture);
    VerifiedInstallCandidate::from_verified(package, &trust)
        .unwrap()
        .release_metadata()
        .clone()
}

#[test]
fn the_release_carries_the_signed_display_name_and_icon() {
    let fixture = Package::new();
    let verified = wasm_package(
        &fixture,
        json!({"displayName": "Todo list", "icon": "ui/icon.png"}),
    );
    let release = candidate_release(&verified, &fixture);
    assert_eq!(release.display_name.as_deref(), Some("Todo list"));
    let url = release.icon.unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");
    assert_eq!(Some(url), verified.icon_data_url());
    let shared = VerifiedAppPackage::Wasm(verified);
    assert_eq!(
        candidate_release(&shared, &fixture).display_name.as_deref(),
        Some("Todo list")
    );
}

#[test]
fn a_release_without_presentation_has_neither_field() {
    let fixture = Package::new();
    let verified = wasm_package(&fixture, json!({}));
    let release = candidate_release(&verified, &fixture);
    assert!(release.display_name.is_none() && release.icon.is_none());
    let bytes = fixture.bytes();
    let node = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![fixture.publisher()]),
    )
    .unwrap();
    let release = candidate_release(&node, &fixture);
    assert!(release.display_name.is_none() && release.icon.is_none());
}

#[test]
fn a_staged_release_keeps_its_presentation_in_the_store() {
    let fixture = Package::new();
    let verified = wasm_package(
        &fixture,
        json!({"displayName": "Todo list", "icon": "ui/icon.png"}),
    );
    let (mut connection, trust) = database(&fixture);
    let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
    let mut registry = InstallationRegistry::new(&mut connection);
    registry
        .create_and_stage_verified("ui", "owner", &candidate, 1)
        .unwrap();
    let journal = registry.journal("ui").unwrap();
    assert_eq!(&journal[0].release, candidate.release_metadata());
    assert_eq!(
        journal[0].release.display_name.as_deref(),
        Some("Todo list")
    );
    assert!(journal[0].release.icon.is_some());
}
