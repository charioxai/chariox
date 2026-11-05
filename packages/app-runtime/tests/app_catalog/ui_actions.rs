use super::*;
use chariox_app_package::{v2, VerifiedApp, VerifiedAppPackage};

fn compile(
    package: &impl VerifiedApp,
    connection: &mut Connection,
    trust: &TrustedPublisherSnapshot,
) -> AppCatalog {
    let candidate = VerifiedInstallCandidate::from_verified(package, trust).unwrap();
    let mut registry = InstallationRegistry::new(connection);
    let token = registry
        .create_and_stage_verified("ui", "owner", &candidate, 1)
        .unwrap()
        .token;
    let binding = registry.staged_trust("owner", &token).unwrap();
    AppCatalog::compile(package, &binding, trust).unwrap()
}

fn component() -> Vec<u8> {
    use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
    use wit_parser::{LiftLowerAbi, ManglingAndAbi, Resolve};
    let mut resolve = Resolve::default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app-wit");
    resolve.push_dir(root.join("wit")).unwrap();
    let source = std::fs::read_to_string(root.join("tests/fixtures/v1.wit")).unwrap();
    let package = resolve.push_str("probe.wit", &source).unwrap();
    let world = resolve.select_world(&[package], Some("probe")).unwrap();
    let mut module = dummy_module(
        &resolve,
        world,
        ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback),
    );
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false).unwrap();
    ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

#[test]
fn wasm_catalog_retains_verified_ui_actions_for_direct_and_shared_proofs() {
    let fixture = Package::new();
    let manifest: v2::ManifestV2 = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":"com.example.catalog", "version":"1.0.0",
        "publisher": fixture.manifest.publisher, "appContractVersion":2,
        "resourcePolicy":"chariox.app.resources.v1", "world":chariox_app_wit::APP_WORLD,
        "requiredFeatures":["wasm-component","ui-actions"],
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"test","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"},"uiActions":"schemas/ui.json","capabilities":{}
    }))
    .unwrap();
    let actions = json!([
        {"name":"inspect","description":"Read the sheet","inputSchema":{"type":"object","additionalProperties":false},"effect":"read"},
        {"name":"clear","description":"Clear the sheet","inputSchema":{"type":"object","additionalProperties":false},"outputSchema":{"type":"boolean"},"effect":"approval","approvalReason":"Clear all cells"}
    ]);
    let files = BTreeMap::from([
        ("runtime/app.wasm".into(), component()),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>Actions</title>".to_vec(),
        ),
        (
            "schemas/ui.json".into(),
            serde_json::to_vec(&json!({"uiActions":actions})).unwrap(),
        ),
    ]);
    let bytes = v2::pack(&manifest, &files, &fixture.key, &[], &Limits::default()).unwrap();
    let policy = v2::PolicyV2::new(
        [
            v2::RequiredFeature::WasmComponent,
            v2::RequiredFeature::UiActions,
        ]
        .into(),
        vec![v2::SupportedWorld {
            name: "chariox:app/app".into(),
            major: 1,
            minor: 0,
        }],
        vec![fixture.publisher()],
    );
    let verified = v2::verify(&bytes, &policy).unwrap();
    let expected: Vec<v2::UiActionDeclaration> = serde_json::from_value(actions).unwrap();
    assert_eq!(verified.ui_actions(), expected);
    let (mut connection, trust) = database(&fixture);
    let catalog = compile(&verified, &mut connection, &trust);
    assert_eq!(catalog.ui_actions(), expected);
    let shared = VerifiedAppPackage::Wasm(verified);
    let (mut connection, trust) = database(&fixture);
    assert_eq!(
        compile(&shared, &mut connection, &trust).ui_actions(),
        expected
    );
}

#[test]
fn non_wasm_catalog_keeps_the_verified_empty_ui_action_set() {
    let fixture = Package::new();
    let bytes = fixture.bytes();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![fixture.publisher()]),
    )
    .unwrap();
    let (mut connection, trust) = database(&fixture);
    assert!(compile(&verified, &mut connection, &trust)
        .ui_actions()
        .is_empty());
    let shared = VerifiedAppPackage::Node(verified);
    let (mut connection, trust) = database(&fixture);
    assert!(compile(&shared, &mut connection, &trust)
        .ui_actions()
        .is_empty());
}
