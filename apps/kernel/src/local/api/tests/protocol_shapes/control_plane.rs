use super::*;
use sha2::{Digest, Sha256};
#[test]
fn mp08_mp11_control_plane_959_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let snapshot = serde_json::json!({"requests":[{"ControlPlane":{"action":"capabilities"}},{"ControlPlane":{"action":"acquire","capability_revision":"revision","request_id":"one"}},{"ControlPlane":{"action":"list"}},{"ControlPlane":{"action":"get","allocation_ref":"resource"}},{"ControlPlane":{"action":"release","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"start","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"stop","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1}},{"ControlPlane":{"action":"start_saved_state","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1,"saved_state_ref":"saved-record"}},{"ControlPlane":{"action":"delete_saved_state","allocation_ref":"resource","capability_revision":"revision","request_id":"two","expected_revision":1,"saved_state_ref":"saved-record"}}],"responses":[{"ControlPlane":{"result":{"kind":"capabilities","machine_allocation":null}}},{"ControlPlane":{"result":{"kind":"capabilities","machine_allocation":{"revision":"revision","actions":["acquire","list"],"offer":{"machine_type":"Standard","vcpu":2,"ram_gb":4.0,"disk_gb":40,"location":"Europe","terms":{"amount":"0.01","currency":"EUR","period":"hour"}}}}}},{"ControlPlane":{"result":{"kind":"capabilities","machine_allocation":{"revision":"revision","actions":["list","get","stop","release","delete_saved_state"],"offer":null}}}},{"ControlPlane":{"result":{"kind":"machines","machines":[{"allocation_ref":"resource","name":"Computer","state":"ready","revision":1,"since":"2026-11-02T00:00:00Z","kernel_id":"kernel","machine_id":"machine","available_actions":["stop","release"]}],"saved_states":[]}}},{"ControlPlane":{"result":{"kind":"machine","allocation_ref":"resource","revision":1,"available_actions":["stop","release"],"state":"ready","access_state":"available","access_reason":null,"kernel_target":{"kernel_id":"kernel","machine_id":"machine","relay_realm_id":"realm","key_thumbprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}]});
    for shape in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *shape);
    }
    for shape in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *shape);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "288c4898698e5e564789f904ec6922c71ebffdb24a3468d61d1d4a03a579dfd4"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ControlPlane":{"action":"capabilities","token":"must-not-travel"}})
    )
    .is_err());
}

/// The connected service builds this offer; its contract test reads the same fixture.
#[test]
fn machine_acquisition_offer_fixture_is_the_service_contract() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../fixtures/machine-acquisition-offer.json"
    ))
    .unwrap();
    let offer: crate::local::MachineAcquisitionOffer =
        serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(serde_json::to_value(offer).unwrap(), fixture);
}

/// Local 1011: the capability lists every machine option and Acquire names the chosen one.
#[test]
fn machine_options_1011_shapes_and_hash() {
    let snapshot = serde_json::json!({
        "requests": [
            {"ControlPlane": {"action": "acquire", "capability_revision": "revision", "request_id": "one"}},
            {"ControlPlane": {"action": "acquire", "capability_revision": "revision", "request_id": "two", "class_id": "small", "region": "auto"}}
        ],
        "responses": [{"ControlPlane": {"result": {"kind": "capabilities", "machine_allocation": {
            "revision": "revision", "actions": ["acquire"], "offer": null,
            "offers": [
                {"class_id": "small", "region": "auto", "vcpu": 2, "ram_gb": 4.0, "disk_gb": 51,
                 "rate_per_minute": {"amount_micros": 284, "currency": "EUR"}, "available": true, "note": "shown by the service"},
                {"class_id": "large", "region": "auto", "vcpu": 8, "ram_gb": 16.0, "disk_gb": 111,
                 "rate_per_minute": {"amount_micros": 1100, "currency": "EUR"}, "available": false}
            ]
        }}}}]
    });
    for shape in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *shape);
    }
    for shape in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *shape);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "d999f9d6271b3bb85cd3daa20c866b240873d5ceaa471e5a5c75779cfc183f27"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"ControlPlane":{"action":"acquire","capability_revision":"r","request_id":"i","price":1}})
    )
    .is_err());
}

#[test]
fn rental_kernel_diagnostics_are_part_of_machine_status() {
    let shape = serde_json::json!({
        "ControlPlane": {
            "result": {
                "kind": "machine",
                "allocation_ref": "allocation",
                "revision": 2,
                "available_actions": [
                    "release"
                ],
                "state": "reconnecting",
                "access_state": "pending",
                "access_reason": "kernel_offline",
                "kernel_target": null,
                "kernel_diagnostics": {
                    "kernel_version": "0.1.0",
                    "kernel_protocol": 1014,
                    "last_relay_refusal": null,
                    "relay_error": "The relay closed the connection because its reply queue filled."
                }
            }
        }
    });
    let parsed: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), shape);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "23b3a48818d406b92d7d9a0971a0c2bbe92415ab5e306bf86664977fad967a2b",
    );
}

/// Local 1016: a machine row carries the closed reason the connected service gave when it stopped
/// or failed the machine without the owner asking; a row the owner stopped carries none.
#[test]
fn machine_event_reason_1016_shapes_and_hash() {
    let row = |state: &str, reason: Option<&str>| {
        let mut row = serde_json::json!({"allocation_ref":"resource","name":"Computer","state":state,"revision":1,"since":"2026-11-02T00:00:00Z","kernel_id":null,"machine_id":null,"available_actions":["delete"]});
        if let Some(reason) = reason {
            row["reason"] = reason.into();
        }
        row
    };
    let snapshot = serde_json::json!({
        "responses": [{"ControlPlane": {"result": {"kind": "machines", "machines": [
            row("failed", Some("copy_failed")),
            row("stopped", Some("hard_stop")),
            row("failed", Some("rental_failed")),
            row("stopped", None)
        ], "saved_states": []}}}]
    });
    for shape in snapshot["responses"].as_array().unwrap() {
        let response: LocalDaemonResponse = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *shape);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "f6b79846680a47fb0551b3f6147ce86fe8bd08f1baa527364d4a2dc4bbae5eb2"
    );
    assert!(serde_json::from_value::<LocalDaemonResponse>(
        serde_json::json!({"ControlPlane": {"result": {"kind": "machines", "machines": [row("failed", Some("because"))], "saved_states": []}}})
    )
    .is_err());
}
