use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use chariox_app_package::{v2::*, Limits, TrustedPublisher};
use chariox_app_wit::{check_component, ComponentError};
use ed25519_dalek::SigningKey;
use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{LiftLowerAbi, ManglingAndAbi, Resolve};

#[test]
fn forbidden_import_is_a_typed_r1_error_even_through_preparation() {
    let mut resolve = Resolve::default();
    resolve.push_dir("../app-wit/wit").unwrap();
    let package = resolve
        .push_str(
            "misuse.wit",
            r#"
        package chariox:sdk-misuse;
        world misuse {
          import wasi:sockets/tcp@0.2.0;
          export chariox:app/handlers@1.0.0;
        }
    "#,
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("misuse")).unwrap();
    let mut module = dummy_module(
        &resolve,
        world,
        ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback),
    );
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false).unwrap();
    let bytes = ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    assert!(
        matches!(check_component(&bytes, APP_WORLD), Err(ComponentError::ImportNotAllowed { import }) if import.starts_with("wasi:sockets/"))
    );
    let error = chariox_app_sdk_tools::prepare_component(&bytes).unwrap_err();
    assert!(
        matches!(error.downcast_ref::<ComponentError>(), Some(ComponentError::ImportNotAllowed { import }) if import.starts_with("wasi:sockets/"))
    );
}

#[test]
#[ignore = "build both templates with test-components.sh first"]
fn source_templates_are_reproducible_and_pass_signed_package_admission() {
    let directory = std::env::var("CHARIOX_SDK_COMPONENTS").expect("CHARIOX_SDK_COMPONENTS");
    for language in ["js", "rust"] {
        let bytes = fs::read(format!("{directory}/{language}-1.wasm")).unwrap();
        let repeated = fs::read(format!("{directory}/{language}-2.wasm")).unwrap();
        assert!(bytes == repeated, "{language} template is not reproducible");
        check_component(&bytes, APP_WORLD).unwrap();
        let source = format!("../app-sdk-{language}/template");
        let manifest: ManifestV2 =
            serde_json::from_slice(&fs::read(format!("{source}/app.json")).unwrap()).unwrap();
        let files = BTreeMap::from([
            ("runtime/app.wasm".into(), bytes),
            (
                "ui/index.html".into(),
                fs::read(format!("{source}/bundle/ui/index.html")).unwrap(),
            ),
            (
                "schemas/tools.json".into(),
                fs::read(format!("{source}/bundle/schemas/tools.json")).unwrap(),
            ),
        ]);
        let key = SigningKey::from_bytes(&[7; 32]);
        let policy = PolicyV2::new(
            BTreeSet::from([RequiredFeature::WasmComponent]),
            vec![SupportedWorld {
                name: "chariox:app/app".into(),
                major: 1,
                minor: 0,
            }],
            vec![TrustedPublisher {
                publisher_id: "com.example".into(),
                key_id: "developer-1".into(),
                public_key: key.verifying_key(),
            }],
        );
        let package = pack(&manifest, &files, &key, &[], &Limits::default()).unwrap();
        verify(&package, &policy).unwrap();
        assert!(package == pack(&manifest, &files, &key, &[], &Limits::default()).unwrap());
    }
}
