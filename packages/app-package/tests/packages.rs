use std::{collections::BTreeMap, io::Read};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chariox_app_package::{
    inspect_untrusted, pack, verify, ErrorCode, Limits, Manifest, TrustedPublisher,
    VerificationPolicy,
};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
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

fn manifest_value() -> Value {
    json!({
        "schema":"chariox.app.v1", "appId":"com.example.todo", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Local Developer"},
        "sdkVersion":"0.1.0", "appContractVersion":1, "minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1",
        "runtime":{"engine":"node","entry":"runtime/main.js"}, "ui":{"entry":"ui/index.html"},
        "tools":"schemas/tools.json", "events":"schemas/events.json",
        "actions":"schemas/actions.json", "informationSets":"schemas/information-sets.json",
        "capabilities":{"network":[{"origin":"https://api.example.com","methods":["POST"]}]},
        "migrations":{"directory":"migrations","targetVersion":1,"steps":[{"from":0,"to":1,"entry":"migrations/001.js"}]}
    })
}

fn manifest() -> Manifest {
    serde_json::from_value(manifest_value()).unwrap()
}
fn closed_schema() -> Value {
    json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false})
}

fn files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("runtime/main.js".to_owned(), b"export default {};".to_vec()),
        ("ui/index.html".to_owned(), b"<!doctype html><title>Todo</title>".to_vec()),
        ("migrations/001.js".to_owned(), b"export function migrate() {}".to_vec()),
        ("schemas/tools.json".to_owned(), serde_json::to_vec_pretty(&json!({"tools":[{"name":"create_todo","inputSchema":closed_schema(),"action":"create_todo"}]})).unwrap()),
        ("schemas/events.json".to_owned(), serde_json::to_vec(&json!({"events":[{"name":"todo_due","payloadSchema":closed_schema()}]})).unwrap()),
        ("schemas/actions.json".to_owned(), serde_json::to_vec(&json!({"actions":[{
            "name":"create_todo","inputSchema":closed_schema(),
            "criticalValidation":{"reason":"Confirm creation","userVerification":true},
            "effectRoutes":[{"origin":"https://api.example.com","method":"POST","path":"/todos","connection":"todo_account"}]
        }]})).unwrap()),
        ("schemas/information-sets.json".to_owned(), serde_json::to_vec(&json!({"informationSets":[{
            "name":"task_result","purpose":"Show the created Todo","sourceScope":"app_task","schemaVersion":1,
            "delivery":"both","fieldsSchema":closed_schema(),"validator":"validate_result"
        }]})).unwrap()),
    ])
}

fn canonical(value: &Value) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(value).unwrap()
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Independent fixture writer deliberately bypasses production validation so
/// negative cases are genuinely signed hostile packages, not packer rejections.
fn signed_entries(manifest: Value, files: BTreeMap<String, Vec<u8>>) -> Vec<(String, Vec<u8>)> {
    let inventory = json!({"schema":"chariox.integrity.v1","files":files.iter().map(|(path,data)| {
        json!({"path":path,"size":data.len(),"digest":hash(data)})
    }).collect::<Vec<_>>()});
    let signed = canonical(
        &json!({"schema":"chariox.package-signature.v1","manifest":manifest,"inventory":inventory}),
    );
    let signature = json!({"key_id":"developer-1","algorithm":"ed25519","digest":hash(&signed),"value":BASE64.encode(key().sign(&signed).to_bytes())});
    let mut entries = vec![
        ("manifest.json".to_owned(), canonical(&manifest)),
        ("integrity.json".to_owned(), canonical(&inventory)),
        ("signatures/publisher.sig".to_owned(), canonical(&signature)),
    ];
    entries.extend(files);
    entries
}

fn archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in entries {
        let mut header = tar::Header::new_ustar();
        header.set_path(path).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, data.as_slice()).unwrap();
    }
    builder.into_inner().unwrap()
}

fn raw_package(value: Value, payload: BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    archive(&signed_entries(value, payload))
}
fn valid_package() -> Vec<u8> {
    pack(&manifest(), &files(), &key(), &Limits::default()).unwrap()
}

fn assert_code(bytes: &[u8], expected: ErrorCode) {
    assert_eq!(verify(bytes, &policy()).unwrap_err().code, expected);
}

