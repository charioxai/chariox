use super::*;
use sha2::{Digest, Sha256};

#[test]
fn offload_jobs_protocol_snapshots_and_hashes() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/remote-jobs-protocol.json"
    )))
    .unwrap();
    assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= fixture["local"].as_u64().unwrap() as u32);
    assert!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            >= fixture["relayPeer"].as_u64().unwrap() as u32
    );
    for shape in fixture["shapes"].as_array().unwrap() {
        let wire = shape["wire"].clone();
        let encoded = match shape["kind"].as_str().unwrap() {
            "request" => serde_json::to_value(
                serde_json::from_value::<LocalDaemonRequest>(wire.clone()).unwrap(),
            )
            .unwrap(),
            "response" => serde_json::to_value(
                serde_json::from_value::<LocalDaemonResponse>(wire.clone()).unwrap(),
            )
            .unwrap(),
            "event" => serde_json::to_value(
                serde_json::from_value::<crate::transport::kernel_protocol::KernelEvent>(
                    wire.clone(),
                )
                .unwrap(),
            )
            .unwrap(),
            "run_job" => serde_json::to_value(
                serde_json::from_value::<crate::local::RunJobRequest>(wire.clone()).unwrap(),
            )
            .unwrap(),
            "job_view" => serde_json::to_value(
                serde_json::from_value::<crate::local::JobView>(wire.clone()).unwrap(),
            )
            .unwrap(),
            "agents_config" => serde_json::to_value(
                serde_json::from_value::<crate::config::UserAgentsConfig>(wire.clone()).unwrap(),
            )
            .unwrap(),
            "session_jobs" => {
                let mut input = wire.clone();
                input["SessionState"]["session"] =
                    serde_json::to_value(crate::session::RuntimeSession::new(
                        "room",
                        None,
                        "workspace",
                        "worktree",
                        "machine",
                        "home",
                    ))
                    .unwrap();
                let mut output = serde_json::to_value(
                    serde_json::from_value::<LocalDaemonResponse>(input).unwrap(),
                )
                .unwrap();
                output["SessionState"]["session"] = serde_json::Value::Null;
                output
            }
            "session_process" => serde_json::to_value(
                serde_json::from_value::<crate::runtime::process_resources::SessionProcess>(
                    wire.clone(),
                )
                .unwrap(),
            )
            .unwrap(),
            kind => panic!("Unknown offload fixture kind: {kind}"),
        };
        assert_eq!(encoded, wire, "{}", shape["kind"]);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&encoded).unwrap())
            ),
            shape["sha256"].as_str().unwrap()
        );
    }
    let absent: crate::config::CharioxUserConfig =
        serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!absent.agents.offload_without_asking.unwrap_or(false));
    for request in [
        serde_json::json!({"command":["true"], "env":["BUILD_FLAG"], "env_values":{"BUILD_FLAG":"must-not-travel"}}),
        serde_json::json!({"command":["true"], "needs":{"provider":"must-not-travel"}}),
    ] {
        assert!(serde_json::from_value::<crate::local::RunJobRequest>(request).is_err());
    }
}
