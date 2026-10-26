use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

mod components;

use super::*;
use crate::{archive, read_inventory, signing_bytes_for, ErrorCode, Limits, TrustedPublisher};

/// Vectors are shared with other implementations. Regenerate one only for a
/// deliberate contract change: `CHARIOX_UPDATE_VECTORS=1 cargo test ...`.
fn check_vectors(file: &str, run: impl Fn(&Value, &Value) -> Value) {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/vectors/v2", file]
        .iter()
        .collect();
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let shared = document.get("shared").cloned().unwrap_or(Value::Null);
    let update = std::env::var_os("CHARIOX_UPDATE_VECTORS").is_some();
    for case in document["cases"].as_array_mut().unwrap() {
        let actual = run(&shared, &case["input"]);
        if update {
            case["expected"] = actual;
        } else {
            assert!(
                case["expected"] == actual,
                "{file} / {}: encoding changed\nexpected {:#}\nactual   {actual:#}",
                case["name"],
                case["expected"]
            );
        }
    }
    if update {
        fs::write(
            &path,
            serde_json::to_string_pretty(&document).unwrap() + "\n",
        )
        .unwrap();
    }
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn policy() -> PolicyV2 {
    PolicyV2::new(
        BTreeSet::from([
            RequiredFeature::WasmComponent,
            RequiredFeature::UiActions,
            RequiredFeature::AgentPanel,
            RequiredFeature::Connections,
            RequiredFeature::DirectedEvents,
        ]),
        vec![
            SupportedWorld {
                name: "chariox:app/app".to_owned(),
                major: 1,
                minor: 0,
            },
            SupportedWorld {
                name: "chariox:app/migration".to_owned(),
                major: 1,
                minor: 0,
            },
        ],
        vec![TrustedPublisher {
            publisher_id: "com.example".to_owned(),
            key_id: "developer-1".to_owned(),
            public_key: key().verifying_key(),
        }],
    )
}

struct Fixture {
    manifest: ManifestV2,
    files: BTreeMap<String, Vec<u8>>,
    reviews: BTreeMap<String, ReviewEntry>,
}

fn fixture(shared: &Value) -> Fixture {
    Fixture {
        manifest: serde_json::from_value(shared["manifest"].clone()).unwrap(),
        files: shared["files"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(path, text)| {
                let text = text.as_str().unwrap();
                let bytes = match text.strip_prefix("wit:") {
                    Some(world) => components::build(world, false),
                    None => text.as_bytes().to_vec(),
                };
                (path.clone(), bytes)
            })
            .collect(),
        reviews: serde_json::from_value(shared["reviews"].clone()).unwrap(),
    }
}

fn base() -> Fixture {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/vectors/v2/package.json"]
        .iter()
        .collect();
    let document: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    fixture(&document["shared"])
}

fn packed(fixture: &Fixture, reviews: &[&str]) -> Vec<u8> {
    let reviews: Vec<_> = reviews
        .iter()
        .map(|k| fixture.reviews[*k].clone())
        .collect();
    pack(
        &fixture.manifest,
        &fixture.files,
        &key(),
        &reviews,
        &Limits::default(),
    )
    .unwrap()
}

fn package_vector(shared: &Value, input: &Value) -> Value {
    let mut fixture = fixture(shared);
    if let Some(ui) = input["ui"].as_str() {
        fixture
            .files
            .insert("ui/index.html".to_owned(), ui.as_bytes().to_vec());
    }
    let reviews: Vec<_> = input["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    let bytes = packed(&fixture, &reviews);
    let policy = policy();
    let verified = verify(&bytes, &policy).unwrap();
    let entries = archive::parse(&bytes, &policy.limits).unwrap();
    let text = |index: usize| String::from_utf8(entries[index].data.to_vec()).unwrap();
    let manifest_value: Value = serde_json::from_str(&text(0)).unwrap();
    let inventory = read_inventory(entries[1].data, INTEGRITY_SCHEMA_V2, &policy.limits).unwrap();
    let subject = signing_bytes_for(SIGNATURE_SCHEMA_V2, &manifest_value, &inventory).unwrap();
    let signature: Value = serde_json::from_str(&text(2)).unwrap();
    let mut vector = json!({
        "manifestBytes": text(0),
        "inventoryBytes": text(1),
        "signingSubject": String::from_utf8(subject).unwrap(),
        "signatureValue": signature["value"],
        "contentDigest": verified.digests().content,
        "publisherArchiveDigest": verified.digests().publisher_archive,
        "deliveredDigest": verified.digests().delivered,
        "reviewKeyIds": verified.reviews().iter().map(|r| &r.key_id).collect::<Vec<_>>(),
    });
    if input["digestsOnly"] == true {
        for bulky in [
            "manifestBytes",
            "inventoryBytes",
            "signingSubject",
            "signatureValue",
        ] {
            vector.as_object_mut().unwrap().remove(bulky);
        }
    }
    vector
}

#[test]
fn package_vectors_are_stable() {
    check_vectors("package.json", package_vector);
}

#[test]
fn review_vectors_are_stable_and_bind_nothing_into_the_publisher_digests() {
    check_vectors("package-reviews.json", package_vector);
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/vectors/v2/package-reviews.json",
    ]
    .iter()
    .collect();
    let document: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let digests: BTreeMap<&str, &Value> = document["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (case["name"].as_str().unwrap(), &case["expected"]))
        .collect();
    let field = |case: &str, name: &str| digests[case][name].clone();
    for case in ["a", "a-b", "b-a-reordered", "b-removed-a"] {
        for name in ["contentDigest", "publisherArchiveDigest"] {
            assert!(
                field(case, name) == field("none", name),
                "{case} changed {name}"
            );
        }
    }
    assert!(field("none", "deliveredDigest") != field("a", "deliveredDigest"));
    assert!(field("a", "deliveredDigest") != field("a-b", "deliveredDigest"));
    assert!(
        field("a-b", "deliveredDigest") == field("b-a-reordered", "deliveredDigest"),
        "review order must not change the archive"
    );
    assert!(field("a", "deliveredDigest") == field("b-removed-a", "deliveredDigest"));
    assert!(field("a-b", "reviewKeyIds") == json!(["reviewer-a", "reviewer-b"]));
    // A changed UI invalidates what an attestation of the old content bound.
    assert!(field("changed-ui-with-a", "contentDigest") != field("a", "contentDigest"));
    assert!(
        field("changed-ui-with-a", "publisherArchiveDigest")
            != field("a", "publisherArchiveDigest")
    );
}

#[test]
fn app_id_vectors_are_stable() {
    check_vectors("app-id.json", |_, input| {
        match AppId::parse(input["appId"].as_str().unwrap()) {
            Ok(id) => json!({"valid": true, "namespaces": id.namespaces()}),
            Err(_) => json!({"valid": false}),
        }
    });
}

#[test]
fn registration_vectors_are_stable() {
    check_vectors("registration.json", |_, input| {
        let signer = SignerPrincipal::of(&crate::Publisher {
            id: input["signer"]["publisherId"].as_str().unwrap().to_owned(),
            key_id: input["signer"]["keyId"].as_str().unwrap().to_owned(),
            name: "Name".to_owned(),
        });
        match registration_outcome(input["bound"].as_str(), &signer) {
            Ok(outcome) => serde_json::to_value(outcome).unwrap(),
            Err(refusal) => serde_json::to_value(refusal).unwrap(),
        }
    });
}

#[test]
fn world_vectors_are_stable() {
    check_vectors("world.json", |_, input| {
        let Ok(requested) = WorldId::parse(input["requested"].as_str().unwrap()) else {
            return json!({"invalid": true});
        };
        let supported: Vec<SupportedWorld> =
            serde_json::from_value(input["supported"].clone()).unwrap();
        match check_world(&requested, &supported) {
            Ok(()) => json!({"supported": true}),
            Err(refusal) => serde_json::to_value(refusal).unwrap(),
        }
    });
}

fn code(result: crate::Result<impl std::fmt::Debug>) -> ErrorCode {
    result.unwrap_err().code
}

/// Rebuild an archive with one more entry, keeping the sorted-tail rule.
fn with_entry(bytes: &[u8], path: &str, data: &[u8]) -> Vec<u8> {
    let limits = Limits::default();
    let entries = archive::parse(bytes, &limits).unwrap();
    let mut all: Vec<(&str, &[u8])> = entries.iter().map(|e| (e.path.as_str(), e.data)).collect();
    all.push((path, data));
    all[3..].sort_by_key(|entry| entry.0);
    archive::encode(&all, &limits).unwrap()
}

#[test]
fn unknown_signature_formats_are_refused_not_trusted() {
    let bytes = packed(&base(), &[]);
    let policy = policy();
    for (path, data) in [
        ("signatures/other.sig", b"{}".as_slice()),
        ("signatures/reviews/reviewer-a.txt", b"{}"),
        ("signatures/reviews/Bad Id.json", b"{}"),
    ] {
        let tampered = with_entry(&bytes, path, data);
        assert_eq!(
            code(verify(&tampered, &policy)),
            ErrorCode::IncompatibleContract,
            "{path}"
        );
    }
    let future = json!({"schema": "chariox.package-review.v2", "keyId": "reviewer-a"});
    let tampered = with_entry(
        &bytes,
        "signatures/reviews/reviewer-a.json",
        &serde_json::to_vec(&future).unwrap(),
    );
    assert_eq!(
        code(verify(&tampered, &policy)),
        ErrorCode::IncompatibleContract
    );
}

#[test]
fn malformed_review_entries_are_refused() {
    let fixture = base();
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let mut wrong_key = fixture.reviews["reviewer-a"].clone();
    wrong_key.key_id = "reviewer-b".to_owned();
    let mut not_canonical = serde_json::to_vec_pretty(&fixture.reviews["reviewer-a"]).unwrap();
    not_canonical.push(b'\n');
    for data in [serde_json::to_vec(&wrong_key).unwrap(), not_canonical] {
        let tampered = with_entry(&bytes, "signatures/reviews/reviewer-a.json", &data);
        assert_eq!(
            code(verify(&tampered, &policy)),
            ErrorCode::InvalidSignature
        );
    }
    let many: Vec<_> = (0..=MAX_REVIEWS).map(|i| format!("reviewer-{i}")).collect();
    let mut reviews = Vec::new();
    for name in &many {
        let mut review = fixture.reviews["reviewer-a"].clone();
        review.key_id = name.clone();
        reviews.push(review);
    }
    let limits = Limits::default();
    assert_eq!(
        code(pack(
            &fixture.manifest,
            &fixture.files,
            &key(),
            &reviews,
            &limits
        )),
        ErrorCode::ArchiveLimit
    );
}

fn check_review_json_limit(attestation: Value, limits: Limits, expected: Option<ErrorCode>) {
    let fixture = base();
    let mut review = fixture.reviews["reviewer-a"].clone();
    review.attestation = attestation;
    let mut policy = policy();
    policy.limits = limits;
    let review_bytes = crate::json::canonical(&review).unwrap();
    assert!(review_bytes.len() <= MAX_REVIEW_BYTES);
    let delivered = with_entry(
        &packed(&fixture, &[]),
        "signatures/reviews/reviewer-a.json",
        &review_bytes,
    );
    let verified = verify(&delivered, &policy);
    let packed = pack(
        &fixture.manifest,
        &fixture.files,
        &key(),
        &[review],
        &policy.limits,
    );
    match expected {
        Some(expected) => {
            assert_eq!(code(verified), expected);
            assert_eq!(packed.err().map(|error| error.code), Some(expected));
        }
        None => {
            verified.unwrap();
            verify(&packed.unwrap(), &policy).unwrap();
        }
    }
}

#[test]
fn review_json_depth_limits_include_the_envelope() {
    // The review root is depth 0; the attestation starts at depth 1.
    let nested = |count| (0..count).fold(json!({}), |value, _| json!({"nested": value}));
    check_review_json_limit(nested(31), Limits::default(), None);
    check_review_json_limit(nested(40), Limits::default(), Some(ErrorCode::ArchiveLimit));
    check_review_json_limit(nested(32), Limits::default(), Some(ErrorCode::ArchiveLimit));
}

#[test]
fn review_json_node_limits_include_the_envelope() {
    let limits = Limits {
        max_json_nodes: 128,
        ..Limits::default()
    };
    // Root, four strings, attestation object and array account for seven nodes.
    check_review_json_limit(json!({"items": vec![0; 121]}), limits.clone(), None);
    check_review_json_limit(
        json!({"items": vec![0; 122]}),
        limits,
        Some(ErrorCode::ArchiveLimit),
    );
}

#[test]
fn review_json_numbers_must_be_interoperable() {
    for number in [9_007_199_254_740_991i64, -9_007_199_254_740_991] {
        check_review_json_limit(json!({"number": number}), Limits::default(), None);
    }
    for number in [9_007_199_254_740_992i64, -9_007_199_254_740_992] {
        check_review_json_limit(
            json!({"number": number}),
            Limits::default(),
            Some(ErrorCode::InvalidSchema),
        );
    }
}

#[test]
fn a_review_never_stands_in_for_the_publisher_signature() {
    let policy = policy();
    let mut untrusted = policy.clone();
    untrusted.trusted_publishers.clear();
    let bytes = packed(&base(), &["reviewer-a"]);
    assert_eq!(
        code(verify(&bytes, &untrusted)),
        ErrorCode::UntrustedPublisher
    );
    verify(&bytes, &policy).unwrap();
}

#[test]
fn compatibility_is_checked_against_world_and_features() {
    let bytes = packed(&base(), &[]);
    let mut no_world = policy();
    no_world.worlds.clear();
    assert_eq!(
        code(verify(&bytes, &no_world)),
        ErrorCode::IncompatibleContract
    );
    let mut no_actions = policy();
    no_actions
        .kernel_features
        .remove(&RequiredFeature::UiActions);
    let error = verify(&bytes, &no_actions).unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleProtocol);
    assert!(error.message.contains("ui-actions"), "{}", error.message);
}

#[test]
fn a_package_cannot_understate_the_features_it_uses() {
    let mut fixture = base();
    fixture.manifest.required_features = vec![RequiredFeature::WasmComponent];
    let limits = Limits::default();
    assert_eq!(
        code(pack(
            &fixture.manifest,
            &fixture.files,
            &key(),
            &[],
            &limits
        )),
        ErrorCode::InvalidManifest
    );
}

#[test]
fn v1_and_v2_do_not_accept_each_others_packages() {
    let bytes = packed(&base(), &[]);
    let v1 = crate::VerificationPolicy::new(500, Vec::new());
    assert_eq!(code(crate::verify(&bytes, &v1)), ErrorCode::InvalidManifest);
    let mut forged = base();
    forged.manifest.schema = "chariox.app.v1".to_owned();
    assert_eq!(
        code(pack(
            &forged.manifest,
            &forged.files,
            &key(),
            &[],
            &Limits::default()
        )),
        ErrorCode::InvalidManifest
    );
}

#[test]
fn tampering_and_serialized_native_code_are_refused() {
    let bytes = packed(&base(), &[]);
    let policy = policy();
    let mut tampered = bytes.clone();
    let position = tampered
        .windows(b"<title>Todo".len())
        .position(|window| window == b"<title>Todo")
        .unwrap();
    tampered[position + 7] = b'X';
    assert_eq!(
        code(verify(&tampered, &policy)),
        ErrorCode::IntegrityMismatch
    );
    let mut fixture = base();
    fixture
        .files
        .insert("runtime/app.cwasm".to_owned(), b"native".to_vec());
    assert_eq!(
        code(pack(
            &fixture.manifest,
            &fixture.files,
            &key(),
            &[],
            &Limits::default()
        )),
        ErrorCode::UnsupportedFeature
    );
}

#[test]
fn ui_action_declarations_are_validated() {
    let mut fixture = base();
    let closed = json!({"type":"object","additionalProperties":false});
    let action = |name: &str, effect: &str, reason: Option<&str>| {
        let mut value =
            json!({"name": name, "description": "d", "inputSchema": closed, "effect": effect});
        if let Some(reason) = reason {
            value["approvalReason"] = json!(reason);
        }
        value
    };
    let cases = [
        vec![action("clear", "approval", None)],
        vec![action("read_it", "read", Some("why"))],
        vec![action("same", "read", None), action("same", "write", None)],
        vec![action("Bad-Name", "read", None)],
    ];
    for actions in cases {
        fixture.files.insert(
            "schemas/ui-actions.json".to_owned(),
            serde_json::to_vec(&json!({"uiActions": actions})).unwrap(),
        );
        assert_eq!(
            code(pack(
                &fixture.manifest,
                &fixture.files,
                &key(),
                &[],
                &Limits::default()
            )),
            ErrorCode::InvalidSchema
        );
    }
    let verified_bytes = packed(&base(), &[]);
    let policy = policy();
    let verified = verify(&verified_bytes, &policy).unwrap();
    let effects: Vec<_> = verified.ui_actions().iter().map(|a| a.effect).collect();
    assert_eq!(
        effects,
        [
            UiActionEffect::Write,
            UiActionEffect::Read,
            UiActionEffect::Approval
        ]
    );
}

#[test]
fn the_dev_identity_is_a_distinct_principal() {
    let mut fixture = base();
    fixture.manifest.publisher.id = DEV_PUBLISHER_ID.to_owned();
    let bytes = pack(
        &fixture.manifest,
        &fixture.files,
        &key(),
        &[],
        &Limits::default(),
    )
    .unwrap();
    let mut policy = policy();
    policy.trusted_publishers[0].publisher_id = DEV_PUBLISHER_ID.to_owned();
    let verified = verify(&bytes, &policy).unwrap();
    assert_eq!(
        SignerPrincipal::of(&verified.manifest().publisher),
        SignerPrincipal::Dev
    );
}

#[test]
fn signed_unknown_component_import_is_refused_before_install() {
    let mut fixture = base();
    fixture.files.insert(
        fixture.manifest.runtime.component.clone(),
        components::build(APP_WORLD, true),
    );
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let Err(error) = verify(&bytes, &policy) else {
        panic!("signed forbidden import was accepted");
    };
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
    assert!(error.message.contains("forbidden"), "{}", error.message);
    assert!(
        error.message.contains("runtime/app.wasm"),
        "{}",
        error.message
    );
}

#[test]
fn signed_migration_world_cannot_be_used_as_the_app_backend() {
    let mut fixture = base();
    fixture.manifest.world = MIGRATION_WORLD.into();
    fixture.files.insert(
        fixture.manifest.runtime.component.clone(),
        components::build(MIGRATION_WORLD, false),
    );
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let Err(error) = verify(&bytes, &policy) else {
        panic!("signed migration-only App backend was accepted");
    };
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
    for expected in ["runtime/app.wasm", APP_WORLD, MIGRATION_WORLD] {
        assert!(error.message.contains(expected), "{}", error.message);
    }
}

#[test]
fn signed_migrations_cannot_import_the_app_http_surface() {
    let mut fixture = base();
    fixture.files.insert(
        "migrations/001.wasm".into(),
        components::build(APP_WORLD, false),
    );
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let Err(error) = verify(&bytes, &policy) else {
        panic!("network-capable migration was accepted");
    };
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
    assert!(
        error.message.contains("migrations/001.wasm"),
        "{}",
        error.message
    );
}

#[test]
fn signed_malformed_components_are_refused() {
    let mut fixture = base();
    fixture.files.insert(
        fixture.manifest.runtime.component.clone(),
        b"malformed component".to_vec(),
    );
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let Err(error) = verify(&bytes, &policy) else {
        panic!("malformed component was accepted");
    };
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
}

#[test]
fn package_policy_cannot_admit_unregistered_worlds_or_migrations() {
    let mut fixture = base();
    fixture.manifest.world = "chariox:app/app@1.0.1".into();
    let bytes = packed(&fixture, &[]);
    let policy = policy();
    let error = verify(&bytes, &policy).unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
    assert!(error.message.contains("1.0.1"));
    let bytes = packed(&base(), &[]);
    let mut no_migration = policy;
    no_migration
        .worlds
        .retain(|world| world.name != "chariox:app/migration");
    let error = verify(&bytes, &no_migration).unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleContract);
    assert!(error.message.contains("chariox:app/migration@1.0.0"));
}