#[test]
fn reproducible_signed_package_round_trip_and_independent_fixture() {
    let first = valid_package();
    assert_eq!(first, valid_package());
    let verified = verify(&first, &policy()).unwrap();
    assert_eq!(verified.manifest(), &manifest());
    assert_eq!(
        verified.file("runtime/main.js"),
        Some(b"export default {};".as_slice())
    );
    assert_eq!(verified.files().count(), files().len());
    assert_eq!(verified.package_digest(), hash(&first));
    assert_eq!(verified.declarations().tools[0].name, "create_todo");
    assert_eq!(
        verified.declarations().information_sets[0]
            .validator
            .as_deref(),
        Some("validate_result")
    );
    assert!(verified.file("signatures/publisher.sig").is_none());
    verify(&raw_package(manifest_value(), files()), &policy()).unwrap();
    // Standard tooling can list/read the archive without a custom decompressor.
    let mut parsed = tar::Archive::new(first.as_slice());
    let mut standard_files = BTreeMap::new();
    for entry in parsed.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        standard_files.insert(path, data);
    }
    assert_eq!(standard_files.len(), files().len() + 3);
}

#[test]
fn no_self_enrollment_or_publisher_substitution() {
    let bytes = valid_package();
    let identity = inspect_untrusted(&bytes, &Limits::default()).unwrap();
    assert_eq!(identity.manifest.publisher.id, "com.example");
    let mut untrusted = policy();
    untrusted.trusted_publishers.clear();
    assert_eq!(
        verify(&bytes, &untrusted).unwrap_err().code,
        ErrorCode::UntrustedPublisher
    );
    untrusted.trusted_publishers = policy().trusted_publishers;
    untrusted.trusted_publishers[0].publisher_id = "com.attacker".to_owned();
    assert_eq!(
        verify(&bytes, &untrusted).unwrap_err().code,
        ErrorCode::UntrustedPublisher
    );
    untrusted.trusted_publishers[0].publisher_id = "com.example".to_owned();
    untrusted.trusted_publishers[0].public_key = SigningKey::from_bytes(&[8; 32]).verifying_key();
    assert_eq!(
        verify(&bytes, &untrusted).unwrap_err().code,
        ErrorCode::InvalidSignature
    );
}

#[test]
fn signature_inventory_and_payload_tampering_reject_the_entire_package() {
    let mut entries = signed_entries(manifest_value(), files());
    let runtime = entries
        .iter_mut()
        .find(|(name, _)| name == "runtime/main.js")
        .unwrap();
    runtime.1[0] ^= 1;
    assert_code(&archive(&entries), ErrorCode::IntegrityMismatch);

    let mut entries = signed_entries(manifest_value(), files());
    let mut signature: Value = serde_json::from_slice(&entries[2].1).unwrap();
    signature["value"] = BASE64.encode([0u8; 64]).into();
    entries[2].1 = canonical(&signature);
    assert_code(&archive(&entries), ErrorCode::InvalidSignature);

    let mut entries = signed_entries(manifest_value(), files());
    entries.push(("zz-unexpected.js".to_owned(), vec![]));
    assert_code(&archive(&entries), ErrorCode::UnexpectedEntry);
    entries.pop();
    entries.pop();
    assert_code(&archive(&entries), ErrorCode::UnexpectedEntry);

    let mut entries = signed_entries(manifest_value(), files());
    let mut inventory: Value = serde_json::from_slice(&entries[1].1).unwrap();
    inventory["files"][0]["size"] = 1.into();
    entries[1].1 = canonical(&inventory);
    assert_code(&archive(&entries), ErrorCode::IntegrityMismatch);
}

#[test]
fn compatibility_has_distinct_actionable_codes() {
    for (field, value, expected) in [
        (
            "minKernelProtocol",
            json!(501),
            ErrorCode::IncompatibleProtocol,
        ),
        ("sdkVersion", json!("9.0.0"), ErrorCode::IncompatibleSdk),
        (
            "appContractVersion",
            json!(2),
            ErrorCode::IncompatibleContract,
        ),
        (
            "resourcePolicy",
            json!("future-policy"),
            ErrorCode::IncompatibleResourcePolicy,
        ),
    ] {
        let mut value_manifest = manifest_value();
        value_manifest[field] = value;
        assert_code(&raw_package(value_manifest, files()), expected);
    }
}

