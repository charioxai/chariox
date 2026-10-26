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
    let bytes = wit_component::ComponentEncoder::default()
        .module(core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let key = SigningKey::from_bytes(&[27; 32]);
    let key_id = format!("dev-{:x}", Sha256::digest(key.verifying_key().as_bytes()));
    let manifest: ManifestV2 = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":"com.example.r4drill", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":key_id,"name":"R4 source App drill"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1",
        "world":v2::APP_WORLD,"requiredFeatures":["wasm-component"],"idempotentTools":["echo"],
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"wit-bindgen 0.57.1","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"},"tools":"schemas/tools.json","capabilities":{}
    }))
    .unwrap();
    let tools: Vec<_> = ["echo","shared","wait","ignore","trap","spin","oom","stream","slow_stream","subscribe","upload"]
        .map(|name| json!({"name":name,"description":format!("Source App drill: {name}"),"inputSchema":{"type":"object","properties":{"portable":{"type":"string"}},"required":["portable"],"additionalProperties":false}})).into();
    let files = BTreeMap::from([
        ("runtime/app.wasm".into(), bytes),
        ("schemas/tools.json".into(), serde_json::to_vec(&json!({"tools":tools})).unwrap()),
        ("ui/index.html".into(), b"<!doctype html><title>R4 source App</title><p>Use the echo tool to verify installation.</p>".to_vec()),
    ]);
    (
        v2::pack(&manifest, &files, &key, &[], &Limits::default()).unwrap(),
        TrustedPublisher {
            publisher_id: manifest.publisher.id,
            key_id,
            public_key: key.verifying_key(),
        },
    )
}
