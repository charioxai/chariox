//! Who answers for a workflow, how a run stands and the requests that change them are part of the client wire.
use crate::local::LocalDaemonRequest;
use crate::session::{WorkflowResponsible, WorkflowRunOutcome};
use crate::transport::relay_peer::RelayPeerRequest;
use sha2::{Digest, Sha256};

// Local 1014 / relay peer 451: a workflow has a responsible, a run has a number, an outcome and an attention flag; workspace waits can name a holding run.
#[test]
fn workflow_responsible_run_outcome_and_requests_are_bound_to_protocol1011() {
    let bytes = include_bytes!("workflow-responsible-1011.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "4333ed38e2ffb25c8ef10f84cf648581129686d4b8d77cc74c27a7d5e14caf80"
    );
    let fixture: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        fixture["local"],
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION
    );
    assert_eq!(
        fixture["peer"],
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
    );
    let responsible: WorkflowResponsible =
        serde_json::from_value(fixture["workflow"]["responsible"].clone()).unwrap();
    assert!(!responsible.is_person());
    assert_eq!(
        serde_json::to_value(responsible).unwrap(),
        fixture["workflow"]["responsible"]
    );
    for key in [
        "failed_outcome",
        "waiting_outcome",
        "workspace_wait_outcome",
    ] {
        let outcome: WorkflowRunOutcome = serde_json::from_value(fixture[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(outcome).unwrap(), fixture[key]);
    }
    for key in ["set_request", "clear_request"] {
        let request: LocalDaemonRequest = serde_json::from_value(fixture[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), fixture[key]);
    }
    for key in ["peer_set_request", "peer_apply_request"] {
        let request: RelayPeerRequest = serde_json::from_value(fixture[key].clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), fixture[key]);
    }
}