#[test]
fn closed_manifest_and_migration_contracts_are_enforced() {
    let mut value = manifest_value();
    value["executeInstallScript"] = true.into();
    assert_code(&raw_package(value, files()), ErrorCode::InvalidManifest);
    let mut value = manifest_value();
    value["runtime"]["engine"] = "deno".into();
    assert_code(&raw_package(value, files()), ErrorCode::InvalidManifest);
    let mut value = manifest_value();
    value["migrations"]["steps"][0]["from"] = 1.into();
    assert_code(&raw_package(value, files()), ErrorCode::InvalidManifest);
    let mut value = manifest_value();
    value["migrations"]["targetVersion"] = 3.into();
    assert_code(&raw_package(value, files()), ErrorCode::InvalidManifest);
    let mut payload = files();
    payload.remove("runtime/main.js");
    assert_code(
        &raw_package(manifest_value(), payload),
        ErrorCode::MissingEntry,
    );
    let mut payload = files();
    payload.insert("migrations/hidden.js".to_owned(), vec![]);
    assert_code(
        &raw_package(manifest_value(), payload),
        ErrorCode::UnexpectedEntry,
    );
}

#[test]
fn hostile_schema_corpus_is_rejected_even_with_a_trusted_signature() {
    for tool_document in [
        json!({"tools":[{"name":"one","inputSchema":{"type":"object"}}]}),
        json!({"tools":[{"name":"one","inputSchema":{"type":"object","additionalProperties":false,"properties":{"x":{"type":"not-a-type"}}}}]}),
        json!({"tools":[{"name":"one","inputSchema":{"type":"object","additionalProperties":false,"properties":{"x":{"$ref":"https://attacker.invalid/schema"}}}}]}),
        json!({"tools":[{"name":"one","inputSchema":{"type":"object","additionalProperties":false,"properties":{"x":{"$ref":"#/properties/x"}}}}]}),
        json!({"tools":[{"name":"one","inputSchema":closed_schema()},{"name":"one","inputSchema":closed_schema()}]}),
        json!({"tools":[{"name":"app.evil.other_installation","inputSchema":closed_schema()}]}),
        json!({"tools":[{"name":"one","inputSchema":closed_schema(),"action":"missing_action"}]}),
    ] {
        let mut payload = files();
        payload.insert("schemas/tools.json".to_owned(), canonical(&tool_document));
        assert_code(
            &raw_package(manifest_value(), payload),
            ErrorCode::InvalidSchema,
        );
    }
    let mut payload = files();
    payload.insert(
        "schemas/tools.json".to_owned(),
        br#"{"tools":[],"tools":[]}"#.to_vec(),
    );
    assert_code(
        &raw_package(manifest_value(), payload),
        ErrorCode::InvalidSchema,
    );
}

#[test]
fn protected_actions_and_information_sets_cannot_exceed_their_contract() {
    for (path, pointer, value) in [
        (
            "schemas/actions.json",
            "/actions/0/effectRoutes/0/origin",
            json!("https://other.example.com"),
        ),
        (
            "schemas/actions.json",
            "/actions/0/effectRoutes/0/path",
            json!("/../transfer"),
        ),
        ("schemas/actions.json", "/actions/0/effectRoutes", json!([])),
        (
            "schemas/information-sets.json",
            "/informationSets/0/sourceScope",
            json!("all_conversations"),
        ),
        (
            "schemas/information-sets.json",
            "/informationSets/0/schemaVersion",
            json!(0),
        ),
        (
            "schemas/information-sets.json",
            "/informationSets/0/purpose",
            json!(""),
        ),
    ] {
        let mut payload = files();
        let mut document: Value = serde_json::from_slice(&payload[path]).unwrap();
        *document.pointer_mut(pointer).unwrap() = value;
        payload.insert(path.to_owned(), canonical(&document));
        assert_code(
            &raw_package(manifest_value(), payload),
            ErrorCode::InvalidSchema,
        );
    }
}

