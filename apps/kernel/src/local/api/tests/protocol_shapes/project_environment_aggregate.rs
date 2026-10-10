//! MP-08: P01 protocol allocation and full contract source guard.
use super::*;
#[test]
fn envp01_aggregate_protocol_471_request_rejects_session_and_agent_context() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 487);
    let request =
        LocalDaemonRequest::GetProjectEnvironment(crate::local::GetProjectEnvironmentRequest {
            project_id: "project-1".into(),
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 487);
    let contract = include_str!("../../../../project_environment/aggregate.rs");
    let requests = include_str!("../../types/project_environment_aggregate.rs");
    assert_eq!(
        format!("{:x}", Sha256::digest(requests.as_bytes())),
        "7d24c7bf248c4f03a5226512377a334493940886ccd7823b459f00b72d7fbb06"
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 487);
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
