use super::*;
use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;
use crate::transport::runtime_tools::{agent_event_tool_specs, agent_messaging_runtime_tool_specs};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn runtime_mcp_paging_accepts_continuation_arguments() {
    let bytes = include_bytes!("runtime-mcp-paging-1014.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "17a5a3f9e9f6fbae22d2a07a7996fd2d09dd628a935163ad8c425c20c6aa553d",
    );
    let fixture: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fixture["local"], LOCAL_DAEMON_PROTOCOL_VERSION);
    assert_eq!(fixture["peer"], RELAY_PEER_PROTOCOL_VERSION);
    let mut specs = agent_event_tool_specs();
    specs.extend(agent_messaging_runtime_tool_specs());
    for (name, fields) in fixture["properties"].as_object().unwrap() {
        let spec = specs.iter().find(|spec| &spec.name == name).unwrap();
        let mut keys: Vec<_> = spec.input_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        keys.sort_unstable();
        assert_eq!(serde_json::json!(keys), *fields, "{name}");
    }
}
