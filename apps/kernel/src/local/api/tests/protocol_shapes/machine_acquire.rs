//! Owner machine acquisition: the options, acquire, status and release requests and their answers.
use super::*;
use sha2::{Digest, Sha256};

#[test]
fn machine_acquire_snapshot_and_hash() {
    let bytes = include_bytes!("machine-acquire-1015.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "bb877eab4f94e7ca23b0bcf7e6e0af02b575fc777b8f10e39a98a80005c349c1"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["protocol_version"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(
        fixture["relay_peer_protocol_version"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    for shape in fixture["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *shape);
    }
    for shape in fixture["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *shape);
    }
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"MachineAcquisition":{"action":"options","token":"must-not-travel"}})
    )
    .is_err());
}

/// A response carries exactly one of `option` (a named choice) and `offer` (the service's default);
/// decoding keeps which one, and the quoted terms with it, even when the value is null.
#[test]
fn machine_acquire_answer_keeps_named_option_apart_from_service_default() {
    use crate::local::{MachineAcquisitionChoice, MachineAcquisitionResult};
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("machine-acquire-1015.json")).unwrap();
    let mut chosen = Vec::new();
    for shape in fixture["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        if let LocalDaemonResponse::MachineAcquisition {
            result: MachineAcquisitionResult::Acquired { chosen: choice, .. },
        } = response
        {
            chosen.push(choice);
        }
    }
    assert_eq!(
        chosen.len(),
        4,
        "named option, service default, null offer, null option"
    );
    assert!(
        matches!(&chosen[0], MachineAcquisitionChoice::Option { option: Some(option) } if option.class_id == "small")
    );
    assert!(
        matches!(&chosen[1], MachineAcquisitionChoice::ServiceDefault { offer: Some(offer) } if offer.terms.amount == "0.01" && offer.machine_type == "Standard")
    );
    assert!(matches!(
        &chosen[2],
        MachineAcquisitionChoice::ServiceDefault { offer: None }
    ));
    assert!(matches!(
        &chosen[3],
        MachineAcquisitionChoice::Option { option: None }
    ));
    // A response with both, or neither, is not a choice.
    let allocation = fixture["responses"][1]["MachineAcquisition"]["result"]["allocation"].clone();
    let estimate = fixture["responses"][1]["MachineAcquisition"]["result"]["estimate"].clone();
    for extra in [
        serde_json::json!({"option": null, "offer": null}),
        serde_json::json!({}),
    ] {
        let mut result = serde_json::json!({"allocation": allocation, "estimate": estimate});
        result
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let answer = serde_json::json!({"MachineAcquisition": {"result": result}});
        assert!(serde_json::from_value::<LocalDaemonResponse>(answer).is_err());
    }
}