#[test]
fn kernel_verifier_admits_the_registered_signed_migration_world() {
    let mut fixture = base();
    if let Some(actions) = fixture.manifest.ui_actions.take() {
        fixture.files.remove(&actions);
    }
    fixture.manifest.required_features = fixture.manifest.derive_features();
    let bytes = packed(&fixture, &[]);
    let policy = crate::VerificationPolicy::new(619, policy().trusted_publishers);
    assert!(matches!(
        crate::verify_app(&bytes, &policy),
        Ok(crate::VerifiedAppPackage::Wasm(_))
    ));
}

#[test]
fn idempotent_tool_policy_is_signed_and_defaults_to_no_retries() {
    let fixture = base();
    let original = serde_json::to_value(&fixture.manifest).unwrap();
    assert!(original.get("idempotentTools").is_none());
    let mut declared = original;
    declared["idempotentTools"] = json!(["list"]);
    let manifest: ManifestV2 =
        serde_json::from_value(declared).expect("v2 supports explicit retry declarations");
    let serialized = serde_json::to_value(&manifest).unwrap();
    assert_eq!(serialized["idempotentTools"], json!(["list"]));
}

#[test]
fn idempotent_tool_policy_rejects_undeclared_names_and_changes_signed_content() {
    let mut fixture = base();
    let original = packed(&fixture, &[]);
    let original_digest = verify(&original, &policy())
        .unwrap()
        .digests()
        .content
        .clone();
    fixture.manifest.idempotent_tools = vec!["create_todo".into()];
    let declared = packed(&fixture, &[]);
    let verified = verify(&declared, &policy()).unwrap();
    assert_ne!(verified.digests().content, original_digest);
    assert_eq!(verified.manifest().idempotent_tools, ["create_todo"]);
    fixture.manifest.idempotent_tools = vec!["absent".into()];
    assert!(pack(
        &fixture.manifest,
        &fixture.files,
        &key(),
        &[],
        &Limits::default()
    )
    .is_err());
    for names in [
        vec!["create_todo", "create_todo"],
        vec!["z", "a"],
        vec!["Bad-Name"],
    ] {
        fixture.manifest.idempotent_tools = names.into_iter().map(String::from).collect();
        assert!(fixture.manifest.validate(&Limits::default()).is_err());
    }
}

