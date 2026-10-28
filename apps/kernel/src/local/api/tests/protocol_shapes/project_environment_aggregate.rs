//! MP-08: P01 protocol allocation and full contract source guard.
use super::*;
#[test]
fn envp01_aggregate_protocol_471_request_rejects_session_and_agent_context() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request =
        LocalDaemonRequest::GetProjectEnvironment(crate::local::GetProjectEnvironmentRequest {
            project_id: "project-1".into(),
            view: None,
        });
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"GetProjectEnvironment":{"projectId":"project-1"}})
    );
    for field in ["sessionId", "agentId", "value"] {
        let mut value = serde_json::json!({"GetProjectEnvironment":{"projectId":"project-1"}});
        value["GetProjectEnvironment"][field] = serde_json::json!("unexpected");
        assert!(serde_json::from_value::<LocalDaemonRequest>(value).is_err());
    }
}
#[test]
fn envp01_full_environment_shapes_require_protocol_bump() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let contract = include_str!("../../../../project_environment/aggregate.rs");
    let view = include_str!("../../../../project_environment/read_view.rs");
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(view.split("#[cfg(test)]").next().unwrap().as_bytes())
        ),
        "ee7fde1092a82995b81e141a9bc374a3bd888e7207d728528302b9c2b17ee20c"
    );
    let requests = include_str!("../../types/project_environment_aggregate.rs");
    assert_eq!(
        format!("{:x}", Sha256::digest(requests.as_bytes())),
        "68f03a45a0e2eeed99c5d72985cf6de3de99092d8c0a5b3478b586c4676e9674"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(contract.as_bytes())),
        "734ac61bcc28f3aace166b06798ddbb4d56a63294413e9ca325089b80f30235c"
    );
}

#[test]
fn envp01_get_response_snapshot_hash() {
    use crate::project_environment::*;
    use sha2::{Digest, Sha256};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let project = crate::session::RuntimeProject::new(
        "project-1",
        "local",
        "/plain/one",
        "Plain",
        crate::session::RuntimeProjectKind::Named,
    );
    let environment = project_environment_snapshot(
        &project,
        EnvironmentLineage {
            project_id: "lineage-project".into(),
            environment_id: "lineage-environment".into(),
        },
        &std::collections::BTreeMap::from([("/plain/one".into(), "folder-1".into())]),
        None,
    );
    let response = LocalDaemonResponse::ProjectEnvironment { environment };
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["ProjectEnvironment"]["environment"]["revision"], 0);
    let encoded = serde_json::to_string(&response).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "84a4f39f64cba008a705b617e6cfefa86d84280431d6981a79bf832362d0d482"
    );
}

// MP-08 / MP-10 / MP-11: protocol 660 keeps the legacy full read and adds explicit bounded reads.
#[test]
fn environment_view_protocol_505_shapes_and_hash() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let value = serde_json::json!({"GetProjectEnvironment":{"projectId":"project-1","view":{"knownDigest":null,"proposals":{"snapshotDigest":"digest","offset":8,"limit":20}}}});
    let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), value);
    let response = LocalDaemonResponse::ProjectEnvironmentView {
        view: crate::project_environment::EnvironmentReadView {
            observed_at_ms: 1234,
            snapshot_digest: "digest".into(),
            environment: None,
            proposal_count: 42,
            proposal_offset: 8,
            next_proposal_offset: Some(28),
            proposals: vec![],
            operation_item_counts: vec![42],
        },
    };
    let snapshot = serde_json::json!({"ProjectEnvironmentView":{"view":{"observed_at_ms":1234,"snapshot_digest":"digest","environment":null,"proposal_count":42,"proposal_offset":8,"next_proposal_offset":28,"proposals":[],"operation_item_counts":[42]}}});
    assert_eq!(serde_json::to_value(&response).unwrap(), snapshot);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&response).unwrap())
        ),
        "42585c4c487e819f3b3cab90ce2577c556ea15f06e1d8473ae15b646e020ed48"
    );
}
