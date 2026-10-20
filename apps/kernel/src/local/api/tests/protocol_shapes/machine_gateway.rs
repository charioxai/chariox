use super::*;
use crate::local::{
    GatewayMachine, GatewayMachineStatus, GatewayMachineVia, ListGatewayMachinesRequest, OnBehalfOf,
};
use crate::transport::kernel_protocol::KernelIncomingFrame;

// EA2-04/05: the shapes an external agent and a forwarding kernel depend on.
#[test]
fn machine_gateway_protocol_568_shapes_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let list = LocalDaemonRequest::ListGatewayMachines(ListGatewayMachinesRequest {});
    let listed = LocalDaemonResponse::GatewayMachinesListed {
        machines: vec![GatewayMachine {
            name: "b3".into(),
            kernel_id: "kernel-b3".into(),
            machine_id: "machine-b3".into(),
            machine_alias: Some("ovh-b3".into()),
            status: GatewayMachineStatus::Online,
            last_seen_ms: 1_000,
            gateway_ready: true,
            via: GatewayMachineVia::Hosted,
        }],
    };
    let frame: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r1", "command_id": "c1", "machine": "b3",
        "request": {"ListGatewayMachines": {}},
    }))
    .unwrap();
    let KernelIncomingFrame::Request { machine, .. } = &frame else {
        panic!("a request frame")
    };
    assert_eq!(machine.as_deref(), Some("b3"));
    let plain: KernelIncomingFrame = serde_json::from_value(serde_json::json!({
        "type": "request", "request_id": "r2", "request": {"ListGatewayMachines": {}},
    }))
    .unwrap();
    assert!(matches!(plain, KernelIncomingFrame::Request { machine: None, .. }));
    let on_behalf_of = OnBehalfOf {
        origin_kernel: "kernel-mac".into(),
        grant_id: "grant-1".into(),
        harness: "claude".into(),
    };
    let snapshot = serde_json::json!({
        "list": list, "listed": listed, "frame": frame, "on_behalf_of": on_behalf_of,
        "capability": crate::local::RUNTIME_CONTROL_CAPABILITIES.contains(&"machine_gateway_v1"),
    });
    assert_eq!(snapshot["capability"], true);
    assert_eq!(snapshot["frame"]["machine"], "b3");
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "68ff6600fb34b32b9a3fbef809e0ebf484e8ec326be9e13ec78e8622009a5c65"
    );
}
