//! MP-08/MP-10/MP-11 ephemeral login interaction and its existing leased bridge.
use super::*;

#[test]
fn mp08_mp10_mp11_provider_login_projection_shape_is_versioned() {
    use crate::session::*;
    use crate::transport::relay_peer::*;
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    assert_eq!(RELAY_PEER_PROTOCOL_VERSION, 74);
    let projection = RuntimeProviderLogin {
        kernel_id: "worker".into(),
        terminal_output_base64: "Zml4dHVyZQ==".into(),
        login: crate::provider::ProviderLoginStart {
            provider: "codex".into(),
            account_profile: "work".into(),
            login_kind: "chatgptDeviceCode".into(),
            login_id: Some("synthetic-login".into()),
            auth_url: None,
            verification_url: Some("http://127.0.0.1/device".into()),
            user_code: Some("SYNTHETIC".into()),
        },
    };
    let interaction = RuntimeInteraction::new(
        "provider-auth-recovery:run:login",
        "agent",
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Log in to Codex on this machine".into()),
        "Complete official login",
        vec![RuntimeInteractionChoice::new(
            "cancel",
            "Cancel login",
            "cancel",
            None,
        )],
        None,
        Some(600),
        None,
    )
    .with_provider_login(projection.clone());
    let mut snapshot = serde_json::to_value(interaction).unwrap();
    snapshot["requested_at_ms"] = serde_json::json!(1000);
    assert_eq!(
        snapshot,
        serde_json::json!({
            "id":"provider-auth-recovery:run:login", "agent_id":"agent", "kind":"choice", "level":"warning",
            "title":"Log in to Codex on this machine", "message":"Complete official login",
            "choices":[{"id":"cancel","label":"Cancel login","reply":"cancel"}], "timeout_sec":600, "requested_at_ms":1000,
            "provider_login": {"kernel_id":"worker","terminal_output_base64":"Zml4dHVyZQ==",
                "login":{"provider":"codex","account_profile":"work","login_kind":"chatgptDeviceCode",
                    "login_id":"synthetic-login","auth_url":null,"verification_url":"http://127.0.0.1/device","user_code":"SYNTHETIC"}}
        })
    );
    let request = RelayPeerRequest::UpdateNativeInteraction {
        context: RemoteNativeInteractionContext {
            home_session_id: "session".into(),
            home_agent_id: "agent".into(),
            leased_agent_id: "leased".into(),
            worker_provider_run_id: "run".into(),
            home_prompt_id: None,
        },
        interaction_id: "provider-auth-recovery:run:login".into(),
        login: None,
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"kind":"update_native_interaction",
            "context":{"home_session_id":"session","home_agent_id":"agent",
                "leased_agent_id":"leased","worker_provider_run_id":"run"},
            "interaction_id":"provider-auth-recovery:run:login", "login":null
        })
    );
    assert_eq!(
        serde_json::to_value(RelayPeerResponse::NativeInteractionUpdated {}).unwrap(),
        serde_json::json!({"kind":"native_interaction_updated"})
    );
}
