use super::*;

#[test]
fn web_bot_auth_project_opt_in_protocol_shape_and_hash() {
    let config = crate::config::UserWebBotAuthConfig {
        projects: vec!["project-wba".into()],
        signature_agent: Some("https://operator.example".into()),
    };
    let snapshot = serde_json::json!({
        "protocol_version": LOCAL_DAEMON_PROTOCOL_VERSION,
        "relay_peer_protocol_version": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "web_bot_auth": config,
    });
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("web-bot-auth-946.json")).unwrap();
    assert_eq!(snapshot, expected);
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
    );
    assert_eq!(
        hash,
        "11966f8c1605e00172c50708a2a6a0d79b8b67d1bc6e8f31af76614538dfb0de"
    );
    let specs = crate::transport::runtime_tools::credential_runtime_tool_specs();
    let plain = specs
        .iter()
        .find(|spec| spec.name == "chariox.http_request")
        .unwrap();
    assert_eq!(plain.input_schema["required"], serde_json::json!(["url"]));
    assert!(plain.input_schema["properties"]
        .get("credential_id")
        .is_none());
    let credential = specs
        .iter()
        .find(|spec| spec.name == "chariox.http_request_with_credential")
        .unwrap();
    assert_eq!(
        credential.input_schema["required"],
        serde_json::json!(["credential_id", "url"])
    );
}
