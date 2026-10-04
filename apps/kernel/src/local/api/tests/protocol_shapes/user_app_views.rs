use super::*;
use crate::local::*;

#[test]
fn user_app_view_protocol_418_shapes_and_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 418);
    let requests = serde_json::json!([
        {"OpenUserAppView":{"installation_id":"todo"}},
        {"ListUserAppViews":{}},
        {"CloseUserAppView":{"view_id":"v"}},
        {"GetUserAppViewFrontend":{"view_id":"v"}},
        {"CallUserAppView":{"view_id":"v","method":"echo","input":{"text":"hi"}}},
        {"SubscribeUserAppViews":{"after":4,"wait_ms":25000}},
        {"AnswerUserDomainInteraction":{"interaction_id":"decision","choice_id":"deny","passkey":null,"passkey_remember_minutes":null}}
    ]);
    for request in requests.as_array().unwrap() {
        let parsed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), *request);
        let command =
            crate::runtime::command::KernelCommand::from_local_request("c", None, None, &parsed);
        assert_eq!(command.session_id, None);
    }
    let view = serde_json::json!({"view_id":"v","installation_id":"todo","generation":"7","origin":"https://app.a.invalid"});
    let frontend = serde_json::json!({"entry":"index.html","assets":[{"path":"index.html","content_type":"text/html","body_base64":"aGk="}],"content_security_policy":"default-src 'none'","iframe_sandbox":"allow-scripts"});
    let responses = serde_json::json!([
        {"UserAppViewOpened":{"view":view,"frontend":frontend}},
        {"UserAppViewsListed":{"views":[view]}},
        {"UserAppViewClosed":{"view_id":"v"}},
        {"UserAppViewFrontend":{"view":view,"frontend":frontend}},
        {"UserAppViewCallResult":{"result":{"ok":true},"error":null}},
        {"UserAppViewCallResult":{"result":null,"error":{"code":"APP_VIEW_STALE","message":"Reopen"}}},
        {"UserAppViewsChanged":{"cursor":5,"views":[view],"interactions":[]}},
        {"UserDomainInteractionAnswered":{"interaction_id":"decision"}}
    ]);
    for response in responses.as_array().unwrap() {
        let parsed: LocalDaemonResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *response);
    }
    let snapshot = serde_json::json!({"requests":requests,"responses":responses});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "94efdbdd0e2f83938757217d9e55d99f205904757b66a20c2308a4536bf7d25f"
    );
    for request in requests.as_array().unwrap() {
        let mut forged = request.clone();
        forged.as_object_mut().unwrap().values_mut().next().unwrap()["owner_id"] =
            serde_json::json!("victim");
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}

#[test]
fn detached_passkey_is_redacted_from_kernel_command_and_debug() {
    let request =
        LocalDaemonRequest::AnswerUserDomainInteraction(AnswerUserDomainInteractionRequest {
            interaction_id: "decision".into(),
            choice_id: "approve".into(),
            passkey: Some(ApprovalPasskey::new("synthetic-test-secret")),
            passkey_remember_minutes: None,
        });
    assert!(!format!("{request:?}").contains("synthetic-test-secret"));
    let command =
        crate::runtime::command::KernelCommand::from_local_request("c", None, None, &request);
    assert_eq!(
        command.payload["AnswerUserDomainInteraction"]["passkey"],
        "[redacted]"
    );
    assert!(!serde_json::to_string(&command)
        .unwrap()
        .contains("synthetic-test-secret"));
}
