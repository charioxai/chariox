//! Sign a source App with a self-contained client view as its UI entry, for the native-open drill.
//! usage: view-package <core.wasm> <view html> <ui actions json> <output directory> [variant [name=<display name>] [icon=<.svg or .png file>] [version=<x.y.z>]]
//! A variant packs the same App under its own id and file name (view-notes-<variant>.cxapp, view-notes-<variant>-<version>.cxapp
//! when a version is given). `name` and `icon` give it the display name and logo of its signed manifest.
use chariox_app_package::{
    v2::{self, ManifestV2},
    Limits,
};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

mod view_signing;

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (core, view, ui_actions, output) = (
        args.next().expect("core module"),
        args.next().expect("view html"),
        args.next().expect("ui actions json"),
        args.next().expect("output directory"),
    );
    let variant = args.next().map(|name| name.to_string_lossy().into_owned());
    let settings: BTreeMap<String, String> = args
        .map(|setting| setting.to_string_lossy().into_owned())
        .map(|setting| {
            let (name, value) = setting.split_once('=').expect("name=value");
            (name.to_owned(), value.to_owned())
        })
        .collect();
    let version = settings.get("version").map_or("1.0.0", String::as_str);
    let (app_id, file) = match &variant {
        Some(variant) if settings.contains_key("version") => (
            format!("com.example.viewnotes{variant}"),
            format!("view-notes-{variant}-{version}.cxapp"),
        ),
        Some(variant) => (
            format!("com.example.viewnotes{variant}"),
            format!("view-notes-{variant}.cxapp"),
        ),
        None => ("com.example.viewnotes".into(), "view-notes.cxapp".into()),
    };
    let component = view_signing::component(&core);
    let key = view_signing::dev_key();
    let key_id = key.id.clone();
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
    let icon = settings.get("icon").map(PathBuf::from);
    let icon_path = icon.as_ref().map(|path| {
        format!(
            "ui/icon.{}",
            path.extension().and_then(|ext| ext.to_str()).unwrap()
        )
    });
    let mut manifest: serde_json::Value = json!({
        "schema":"chariox.app.v2", "appId":app_id, "version":version,
        "publisher":{"id":"com.example","keyId":key_id,"name":"Native view fixture"},
        "appContractVersion":2, "resourcePolicy":"chariox.app.resources.v1",
        "world":v2::APP_WORLD, "requiredFeatures":["wasm-component","ui-actions"], "idempotentTools":idempotent,
        "runtime":{"engine":"wasm","component":"runtime/app.wasm"},
        "build":{"language":"rust","toolchain":"wit-bindgen 0.63.0","sdkVersion":"1.0.0"},
        "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "uiActions":"schemas/ui-actions.json", "capabilities":{}
    });
    if let Some(name) = settings.get("name") {
        manifest["displayName"] = json!(name);
    }
    if let Some(path) = &icon_path {
        manifest["icon"] = json!(path);
    }
    let manifest: ManifestV2 = serde_json::from_value(manifest).unwrap();
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
    let mut files = BTreeMap::from([
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
    if let (Some(path), Some(source)) = (icon_path, icon) {
        files.insert(path, std::fs::read(source).unwrap());
    }
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join(&file),
        v2::pack(&manifest, &files, &key.key, &[], &Limits::default()).unwrap(),
    )
    .unwrap();
    view_signing::write_publisher(&output, "com.example", "Native view fixture", &key);
}
