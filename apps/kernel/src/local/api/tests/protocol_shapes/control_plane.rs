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

/// Provisional local 1015: an acquire request may carry `timeout_s`, the rental's hard stop in seconds.
#[test]
fn machine_hard_stop_1015_shapes_and_hash() {
    let snapshot = serde_json::json!({
        "requests": [
            {"ControlPlane": {"action": "acquire", "capability_revision": "revision", "request_id": "one", "class_id": "small", "region": "auto", "timeout_s": 300}}
        ]
    });
    for shape in snapshot["requests"].as_array().unwrap() {
        let request: LocalDaemonRequest = serde_json::from_value(shape.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *shape);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "b26d2c8d980fd7803c9d4521660503546df57478acdae7312b52b62e66b94c49"
    );
    for bad in [serde_json::json!(-1), serde_json::json!("300")] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(
            serde_json::json!({"ControlPlane":{"action":"acquire","capability_revision":"r","request_id":"i","timeout_s":bad}})
        )
        .is_err());
    }
}

/// Local 1015 (unreleased with Landing 14): the `runtime_interactions_changed` event carries the machine
/// approval's receipt in each of its three states.
#[test]
fn machine_approval_receipt_event_shape_and_hash() {
    let receipts = serde_json::json!([
        {"id": "machine-acquisition-direct-a", "shown_in_agent_id": "agent", "message": "builder in Build Room · Run the build — Acquire small in north at 0.000284 EUR per minute", "state": "approving", "expires_at_ms": null},
        {"id": "machine-acquisition-direct-b", "shown_in_agent_id": null, "message": "Acquire a machine", "state": "sent", "expires_at_ms": null},
        {"id": "machine-acquisition-direct-c", "shown_in_agent_id": "agent", "message": "Acquire small", "state": "expired", "expires_at_ms": 300000}
    ]);
    let shape = serde_json::json!({"event": "runtime_interactions_changed", "session_id": "room", "active_interactions": [], "machine_approval_receipts": receipts});
    let event: crate::transport::kernel_protocol::KernelEvent =
        serde_json::from_value(shape.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), shape);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "f941a4989240727c5de851ba45dfa9557d45472a54231a019cec6dd9bd77edfb"
    );
}

/// The serialized Room session carries the same receipts under `machine_approval_receipts`.
#[test]
fn machine_approval_receipts_in_the_session_shape_and_hash() {
    use crate::session::{MachineApprovalReceipt, MachineApprovalReceiptState as State};
    let mut session = crate::session::RuntimeSession::new(
        "s",
        crate::session::ProjectId::test(),
        None,
        "workspace",
        "worktree",
        "machine",
        "daemon",
    );
    for (id, agent, message, state, expires) in [
        (
            "machine-acquisition-direct-a",
            Some("agent"),
            "Acquire small",
            State::Approving,
            None,
        ),
        (
            "machine-acquisition-direct-b",
            None,
            "Acquire a machine",
            State::Sent,
            None,
        ),
        (
            "machine-acquisition-direct-c",
            Some("agent"),
            "Acquire small",
            State::Expired,
            Some(300000),
        ),
    ] {
        session.upsert_machine_approval_receipt(MachineApprovalReceipt {
            id: id.into(),
            shown_in_agent_id: agent.map(str::to_owned),
            message: message.into(),
            state,
            expires_at_ms: expires,
        });
    }
    let shape = serde_json::to_value(&session).unwrap()["machine_approval_receipts"].clone();
    assert_eq!(
        shape,
        serde_json::json!([
            {"id": "machine-acquisition-direct-a", "shown_in_agent_id": "agent", "message": "Acquire small", "state": "approving", "expires_at_ms": null},
            {"id": "machine-acquisition-direct-b", "shown_in_agent_id": null, "message": "Acquire a machine", "state": "sent", "expires_at_ms": null},
            {"id": "machine-acquisition-direct-c", "shown_in_agent_id": "agent", "message": "Acquire small", "state": "expired", "expires_at_ms": 300000}
        ])
    );
    let back: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&session).unwrap()).unwrap();
    assert_eq!(
        back.machine_approval_receipts(),
        session.machine_approval_receipts()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&shape).unwrap())),
        "94e6a5f1c1c3e7d97ff0d57122df03ce00d2c42ca8b96c376e50fb3604b9d12b"
    );
}

/// Local 1017: a machine row may carry the class, region, creation time and hard stop the service lists,
/// and a status request without an allocation_ref lists the owner's machines.
#[test]
fn machine_list_1017_shapes_and_hash() {
    let snapshot = serde_json::json!({"requests":[{"ControlPlane":{"action":"list"}},{"MachineAcquisition":{"action":"status"}}],"responses":[{"ControlPlane":{"result":{"kind":"machines","machines":[
        {"allocation_ref":"resource","name":"Computer","state":"ready","revision":1,"since":"2026-11-22T00:00:00Z","kernel_id":null,"machine_id":null,"available_actions":["release"],"class_id":"small","region":"north","created_at":"2026-11-21T12:00:00Z","hard_stop_at":"2026-11-22T01:00:00Z"},
        {"allocation_ref":"older","name":"Computer 2","state":"ready","revision":1,"since":"2026-11-22T00:00:00Z","kernel_id":null,"machine_id":null,"available_actions":["release"]}],"saved_states":[]}}}]});
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
        "cbcaa821b538dcd51d626060628c3d8330e9c92598004a343e4c03613d3fa579"
    );
}
