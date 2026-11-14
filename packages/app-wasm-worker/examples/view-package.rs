//! Sign a source App with a self-contained client view as its UI entry, for the native-open drill.
//! usage: view-package <core.wasm> <view html> <ui actions json> <output directory> [variant]
//! A variant packs the same App under its own id and file name (view-notes-<variant>.cxapp).
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
    let (core, view, ui_actions, output) = (
        args.next().expect("core module"),
        args.next().expect("view html"),
        args.next().expect("ui actions json"),
        args.next().expect("output directory"),
    );
    let variant = args.next().map(|name| name.to_string_lossy().into_owned());
    let (app_id, file) = match &variant {
        Some(variant) => (
            format!("com.example.viewnotes{variant}"),
            format!("view-notes-{variant}.cxapp"),
        ),
        None => ("com.example.viewnotes".into(), "view-notes.cxapp".into()),
    };
    let component = wit_component::ComponentEncoder::default()
        .module(&std::fs::read(core).unwrap())
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
    let key_id = format!("dev-{:x}", Sha256::digest(key.verifying_key().as_bytes()));
    let declared: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ui_actions).unwrap()).unwrap();
    let declared = declared["uiActions"].as_array().expect("uiActions list");
    // A declared action is the backend tool of the same name; the SDK's view tools come on top.
    let mut idempotent: Vec<&str> = declared
        .iter()
        .filter(|action| action["effect"] == "read")
        .map(|action| action["name"].as_str().unwrap())
        .chain(["view_actions", "view_state", "view_changes"])
        .collect();
    idempotent.sort_unstable();
    let manifest: ManifestV2 = serde_json::from_value(json!({
        "schema":"chariox.app.v2", "appId":app_id, "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":key_id,"name":"Native view fixture"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1",
        "world":v2::APP_WORLD, "requiredFeatures":["wasm-component","ui-actions"], "idempotentTools":idempotent,
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"wit-bindgen 0.63.0","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "uiActions":"schemas/ui-actions.json", "capabilities":{}
    }))
    .unwrap();
    let mut tools: Vec<serde_json::Value> = declared
        .iter()
        .map(|action| json!({"name":action["name"],"description":action["description"],"inputSchema":action["inputSchema"]}))
        .collect();
    tools.extend([
        json!({"name":"view_actions","description":"The actions this backend registers for its view","inputSchema":{"type":"object","additionalProperties":false}}),
        json!({"name":"view_state","description":"The state of this backend's view","inputSchema":{"type":"object","additionalProperties":false}}),
        json!({"name":"view_changes","description":"The changes this backend's view recorded after `since`","inputSchema":{"type":"object","additionalProperties":false,"properties":{"since":{"type":"integer","minimum":0}}}}),
    ]);
    let tools = json!({ "tools": tools });
    let files = BTreeMap::from([
        ("runtime/app.wasm".to_string(), component),
        (
            "schemas/tools.json".into(),
            serde_json::to_vec(&tools).unwrap(),
        ),
        (
            "schemas/ui-actions.json".into(),
            std::fs::read(&ui_actions).unwrap(),
        ),
        ("ui/index.html".into(), std::fs::read(view).unwrap()),
    ]);
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join(&file),
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
