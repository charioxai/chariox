use chariox_app_package::{Limits, TrustedPublisher};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};
use wit_component::{dummy_module, embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::{LiftLowerAbi, ManglingAndAbi, Resolve};

pub(crate) fn archive(signing_byte: u8) -> Vec<u8> {
    archive_as(signing_byte, "com.example.prepared", "1.0.0", "developer-1")
}

pub(crate) fn archive_as(signing_byte: u8, app_id: &str, version: &str, key_id: &str) -> Vec<u8> {
    static COMPONENT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let component = COMPONENT.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/app-wit");
        let mut resolve = Resolve::default();
        resolve.push_dir(root.join("wit")).unwrap();
        let source = fs::read_to_string(root.join("tests/fixtures/v1.wit")).unwrap();
        let package = resolve.push_str("probe.wit", &source).unwrap();
        let world = resolve.select_world(&[package], Some("probe")).unwrap();
        let mut module = dummy_module(
            &resolve,
            world,
            ManglingAndAbi::Legacy(LiftLowerAbi::AsyncCallback),
        );
        embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false)
            .unwrap();
        ComponentEncoder::default()
            .module(&module)
            .unwrap()
            .validate(true)
            .encode()
            .unwrap()
    });
    let manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":app_id, "version":version,
        "publisher":{"id":"com.example","keyId":key_id,"name":"Developer"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1", "world":chariox_app_package::v2::APP_WORLD,
        "requiredFeatures":["wasm-component"], "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"fixture","toolchain":"wit-component 0.262.0","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"}, "capabilities":{}
    })).unwrap();
    chariox_app_package::v2::pack(
        &manifest,
        &BTreeMap::from([
            ("runtime/app.wasm".into(), component.clone()),
            (
                "ui/index.html".into(),
                b"<!doctype html><title>prepared</title>".to_vec(),
            ),
        ]),
        &SigningKey::from_bytes(&[signing_byte; 32]),
        &[],
        &Limits::default(),
    )
    .unwrap()
}

pub(crate) fn package() -> (Vec<u8>, TrustedPublisher) {
    (
        archive(27),
        TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "developer-1".into(),
            public_key: SigningKey::from_bytes(&[27; 32]).verifying_key(),
        },
    )
}
