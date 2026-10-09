use super::*;
use crate::local::{KernelConnectionClass, PasskeyPrompt, PasskeyPromptKind};
use crate::runtime::state::{critical_approval_audit_payload, PASSKEY_ALREADY_ANSWERED};
use crate::transport::kernel_protocol::KernelEvent;

#[test]
fn sudo_protocol_415_attributes_one_turn_to_its_human_entry() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let turn = crate::local::KernelSudoTurn {
        entry_id: "sudo:one".into(),
        session_id: "s".into(),
        agent_id: "a".into(),
        owner_user_id: "local".into(),
        terminal_id: "terminal".into(),
        requester: None,
        prompt_id: Some("turn-one".into()),
        provider_run_id: Some("run-one".into()),
    };
    let snapshot = serde_json::json!({"kind": PasskeyPromptKind::Sudo, "turn":turn,
        "receipt":crate::runtime::state::sudo_approval_receipt(&turn, "other", "critical", "approve")});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "8f52a7b4c7bdf054826de5653268cf097b60b1ce0ed8aee9da0ee4aec664d83d"
    );
    assert!(snapshot["turn"].get("expires_at_ms").is_none());
    assert!(snapshot["turn"].get("token").is_none());
}

#[test]
fn kernel_access_lifetime_config_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let response = LocalDaemonResponse::UserConfig {
        path: "/state/config.toml".into(),
        config: crate::config::CharioxUserConfig::default(),
    };
    let wire = serde_json::to_value(&response).unwrap();
    let lifetimes = &wire["UserConfig"]["config"]["kernel_access"];
    assert_eq!(
        lifetimes,
        &serde_json::json!({
            "grant_default_minutes": 30,
            "grant_max_minutes": 240,
            "grant_extend_notice_minutes": 5,
            "request_timeout_minutes": 10,
        })
    );
    let digest = Sha256::digest(serde_json::to_vec(lifetimes).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "d1286fb0a2b6dd753fa9691cdc9c1338b823fd1d0c8c8df115108edf30fbd012"
    );
}

/// Exhaustive, so a new class fails to compile here until it is versioned.
fn wire_name(class: KernelConnectionClass) -> &'static str {
    match class {
        KernelConnectionClass::Terminal => "terminal",
        KernelConnectionClass::ExternalAgent => "external_agent",
        KernelConnectionClass::KernelAgent => "kernel_agent",
        KernelConnectionClass::Host => "host",
        KernelConnectionClass::RelayPeer => "relay_peer",
        KernelConnectionClass::Unauthenticated => "unauthenticated",
    }
}

#[test]
fn kernel_connection_classes_and_their_audit_attribution_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let classes = [
        KernelConnectionClass::Terminal,
        KernelConnectionClass::ExternalAgent,
        KernelConnectionClass::KernelAgent,
        KernelConnectionClass::Host,
        KernelConnectionClass::RelayPeer,
        KernelConnectionClass::Unauthenticated,
    ];
    let vocabulary = serde_json::to_value(classes).unwrap();
    assert_eq!(
        vocabulary,
        serde_json::json!(classes.map(wire_name)),
        "the wire vocabulary is fixed"
    );
    for class in classes {
        assert_eq!(
            serde_json::from_value::<KernelConnectionClass>(serde_json::json!(wire_name(class)))
                .unwrap(),
            class
        );
    }
    // Protocol 412: only admitted terminals may submit a passkey.
    assert_eq!(
        classes.map(KernelConnectionClass::may_submit_passkey),
        [true, false, false, false, false, false]
    );

    let audit = critical_approval_audit_payload(
        "local",
        "decision",
        "verified",
        Some(5),
        Some(KernelConnectionClass::Terminal),
    );
    assert_eq!(
        audit,
        serde_json::json!({
            "owner": "local", "interaction_id": "decision", "outcome": "verified",
            "remember_minutes": 5, "connection_class": "terminal",
        })
    );
    // The kernel's own callers have no connection, so no class.
    let internal = critical_approval_audit_payload("local", "decision", "missing", None, None);
    assert!(internal["connection_class"].is_null());

    let snapshot = serde_json::json!({"classes": vocabulary, "audit": audit, "internal": internal});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "947eca667500d33980960f533e1e35e66ff9dc57c50189afcc1cc8671976bfa4"
    );
}

