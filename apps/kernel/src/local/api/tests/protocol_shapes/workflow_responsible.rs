//! Who answers for a workflow, how a run stands and the requests that change them are part of the client wire.
use crate::local::LocalDaemonRequest;
use crate::session::{WorkflowResponsible, WorkflowRunOutcome};
use crate::transport::relay_peer::RelayPeerRequest;
use sha2::{Digest, Sha256};

// Local 1012 / relay peer 449: a workflow has a responsible, a run has a number, an outcome and an attention flag.
#[test]
fn workflow_responsible_run_outcome_and_requests_are_bound_to_protocol1011() {
    let bytes = include_bytes!("workflow-responsible-1011.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "27e54d2c445f487e710e594a0eb50561d7023f081b4639fcbe4006d569c67237"
