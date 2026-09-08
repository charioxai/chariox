use crate::local::LOCAL_DAEMON_PROTOCOL_VERSION;
use chariox_app_package::{EventDeclaration, SUPPORTED_SDK_VERSION};
use chariox_app_runtime::app_outbox::Occurrence;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn app_event_payload_contract_is_versioned_and_matches_the_sdk_fixture() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 292);
    let fixture: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
        "/../../packages/app-sdk/test/event-contract.json"))).unwrap();
    assert_eq!(fixture["minimumKernelProtocol"], 292);
    assert_eq!(fixture["sdkVersion"], SUPPORTED_SDK_VERSION);
    let declaration: EventDeclaration = serde_json::from_value(fixture["declaration"].clone()).unwrap();
    let occurrence: Occurrence = serde_json::from_value(fixture["occurrence"].clone()).unwrap();
    let actual = json!({"declaration": declaration, "occurrence": occurrence});
    assert_eq!(actual["declaration"], fixture["declaration"]);
    assert_eq!(actual["occurrence"], fixture["occurrence"]);
    assert_eq!(format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "71b7f9838ad25fcec71051053e415fe2aae02ea8887f3f0d140f3236f1aceea1");
    for field in ["occurredAtMs", "eventVersion", "invocation"] {
        let mut incomplete = fixture["occurrence"].clone();
        incomplete.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Occurrence>(incomplete).is_err());
    }
    let mut forged = fixture["occurrence"].clone();
    forged["owner"] = json!("forged");
    assert!(serde_json::from_value::<Occurrence>(forged).is_err());
}
