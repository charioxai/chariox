use crate::local::*;
use serde_json::json;

#[test]
fn app_discovery_cursor_is_part_of_the_versioned_shared_tool_contract() {
    use crate::transport::runtime_tools::{
        extension_runtime_tool_specs, ListExtensionsArgs, LIST_EXTENSIONS_TOOL,
    };
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 491);
    let args = ListExtensionsArgs {
        kind: Some("app".into()),
        apps_cursor: Some("app-099".into()),
    };
    let snapshot = json!({"kind":"app","apps_cursor":"app-099"});
    assert_eq!(serde_json::to_value(&args).unwrap(), snapshot);
    assert_eq!(
        serde_json::from_value::<ListExtensionsArgs>(snapshot).unwrap(),
        args
    );
    let spec = extension_runtime_tool_specs()
        .into_iter()
        .find(|spec| spec.name == LIST_EXTENSIONS_TOOL)
        .unwrap();
    assert_eq!(
        spec.input_schema,
        json!({
            "type":"object",
            "properties": {
                "kind":{"type":"string","enum":["mcp","skill","script","connector","app","all"]},
                "apps_cursor":{"type":"string","minLength":1,"maxLength":128}
            },
            "additionalProperties":false
        })
    );
}

#[test]
fn app_bindings_use_the_shared_extension_request_contract() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 491);
    let grant = LocalDaemonRequest::GrantAgentExtension(GrantAgentExtensionRequest {
        workspace_id: None,
        agent_ref: "agent-1".into(),
        kind: ExtensionKind::App,
        name: "installed-app".into(),
        environment: None,
        credential: None,
        max_safety: None,
    });
    let revoke = LocalDaemonRequest::RevokeAgentExtension(RevokeAgentExtensionRequest {
        agent_ref: "agent-1".into(),
        kind: ExtensionKind::App,
        name: "installed-app".into(),
    });
    for (request, snapshot) in [
        (
            grant,
            json!({"GrantAgentExtension":{"workspace_id":null,"agent_ref":"agent-1","kind":"app","name":"installed-app"}}),
        ),
        (
            revoke,
            json!({"RevokeAgentExtension":{"agent_ref":"agent-1","kind":"app","name":"installed-app"}}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&request).unwrap(), snapshot);
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(snapshot).unwrap(),
            request
        );
    }
    let binding = crate::extension::ExtensionGrant::app("installed-app");
    assert_eq!(
        serde_json::to_value(&binding).unwrap(),
        json!({"kind":"app","name":"installed-app"})
    );
    binding.validate_app_binding().unwrap();
    for field in ["environment", "credential", "max_safety"] {
        let mut value = serde_json::to_value(&binding).unwrap();
        value[field] = json!("cannot-grant-extra-authority");
        assert!(
            serde_json::from_value::<crate::extension::ExtensionGrant>(value)
                .unwrap()
                .validate_app_binding()
                .is_err()
        );
    }
    assert_eq!(
        crate::extension::ExtensionKind::from(ExtensionKind::App).as_str(),
        "app"
    );
}

// MP-08/MP-10/MP-11 A05: persisted and projected App grants are kernel issued.
#[test]
fn capability_app_authority_protocol_462_shape_and_hash() {
    use sha2::{Digest, Sha256};
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 491);
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        101
    );
    let mut grant = crate::extension::ExtensionGrant::app("installed");
    grant.app_grant = Some(crate::extension::AppCapabilityGrant {
        grant_id: "grant-1".into(),
        expires_at_ms: 28_800_001,
        prompt_id: Some("prompt-1".into()),
        delegated_by_agent_id: Some("parent".into()),
        delegated_from_grant_id: Some("grant-parent".into()),
    });
    let value = serde_json::to_value(&grant).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
        "f3c23ca0b26737a55b1bb6edd84edcedcf5c43ea52daa4b23207763218f9ca38"
    );
    assert_eq!(
        serde_json::from_value::<crate::extension::ExtensionGrant>(value).unwrap(),
        grant
    );
    assert!(
        grant.validate_app_binding().is_err(),
        "MP-11: issued authority cannot be submitted as a grant request"
    );
}