#[test]
fn passkey_prompts_and_their_popup_event_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let prompt = |session_alias: Option<&str>, interaction_id: &str| PasskeyPrompt {
        kind: PasskeyPromptKind::CriticalApproval,
        session_id: "session-1".into(),
        session_alias: session_alias.map(str::to_owned),
        interaction_id: interaction_id.into(),
        title: "Approve App action".into(),
        message: "An App asks to perform a protected action.".into(),
        approve_choice_id: "approve".into(),
        refuse_choice_id: "deny".into(),
        requested_at_ms: 1_000,
        expires_at_ms: 301_000,
        lifetime_minutes: None,
        max_lifetime_minutes: None,
    };
    let event = KernelEvent::PasskeyPromptsChanged {
        prompts: vec![
            prompt(Some("Payments"), "app_validation_op-1"),
            prompt(None, "app_validation_op-2"),
        ],
    };
    let wire = serde_json::to_value(&event).unwrap();
    let expected = |alias: Option<&str>, interaction_id: &str| {
        let mut value = serde_json::json!({
            "kind": "critical_approval",
            "session_id": "session-1",
            "interaction_id": interaction_id,
            "title": "Approve App action",
            "message": "An App asks to perform a protected action.",
            "approve_choice_id": "approve",
            "refuse_choice_id": "deny",
            "requested_at_ms": 1_000,
            "expires_at_ms": 301_000,
        });
        if let Some(alias) = alias {
            value["session_alias"] = serde_json::json!(alias);
        }
        value
    };
    assert_eq!(
        wire,
        serde_json::json!({
            "event": "passkey_prompts_changed",
            "prompts": [
                expected(Some("Payments"), "app_validation_op-1"),
                expected(None, "app_validation_op-2"),
            ],
        })
    );
    assert_eq!(
        serde_json::from_value::<KernelEvent>(wire.clone()).unwrap(),
        event
    );
    // An empty set closes every popup.
    let closed =
        serde_json::to_value(KernelEvent::PasskeyPromptsChanged { prompts: vec![] }).unwrap();
    assert_eq!(
        closed,
        serde_json::json!({"event": "passkey_prompts_changed", "prompts": []})
    );
    // A later answer to an answered prompt is refused with this code.
    assert_eq!(PASSKEY_ALREADY_ANSWERED, "PASSKEY_ALREADY_ANSWERED");

    let snapshot = serde_json::json!({
        "event": wire,
        "closed": closed,
        "already_answered": PASSKEY_ALREADY_ANSWERED,
    });
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "eb4f707985865d80b8ddc73fcf3f95534c4b6a99c30de7fbe666da5830159350"
    );
}

#[test]
fn process_bound_access_protocol_404_has_metadata_but_no_bearer() {
    use crate::local::{
        KernelAccessGrant, ListKernelAccessGrantsRequest, RequestKernelAccessRequest,
        RevokeKernelAccessGrantRequest,
    };
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let grant = KernelAccessGrant {
        grant_id: "g".into(),
        session_id: "s".into(),
        owner_user_id: "local".into(),
        holder_pid: 42,
        holder_executable: "/usr/bin/agent".into(),
        lifetime_minutes: 30,
        expires_at_ms: 1_801_000,
    };
    let requests = [
        LocalDaemonRequest::RequestKernelAccess(RequestKernelAccessRequest {
            session_id: "s".into(),
            holder_pid: 42,
            lifetime_minutes: Some(30),
        }),
        LocalDaemonRequest::ListKernelAccessGrants(ListKernelAccessGrantsRequest {}),
        LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: Some("g".into()),
        }),
        LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: None,
        }),
    ];
    let responses = [
        LocalDaemonResponse::KernelAccessGranted {
            grant: grant.clone(),
        },
        LocalDaemonResponse::KernelAccessGrantsListed {
            grants: vec![grant],
            sudo_turns: vec![],
        },
        LocalDaemonResponse::KernelAccessRevoked { revoked: 1 },
    ];
    for request in &requests {
        assert_eq!(
            serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(request).unwrap())
                .unwrap(),
            *request
        );
    }
    for response in &responses {
        assert_eq!(
            serde_json::from_value::<LocalDaemonResponse>(serde_json::to_value(response).unwrap())
                .unwrap(),
            *response
        );
    }
    let snapshot = serde_json::json!({ "requests": requests, "responses": responses,
        "kinds": [PasskeyPromptKind::AccessGrant, PasskeyPromptKind::AccessExtension] });
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "328c2b79aca163f32a42062d5fa47cd81221f9134052e286c96e051b6a7b09f0"
    );
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"RequestKernelAccess": {
            "session_id": "s", "holder_pid": 42, "bearer_token": "forbidden"
        }})
    )
    .is_err());
}

#[test]
fn external_sudo_protocol_415_is_versioned_and_accepts_no_credentials() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let request = LocalDaemonRequest::RequestKernelSudo(crate::local::RequestKernelSudoRequest {
        agent_id: "a".into(),
        prompt: "full\nprompt".into(),
    });
    let response = LocalDaemonResponse::KernelSudoRequested {
        agent_id: "a".into(),
    };
    let snapshot = serde_json::json!({"request":request, "response":response});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "f29d8b3bee3e4796114213933a289d0f7cb10bdc95d937128eeba3bc44243f09"
    );
    for field in ["passkey", "token", "holder_pid", "session_id"] {
        let mut forged = serde_json::json!({"agent_id":"a", "prompt":"task"});
        forged[field] = serde_json::json!("forged");
        assert!(serde_json::from_value::<crate::local::RequestKernelSudoRequest>(forged).is_err());
    }
}

#[test]
fn external_sudo_requester_and_host_terminal_attribution_are_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 455);
    let turn: crate::local::KernelSudoTurn = serde_json::from_value(serde_json::json!({"entry_id":"sudo:external","session_id":"s","agent_id":"a","owner_user_id":"local","terminal_id":"host-terminal","prompt_id":"prompt","provider_run_id":"run","requester":{"grant_id":"grant","session_id":"s","owner_user_id":"local","holder_pid":123,"holder_executable":"/fixture/external","lifetime_minutes":30,"expires_at_ms":123456}})).unwrap();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&serde_json::to_value(&turn).unwrap()).unwrap())
        ),
        "03d835d6424136bce70c39991d068f8664f8c32d05427c01cb779e010fc5b021"
    );
}
