//! Sign a source App with a self-contained client view as its UI entry, for the native-open drill.
//! usage: view-package <core.wasm> <view html> <output directory>
//! The key is fixed public test material; the drill enrolls its public half like any publisher.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_app_package::{
    v2::{self, ManifestV2},
    Limits,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (core, view, output) = (
        args.next().expect("core module"),
        args.next().expect("view html"),
        args.next().expect("output directory"),
    );
    let component = wit_component::ComponentEncoder::default()
        .module(&std::fs::read(core).unwrap())
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    let key_id = format!("dev-{:x}", Sha256::digest(key.verifying_key().as_bytes()));
    let manifest: ManifestV2 = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":"com.example.viewnotes", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":key_id,"name":"Native view fixture"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1",
        "world":v2::APP_WORLD, "requiredFeatures":["wasm-component"], "idempotentTools":["list_notes"],
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"wit-bindgen 0.63.0","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "capabilities":{}
    }))
    .unwrap();
    let tools = json!({"tools":[
        {"name":"list_notes","description":"Read the saved notes","inputSchema":{"type":"object","additionalProperties":false}},
        {"name":"add_note","description":"Save one note","inputSchema":{"type":"object","additionalProperties":false,"required":["text"],
            "properties":{"text":{"type":"string","minLength":1,"maxLength":200}}}}
    ]});
    let files = BTreeMap::from([
        ("runtime/app.wasm".to_string(), component),
        (
            "schemas/tools.json".into(),
            serde_json::to_vec(&tools).unwrap(),
        ),
        ("ui/index.html".into(), std::fs::read(view).unwrap()),
    ]);
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join("view-notes.cxapp"),
        v2::pack(&manifest, &files, &key, &[], &Limits::default()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("publisher.json"),
        serde_json::to_vec(&json!({
            "schema":"chariox.developer-publisher.v1", "algorithm":"ed25519",
            "publisher":{"id":"com.example","keyId":key_id,"name":"Native view fixture"},
            "publicKey":STANDARD.encode(key.verifying_key().as_bytes()),
        }))
        .unwrap(),
    )
    .unwrap();
}