#[test]
fn generic_verifier_accepts_declared_ui_actions() {
    let fixture = base();
    let bytes = packed(&fixture, &[]);
    let generic = crate::VerificationPolicy::new(500, policy().trusted_publishers.clone());
    let verified = crate::verify_app(&bytes, &generic).unwrap();
    assert!(!crate::VerifiedApp::ui_actions(&verified).is_empty());
}

#[test]
fn generic_verifier_refuses_a_signed_unknown_required_feature() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use ed25519_dalek::Signer;
    use sha2::Digest;
    let fixture = base();
    let bytes = packed(&fixture, &[]);
    let entries = archive::parse(&bytes, &Limits::default()).unwrap();
    let mut manifest: Value = serde_json::from_slice(entries[0].data).unwrap();
    manifest["requiredFeatures"]
        .as_array_mut()
        .unwrap()
        .push(json!("future-not-advertised"));
    let inventory =
        read_inventory(entries[1].data, INTEGRITY_SCHEMA_V2, &Limits::default()).unwrap();
    let subject = signing_bytes_for(SIGNATURE_SCHEMA_V2, &manifest, &inventory).unwrap();
    let signature = json!({"key_id":manifest["publisher"]["keyId"],"algorithm":"ed25519","digest":format!("sha256:{:x}",sha2::Sha256::digest(&subject)),"value":STANDARD.encode(key().sign(&subject).to_bytes())});
    let manifest = serde_json_canonicalizer::to_vec(&manifest).unwrap();
    let signature = serde_json_canonicalizer::to_vec(&signature).unwrap();
    let mut forged = vec![
        (archive::CONTROL_PATHS[0], manifest.as_slice()),
        (archive::CONTROL_PATHS[1], entries[1].data),
        (archive::CONTROL_PATHS[2], signature.as_slice()),
    ];
    forged.extend(
        entries[3..]
            .iter()
            .map(|entry| (entry.path.as_str(), entry.data)),
    );
    let bytes = archive::encode(&forged, &Limits::default()).unwrap();
    let generic = crate::VerificationPolicy::new(500, policy().trusted_publishers.clone());
    assert_eq!(
        code(crate::verify_app(&bytes, &generic)),
        crate::ErrorCode::InvalidManifest
    );
}
