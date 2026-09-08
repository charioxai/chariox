use super::*;
use chariox_app_runtime::worker_readiness::{ReadinessContract, ReadinessError};
use std::sync::Arc;

fn package_with_events(directions: &[(&str, &str)]) -> Package {
    let mut package = Package::new();
    package.manifest.events = Some("schemas/events.json".into());
    package.files.insert(
        "schemas/events.json".into(),
        serde_json::to_vec(&json!({
            "events": directions.iter().map(|(name, direction)| json!({
                "name":name,"schemaVersion":1,"direction":direction,"payloadSchema":text_schema()
            })).collect::<Vec<_>>()
        }))
        .unwrap(),
    );
    package
}
fn setup(package: &Package) -> Arc<AppCatalog> {
    let (mut connection, trust) = database(package);
    let token = install(&mut connection, "ready-app", package, &trust);
    Arc::new(package.catalog(&mut connection, &token, &trust).unwrap())
}
fn contract(package: &Package, catalog: Arc<AppCatalog>) -> ReadinessContract {
    let bytes = package.bytes();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![package.publisher()]),
    )
    .unwrap();
    ReadinessContract::from_verified(&verified, catalog).unwrap()
}

#[test]
fn outgoing_only_registration_needs_no_incoming_handler_and_is_single_use() {
    let package = package_with_events(&[("changed", "outgoing")]);
    let catalog = setup(&package);
    let mut contract = contract(&package, catalog.clone());
    assert_eq!(contract.tools().collect::<Vec<_>>(), vec!["echo"]);
    assert_eq!(contract.incoming_events().count(), 0);
    let registration = contract
        .accept(json!({"tools":["echo"],"events":[],"lifecycle":["startup","shutdown"]}))
        .unwrap();
    assert!(Arc::ptr_eq(registration.catalog(), &catalog));
    assert!(registration.supports_lifecycle("shutdown"));
    assert!(!registration.supports_lifecycle("configuration_change"));
    assert!(matches!(
        contract.accept(json!({"tools":["echo"],"events":[],"lifecycle":[]})),
        Err(ReadinessError::AlreadyReported)
    ));
}

#[test]
fn incoming_and_both_are_exact_unordered_sets_and_outgoing_is_not_a_handler() {
    let package = package_with_events(&[
        ("changed", "outgoing"),
        ("notify", "incoming"),
        ("sync", "both"),
    ]);
    let catalog = setup(&package);
    let mut contract = contract(&package, catalog);
    assert_eq!(
        contract.incoming_events().collect::<Vec<_>>(),
        vec!["notify", "sync"]
    );
    contract
        .accept(json!({"tools":["echo"],"events":["sync","notify"],"lifecycle":[]}))
        .unwrap();
}

#[test]
fn malformed_or_inexact_report_consumes_the_attempt() {
    let package = package_with_events(&[("notify", "incoming")]);
    let catalog = setup(&package);
    let valid = json!({"tools":["echo"],"events":["notify"],"lifecycle":[]});
    let invalid = [
        json!({"tools":["echo"],"events":["notify"],"lifecycle":[],"owner":"attacker"}),
        json!({"tools":[],"events":["notify"],"lifecycle":[]}),
        json!({"tools":["echo"],"events":[],"lifecycle":[]}),
        json!({"tools":["other"],"events":["notify"],"lifecycle":[]}),
        json!({"tools":["echo"],"events":["notify","notify"],"lifecycle":[]}),
        json!({"tools":["echo"],"events":["notify"],"lifecycle":["startup","startup"]}),
        json!({"tools":["echo"],"events":["notify"],"lifecycle":["execute_shell"]}),
        json!({"tools":["x".repeat(65)],"events":["notify"],"lifecycle":[]}),
        json!({"tools":["echo"],"events":["notify"],"lifecycle":null}),
        json!({"tools":["echo"],"events":["notify"]}),
    ];
    for params in invalid {
        let mut contract = contract(&package, catalog.clone());
        assert!(matches!(
            contract.accept(params),
            Err(ReadinessError::Invalid)
        ));
        assert!(matches!(
            contract.accept(valid.clone()),
            Err(ReadinessError::AlreadyReported)
        ));
    }
}

#[test]
fn another_verified_package_cannot_supply_the_bootstrap_contract() {
    let mut package = package_with_events(&[("notify", "incoming")]);
    let catalog = setup(&package);
    package.files.insert(
        "runtime/main.js".into(),
        b"export default function altered() {}".to_vec(),
    );
    let bytes = package.bytes();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(500, vec![package.publisher()]),
    )
    .unwrap();
    assert!(matches!(
        ReadinessContract::from_verified(&verified, catalog),
        Err(ReadinessError::Provenance)
    ));
}
