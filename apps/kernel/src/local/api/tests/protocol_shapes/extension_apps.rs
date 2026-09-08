use crate::local::*;
use serde_json::json;

#[test]
fn app_discovery_cursor_is_part_of_the_versioned_shared_tool_contract() {
    use crate::transport::runtime_tools::{
        extension_runtime_tool_specs, ListExtensionsArgs, LIST_EXTENSIONS_TOOL,
    };
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 295);
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 295);
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
