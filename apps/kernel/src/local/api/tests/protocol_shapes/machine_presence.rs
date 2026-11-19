//! Local 1012: a listed machine is online or reconnecting; machines gone longer are offline in their own list, Remove forgets one, and a listed machine names its kernels with their protocols.
use super::*;
use crate::local::{
    ForgetRemoteMachineRequest, OfflineMachineRecord, RemoteMachineKernel, RemoteMachinePresence,
    RemoteMachineRecord, RemoteMachineTrustStatus,
};

#[test]
fn local_daemon_protocol_machine_presence_shape_is_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );

    let forget = serde_json::json!({ "ForgetRemoteMachine": { "machine_ref": "machine-2" } });
    let parsed: LocalDaemonRequest = serde_json::from_value(forget.clone()).unwrap();
    assert!(matches!(
        &parsed,
        LocalDaemonRequest::ForgetRemoteMachine(ForgetRemoteMachineRequest { machine_ref })
            if machine_ref == "machine-2"
    ));
    assert_eq!(serde_json::to_value(parsed).unwrap(), forget);

    let listed = LocalDaemonResponse::RemoteMachinesListed {
        machines: vec![RemoteMachineRecord {
            machine_id: "machine-1".into(),
            machine_alias: None,
            display_name: "builder".into(),
            trust_status: RemoteMachineTrustStatus::Approved,
            presence: RemoteMachinePresence::Reconnecting,
            pending: false,
            kernel_count: 1,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            kernels: vec![RemoteMachineKernel {
                kernel_id: "kernel-1".to_string(),
                local_protocol: Some(1012),
            }],
        }],
        offline_machines: vec![OfflineMachineRecord {
            machine_id: "machine-2".into(),
            display_name: "laptop".into(),
            last_seen_ms: 42,
        }],
    };
    assert_eq!(
        serde_json::to_value(listed).unwrap(),
        serde_json::json!({
            "RemoteMachinesListed": {
                "machines": [{
                    "machine_id": "machine-1",
                    "machine_alias": null,
                    "display_name": "builder",
                    "trust_status": "approved",
                    "presence": "reconnecting",
                    "pending": false,
                    "kernel_count": 1,
                    "available_providers": [],
                    "provider_accounts": [],
                    "kernels": [{"kernel_id": "kernel-1", "local_protocol": 1012}]
                }],
                "offline_machines": [
                    { "machine_id": "machine-2", "display_name": "laptop", "last_seen_ms": 42 }
                ]
            }
        })
    );

    let forgotten = LocalDaemonResponse::RemoteMachineForgotten {
        machine_id: "machine-2".into(),
        display_name: "laptop".into(),
    };
    assert_eq!(
        serde_json::to_value(forgotten).unwrap(),
        serde_json::json!({
            "RemoteMachineForgotten": { "machine_id": "machine-2", "display_name": "laptop" }
        })
    );
}
