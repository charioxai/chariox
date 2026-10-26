//! Package the source App for installation and process drills, without reading
//! production signing identities. The fixed key is public test material only.
use chariox_app_package::{
    v2::{self, ManifestV2},
    Limits, TrustedPublisher,
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn package(core: &[u8]) -> (Vec<u8>, TrustedPublisher) {
    package_version(core, None, "1.0.0", 0)
}

pub fn package_version(
    core: &[u8],
    migration: Option<&[u8]>,
    version: &str,
    schema: u32,
) -> (Vec<u8>, TrustedPublisher) {
    let bytes = wit_component::ComponentEncoder::default()
        .module(core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let key = SigningKey::from_bytes(&[27; 32]);
    let key_id = format!("dev-{:x}", Sha256::digest(key.verifying_key().as_bytes()));
    let mut manifest: ManifestV2 = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":"com.example.r4drill", "version":version,
        "publisher":{"id":"com.example","keyId":key_id,"name":"R4 source App drill"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1",
        "world":v2::APP_WORLD,"requiredFeatures":["wasm-component","ui-actions"],"idempotentTools":["echo"],
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"wit-bindgen 0.57.1","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"},"tools":"schemas/tools.json","uiActions":"schemas/ui-actions.json","capabilities":{}
    }))
    .unwrap();
    let tools: Vec<_> = ["echo","shared","wait","ignore","trap","spin","oom","stream","slow_stream","subscribe","upload","early_response","grace"]
        .map(|name| json!({"name":name,"description":format!("Source App drill: {name}"),"inputSchema":{"type":"object","properties":{"portable":{"type":"string"}},"required":["portable"],"additionalProperties":false}})).into();
    let mut files = BTreeMap::from([
        ("runtime/app.wasm".into(), bytes),
        ("schemas/ui-actions.json".into(), serde_json::to_vec(&json!({"uiActions":[{"name":"inspect","description":"Read the App view","inputSchema":{"type":"object","additionalProperties":false},"effect":"read"}]})).unwrap()),
        ("schemas/tools.json".into(), serde_json::to_vec(&json!({"tools":tools})).unwrap()),
        ("ui/index.html".into(), b"<!doctype html><title>R4 source App</title><p>Use the echo tool to verify installation.</p>".to_vec()),
    ]);
    if schema > 0 {
        let migration = wit_component::ComponentEncoder::default()
            .module(migration.unwrap())
            .unwrap()
            .validate(true)
            .encode()
            .unwrap();
        let steps = (0..schema)
            .map(|from| {
                let entry = format!("migrations/{}.wasm", from + 1);
                files.insert(entry.clone(), migration.clone());
                chariox_app_package::Migration {
                    from,
                    to: from + 1,
                    entry,
                }
            })
            .collect();
        manifest.migrations = Some(chariox_app_package::Migrations {
            directory: "migrations".into(),
            target_version: schema,
            steps,
        });
    }
    (
        v2::pack(&manifest, &files, &key, &[], &Limits::default()).unwrap(),
        TrustedPublisher {
            publisher_id: manifest.publisher.id,
            key_id,
            public_key: key.verifying_key(),
        },
    )
}