#[test]
fn schema_keywords_are_distinct_from_literal_data_and_property_names() {
    let input = json!({
        "type":"object", "additionalProperties":false,
        "properties":{
            "$ref":{"type":"string"},
            "$id":{"type":"string"},
            "literal":{"const":{"$ref":"https://example.invalid/not-a-reference","$id":"http://[invalid"}},
            "choice":{"enum":[{"$id":"http://[invalid"}]},
            "nested":{"type":"object","default":{"$id":"http://[invalid"},"examples":[{"$ref":"literal"}]}
        },
        "default":{"$id":"http://[invalid"},
        "x-domain-annotation":{"$id":"http://[invalid"},
        "examples":[{"$id":"http://[invalid"}]
    });
    let mut payload = files();
    payload.insert(
        "schemas/tools.json".to_owned(),
        canonical(&json!({"tools":[{"name":"inspect_data","inputSchema":input}]})),
    );
    let bytes = raw_package(manifest_value(), payload);
    let verified = verify(&bytes, &policy()).unwrap();
    // Compilation must not strip defaults or domain annotations from the
    // original declaration delivered to the App/kernel contract consumer.
    assert_eq!(verified.declarations().tools[0].input_schema, input);
    let compiled = chariox_app_package::compile_schema(&input, true, &Limits::default()).unwrap();
    assert!(compiled.is_valid(
        &json!({"$ref":"literal","$id":"identifier","choice":{"$id":"http://[invalid"}})
    ));
    assert!(!compiled.is_valid(&json!({"$ref":42})));
    assert!(!compiled.is_valid(&json!({"choice":{"$id":"different"}})));
}

#[test]
fn every_schema_position_is_checked_and_pattern_property_keys_are_bounded() {
    for schema in [
        json!({"type":"object","additionalProperties":false,"patternProperties":{ "a".repeat(257): {"type":"string"} }}),
        json!({"type":"object","additionalProperties":false,"dependencies":{"id":{"$ref":"https://attacker.invalid/schema"}}}),
        json!({"type":"object","additionalProperties":false,"properties":{"items":{"type":"array","items":[{"$ref":"https://attacker.invalid/schema"}]}}}),
        json!({"type":"object","additionalProperties":false,"if":{"$ref":"https://attacker.invalid/schema"}}),
        json!({"type":"object","additionalProperties":false,"propertyNames":{"$id":"https://attacker.invalid/schema"}}),
    ] {
        let mut payload = files();
        payload.insert(
            "schemas/tools.json".to_owned(),
            canonical(&json!({"tools":[{"name":"inspect_data","inputSchema":schema}]})),
        );
        assert_code(
            &raw_package(manifest_value(), payload),
            ErrorCode::InvalidSchema,
        );
    }
}

