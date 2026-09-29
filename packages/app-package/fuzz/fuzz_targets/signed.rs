//! A validly signed package whose manifest or one declaration document is
//! arbitrary: signing does not make content trusted, so every manifest,
//! declaration and JSON Schema check must still reject it without panicking.
//! The first byte picks the document; the rest replaces it.
#![no_main]
use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chariox_app_package::{verify, TrustedPublisher, VerificationPolicy};
use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const DOCUMENTS: [&str; 5] = [
    "manifest.json",
    "schemas/tools.json",
    "schemas/events.json",
    "schemas/actions.json",
    "schemas/information-sets.json",
];

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}
fn canonical(value: &Value) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(value).unwrap_or_default()
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn manifest() -> Value {
    json!({
        "schema":"chariox.app.v1", "appId":"com.example.todo", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Local Developer"},
        "sdkVersion":"0.8.0", "appContractVersion":1, "minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1",
        "runtime":{"engine":"node","entry":"runtime/main.js"}, "ui":{"entry":"ui/index.html"},
        "tools":"schemas/tools.json", "events":"schemas/events.json",
        "actions":"schemas/actions.json", "informationSets":"schemas/information-sets.json",
        "capabilities":{"network":[{"origin":"https://api.example.com","methods":["POST"]}]}
    })
}

fn files() -> BTreeMap<String, Vec<u8>> {
    let closed = json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false});
    BTreeMap::from([
        ("runtime/main.js".to_owned(), b"export default {};".to_vec()),
        ("ui/index.html".to_owned(), b"<!doctype html><title>Todo</title>".to_vec()),
        ("schemas/tools.json".to_owned(), serde_json::to_vec(&json!({"tools":[{"name":"create_todo","inputSchema":closed,"action":"create_todo"}]})).unwrap()),
        ("schemas/events.json".to_owned(), serde_json::to_vec(&json!({"events":[{"name":"todo_due","schemaVersion":1,"direction":"outgoing","payloadSchema":closed}]})).unwrap()),
        ("schemas/actions.json".to_owned(), serde_json::to_vec(&json!({"actions":[{
            "name":"create_todo","inputSchema":closed,
            "criticalValidation":{"reason":"Confirm creation","userVerification":true},
            "effectRoutes":[{"origin":"https://api.example.com","method":"POST","path":"/todos","connection":"todo_account"}]
        }]})).unwrap()),
        ("schemas/information-sets.json".to_owned(), serde_json::to_vec(&json!({"informationSets":[{
            "name":"task_result","purpose":"Show the created Todo","sourceScope":"app_task","schemaVersion":1,
            "delivery":"both","fieldsSchema":closed,"validator":"validate_result"
        }]})).unwrap()),
    ])
}

/// Signs exactly what it is given, bypassing the packer's validation, so the
/// verifier alone decides.
fn package(manifest: &Value, files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let inventory = json!({"schema":"chariox.integrity.v1","files":files.iter().map(|(path, data)| {
        json!({"path":path,"size":data.len(),"digest":hash(data)})
    }).collect::<Vec<_>>()});
    let signed = canonical(
        &json!({"schema":"chariox.package-signature.v1","manifest":manifest,"inventory":inventory}),
    );
    let signature = json!({"key_id":"developer-1","algorithm":"ed25519","digest":hash(&signed),
        "value":BASE64.encode(key().sign(&signed).to_bytes())});
    let mut entries = vec![
        ("manifest.json".to_owned(), canonical(manifest)),
        ("integrity.json".to_owned(), canonical(&inventory)),
        ("signatures/publisher.sig".to_owned(), canonical(&signature)),
    ];
    entries.extend(files.clone());
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in &entries {
        let mut header = tar::Header::new_ustar();
        if header.set_path(path).is_err() {
            return Vec::new();
        }
        // The verifier accepts only its canonical header: without explicit
        // octal uid/gid/mtime fields every package is refused as
        // noncanonical before any JSON is read.
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(data.len() as u64);
        header.set_cksum();
        if builder.append(&header, data.as_slice()).is_err() {
            return Vec::new();
        }
    }
    builder.into_inner().unwrap_or_default()
}

fn policy() -> VerificationPolicy {
    VerificationPolicy::new(
        500,
        vec![TrustedPublisher {
            publisher_id: "com.example".to_owned(),
            key_id: "developer-1".to_owned(),
            public_key: key().verifying_key(),
        }],
    )
}

static BASELINE: std::sync::Once = std::sync::Once::new();

fuzz_target!(|input: &[u8]| {
    // A harness whose unmodified package does not verify would only fuzz the
    // rejection path; fail loudly instead.
    BASELINE.call_once(|| {
        if let Err(error) = verify(&package(&manifest(), &files()), &policy()) {
            panic!("baseline package must verify: {error:?}");
        }
    });
    let Some((&selector, document)) = input.split_first() else {
        return;
    };
    let mut manifest = manifest();
    let mut files = files();
    match DOCUMENTS[selector as usize % DOCUMENTS.len()] {
        "manifest.json" => match serde_json::from_slice(document) {
            Ok(value) => manifest = value,
            Err(_) => return,
        },
        path => {
            files.insert(path.to_owned(), document.to_vec());
        }
    }
    let _ = verify(&package(&manifest, &files), &policy());
});
