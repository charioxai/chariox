use super::*;
use sha2::{Digest, Sha256};
#[test]
fn byom_mp08_ssh_machine_protocol_479_shape_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 479);
    let request = LocalDaemonRequest::AddSshMachine(crate::local::AddSshMachineRequest {
        host: "linux-lan".into(),
        install_id: Some("byom-lan-eval".into()),
        port: Some(55129),
        release: Some("approved".into()),
    });
    let remove = LocalDaemonRequest::RemoveSshMachine(crate::local::RemoveSshMachineRequest {
        install_id: "byom-lan-eval".into(),
    });
    let response = LocalDaemonResponse::SshMachine {
        machine: crate::local::SshMachineResult {
            install_id: "byom-lan-eval".into(),
            status: "ready".into(),
            kernel_id: Some("kernel".into()),
            machine_id: Some("machine".into()),
            release_digest: "sha256:approved".into(),
            state_retained: false,
        },
    };
    let snapshot = serde_json::json!({"request":request,"remove":remove,"response":response});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "c9d37515b4b3bce4701b8c0ba5903ba5129861b3fcb74b19c46c391481ecf42b"
    );
}