#[test]
fn paths_cannot_collide_or_escape_on_supported_filesystems() {
    for path in [
        "/absolute",
        "C:/drive",
        "runtime/../escape",
        "runtime/./file",
        "runtime//file",
        "runtime/back\\slash",
        "runtime/CON.txt",
        "runtime/trailing.",
        "runtime/trailing ",
        "runtime/café.js",
        "runtime/evil\0.js",
    ] {
        let mut payload = files();
        payload.insert(path.to_owned(), vec![]);
        assert_eq!(
            pack(&manifest(), &payload, &key(), &Limits::default())
                .unwrap_err()
                .code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
    for path in [
        "Runtime/extra.js",
        "UI/index.html",
        "runtime/main.js/child",
        "runtime",
    ] {
        let mut payload = files();
        payload.insert(path.to_owned(), vec![]);
        assert_eq!(
            pack(&manifest(), &payload, &key(), &Limits::default())
                .unwrap_err()
                .code,
            ErrorCode::DuplicatePath,
            "{path}"
        );
    }
    let mut payload = files();
    payload.insert("runtime/native.node".to_owned(), vec![]);
    assert_code(
        &raw_package(manifest_value(), payload),
        ErrorCode::UnsupportedFeature,
    );
}

fn header_offset(entries: &[(String, Vec<u8>)], index: usize) -> usize {
    entries[..index]
        .iter()
        .map(|(_, data)| 512 + data.len().div_ceil(512) * 512)
        .sum()
}

fn rewrite_header(bytes: &mut [u8], offset: usize, update: impl FnOnce(&mut tar::Header)) {
    let mut header = tar::Header::new_ustar();
    header
        .as_mut_bytes()
        .copy_from_slice(&bytes[offset..offset + 512]);
    update(&mut header);
    header.set_cksum();
    bytes[offset..offset + 512].copy_from_slice(header.as_bytes());
}

#[test]
fn hostile_archive_headers_are_rejected_before_payload_processing() {
    let entries = signed_entries(manifest_value(), files());
    let original = archive(&entries);
    let offset = header_offset(&entries, 3);
    for kind in [
        tar::EntryType::Symlink,
        tar::EntryType::Link,
        tar::EntryType::Char,
        tar::EntryType::Block,
        tar::EntryType::Fifo,
        tar::EntryType::Directory,
        tar::EntryType::XHeader,
        tar::EntryType::GNULongName,
    ] {
        let mut bytes = original.clone();
        rewrite_header(&mut bytes, offset, |header| header.set_entry_type(kind));
        assert_code(&bytes, ErrorCode::InvalidArchive);
    }
    for name in [
        "../escape",
        "/etc/passwd",
        "runtime\\escape",
        "runtime/CON.txt",
    ] {
        let mut bytes = original.clone();
        rewrite_header(&mut bytes, offset, |header| {
            header.as_mut_bytes()[..100].fill(0);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        });
        assert_code(&bytes, ErrorCode::InvalidPath);
    }
    let mut bytes = original.clone();
    rewrite_header(&mut bytes, offset, |header| header.set_mode(0o755));
    assert_code(&bytes, ErrorCode::InvalidArchive);
    let mut bytes = original.clone();
    rewrite_header(&mut bytes, offset, |header| header.set_uid(1));
    assert_code(&bytes, ErrorCode::InvalidArchive);
    let mut bytes = original.clone();
    bytes[offset + 148] ^= 1;
    assert_code(&bytes, ErrorCode::InvalidArchive);
    let mut duplicate = entries.clone();
    duplicate.insert(4, entries[3].clone());
    assert_code(&archive(&duplicate), ErrorCode::DuplicatePath);
    let mut reordered = entries.clone();
    reordered.swap(3, 4);
    assert_code(&archive(&reordered), ErrorCode::InvalidArchive);
}

#[test]
fn parser_limits_bound_memory_depth_and_counts_without_extraction() {
    let bytes = valid_package();
    let mut bounded = policy();
    bounded.limits.max_archive_bytes = bytes.len() - 1;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_entries = 3;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_file_bytes = 1;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_metadata_bytes = 10;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_json_depth = 2;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_path_depth = 1;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    let mut bounded = policy();
    bounded.limits.max_json_nodes = 2;
    assert_eq!(
        verify(&bytes, &bounded).unwrap_err().code,
        ErrorCode::ArchiveLimit
    );
    // A claimed giant file must fail before allocating or slicing its body.
    let mut bytes = bytes.clone();
    rewrite_header(&mut bytes, 0, |header| header.set_size(u64::MAX / 2));
    assert_code(&bytes, ErrorCode::ArchiveLimit);
}

#[test]
fn truncation_padding_compression_and_control_ambiguity_are_rejected() {
    let entries = signed_entries(manifest_value(), files());
    let bytes = archive(&entries);
    for length in [
        0,
        1,
        511,
        512,
        bytes.len() - 1,
        bytes.len() - 512,
        bytes.len() - 1024,
    ] {
        assert_code(&bytes[..length], ErrorCode::InvalidArchive);
    }
    let mut padding = bytes.clone();
    padding[512 + entries[0].1.len()] = 1;
    assert_code(&padding, ErrorCode::InvalidArchive);
    let mut trailing = bytes.clone();
    trailing.extend([0; 512]);
    assert_code(&trailing, ErrorCode::InvalidArchive);
    let mut compression = vec![0; 1024];
    compression[..4].copy_from_slice(&[0x1f, 0x8b, 0x08, 0]);
    assert_code(&compression, ErrorCode::InvalidArchive);
    let mut reordered = entries.clone();
    reordered.swap(0, 1);
    assert_code(&archive(&reordered), ErrorCode::InvalidArchive);
    let mut noncanonical = entries.clone();
    noncanonical[0].1.push(b'\n');
    assert_code(&archive(&noncanonical), ErrorCode::InvalidManifest);
    let mut duplicate = entries.clone();
    duplicate[0].1 = duplicate[0].1.strip_suffix(b"}").unwrap().to_vec();
    duplicate[0].1.extend(br#", "schema":"chariox.app.v1"}"#);
    assert_code(&archive(&duplicate), ErrorCode::InvalidManifest);
}

#[test]
fn stable_error_code_wire_spelling() {
    assert_eq!(
        serde_json::to_string(&ErrorCode::InvalidSignature).unwrap(),
        "\"INVALID_SIGNATURE\""
    );
    assert_eq!(
        serde_json::to_string(&ErrorCode::IncompatibleProtocol).unwrap(),
        "\"INCOMPATIBLE_PROTOCOL\""
    );
}
