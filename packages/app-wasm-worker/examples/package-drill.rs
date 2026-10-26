//! Build the real R3 source App's signed local .cxapp for R4 drills.
#[path = "../tests/support/package.rs"]
mod package;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;

fn main() {
    let core = std::fs::read(
        std::env::var("CHARIOX_WORKER_DRILL_CORE").expect("build the source App first"),
    )
    .unwrap();
    let output = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let (bytes, publisher) = package::package(&core);
    std::fs::write(output.join("source.cxapp"), bytes).unwrap();
    let (bytes, _) = package::package_version(&core, None, "1.1.0", 0);
    std::fs::write(output.join("no-migration-update.cxapp"), bytes).unwrap();
    std::fs::write(
        output.join("node.cxapp"),
        node_package(
            &publisher,
            "com.example.r4node",
            b"export default function register() {}",
        ),
    )
    .unwrap();
    std::fs::write(output.join("legacy-source.cxapp"), node_package(&publisher, "com.example.r4drill", b"import {writeFileSync} from 'node:fs'; export default function register() {writeFileSync('/app/data/legacy-marker', 'legacy-owned-data');}")).unwrap();
    std::fs::write(output.join("publisher.json"), serde_json::to_vec(&json!({
        "schema":"chariox.developer-publisher.v1", "algorithm":"ed25519",
        "publisher":{"id":publisher.publisher_id,"keyId":publisher.key_id,"name":"R4 source App drill"},
        "publicKey":STANDARD.encode(publisher.public_key.as_bytes()),
    })).unwrap()).unwrap();
    if let Ok(path) = std::env::var("CHARIOX_INSTALL_MIGRATION_CORE") {
        let migration = std::fs::read(path).unwrap();
        for (name, version, schema) in [("update", "2.0.0", 1), ("failed-update", "3.0.0", 2)] {
            let (bytes, _) = package::package_version(&core, Some(&migration), version, schema);
            std::fs::write(output.join(format!("{name}.cxapp")), bytes).unwrap();
        }
    }
    println!("Source App package and public publisher enrollment written");
}

// Same public fixture signer, for the real client's rebuild-error drill.
fn node_package(
    publisher: &chariox_app_package::TrustedPublisher,
    app_id: &str,
    backend: &[u8],
) -> Vec<u8> {
    let manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1", "appId":app_id, "version":"1.0.0",
        "publisher":{"id":publisher.publisher_id,"keyId":publisher.key_id,"name":"R4 source App drill"},
        "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION, "appContractVersion":1,
        "minKernelProtocol":1,"resourcePolicy":"chariox.app.resources.v1",
        "runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"capabilities":{}
    })).unwrap();
    chariox_app_package::pack(
        &manifest,
        &std::collections::BTreeMap::from([
            ("runtime/main.js".into(), backend.to_vec()),
            (
                "ui/index.html".into(),
                b"<!doctype html><title>Node rebuild drill</title>".to_vec(),
            ),
        ]),
        &ed25519_dalek::SigningKey::from_bytes(&[27; 32]),
        &chariox_app_package::Limits::default(),
    )
    .unwrap()
}
