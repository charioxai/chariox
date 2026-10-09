use super::*;
use crate::local::{KernelConnectionClass, PasskeyPrompt, PasskeyPromptKind};
use crate::runtime::state::{critical_approval_audit_payload, PASSKEY_ALREADY_ANSWERED};
use crate::transport::kernel_protocol::KernelEvent;

#[test]
fn sudo_protocol_415_attributes_one_turn_to_its_human_entry() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let response = LocalDaemonResponse::UserConfig {
        path: "/state/config.toml".into(),
        config: crate::config::CharioxUserConfig::default(),
    };
    let wire = serde_json::to_value(&response).unwrap();
    let lifetimes = &wire["UserConfig"]["config"]["kernel_access"];
    assert_eq!(
        lifetimes,
        &serde_json::json!({
            "grant_default_minutes": 480,
            "grant_max_minutes": 1440,
            "grant_extend_notice_minutes": 5,
            "request_timeout_minutes": 10,
        })
    );
    let digest = Sha256::digest(serde_json::to_vec(lifetimes).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "f8aaa273db488ebb9318507d59428ede59b5a75e8b2bb560ca2122a556a6c369"
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let prompt = |session_alias: Option<&str>, interaction_id: &str| PasskeyPrompt {
        kind: PasskeyPromptKind::CriticalApproval,
        requester: None,
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
fn local_kernel_access_protocol_451_has_no_session_scope_or_bearer() {
    use crate::local::{
        KernelAccessGrant, ListKernelAccessGrantsRequest, RequestKernelAccessRequest,
        RevokeKernelAccessGrantRequest,
    };
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let grant = KernelAccessGrant {
        grant_id: "g".into(),
        owner_user_id: "local".into(),
        holder_pid: 42,
        holder_executable: "/usr/bin/agent".into(),
        lifetime_minutes: 30,
        expires_at_ms: 1_801_000,
    };
    let requests = [
        LocalDaemonRequest::RequestKernelAccess(RequestKernelAccessRequest {
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
        LocalDaemonResponse::KernelAccessDecisionResponded {
            interaction_id: "g-grant".into(),
        },
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
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"RequestKernelAccess": {
            "holder_pid": 42, "session_id": "obsolete"
        }}))
        .is_err(),
        "451 rejects session-scoped clients"
    );
    let snapshot = serde_json::json!({ "requests": requests, "responses": responses,
        "kinds": [PasskeyPromptKind::AccessGrant, PasskeyPromptKind::AccessExtension] });
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "2e0ccb5c1c904a48d996076f62a500b3dcdab528ab6f1f8c9bbabc275c90f2c1"
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
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
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let turn: crate::local::KernelSudoTurn = serde_json::from_value(serde_json::json!({"entry_id":"sudo:external","session_id":"s","agent_id":"a","owner_user_id":"local","terminal_id":"host-terminal","prompt_id":"prompt","provider_run_id":"run","requester":{"grant_id":"grant","owner_user_id":"local","holder_pid":123,"holder_executable":"/fixture/external","lifetime_minutes":30,"expires_at_ms":123456}})).unwrap();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&serde_json::to_value(&turn).unwrap()).unwrap())
        ),
        "1cd52d9eafdcad1f6c0e3becedbf9654e87fc4bb3805149b9f9eca72b83a25e7"
    );
}

// MP-08 / MP-10 / MP-11: shared interaction/event wire identity, including u64 precision.
#[test]
fn access_requester_protocol_470_shape_and_hash() {
    use crate::local::{KernelAccessProviderHarness, KernelAccessRequester};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let requester: KernelAccessRequester = serde_json::from_value(serde_json::json!({"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"})).unwrap();
    let interaction: crate::session::RuntimeInteraction = serde_json::from_value(serde_json::json!({"id": "g-grant", "kernel_operation_id": "access-grant:g", "kind": "permission", "level": "warning", "title": "Grant external agent access", "message": "Display only", "choices": [{"id": "refuse", "label": "Refuse", "reply": "refuse"}, {"id": "approve", "label": "Approve", "reply": "approve", "requires_passkey": true}], "timeout_sec": 300, "requested_at_ms": 1000, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    assert_eq!(interaction.requester(), Some(&requester));
    let grant: PasskeyPrompt = serde_json::from_value(serde_json::json!({"kind": "access_grant", "session_id": "kernel-access", "interaction_id": "g-grant", "title": "External access", "message": "Display only", "approve_choice_id": "approve", "refuse_choice_id": "refuse", "requested_at_ms": 1000, "expires_at_ms": 301000, "lifetime_minutes": 30, "max_lifetime_minutes": 1440, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    let extension: PasskeyPrompt = serde_json::from_value(serde_json::json!({"kind": "access_extension", "session_id": "kernel-access", "interaction_id": "g-extension", "title": "External access", "message": "Display only", "approve_choice_id": "approve", "refuse_choice_id": "refuse", "requested_at_ms": 1000, "expires_at_ms": 301000, "lifetime_minutes": 30, "max_lifetime_minutes": 1440, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    let event = KernelEvent::PasskeyPromptsChanged {
        prompts: vec![grant, extension],
    };
    let snapshot = serde_json::json!({"interaction":interaction,"event":event,"harnesses":[KernelAccessProviderHarness::Codex, KernelAccessProviderHarness::Claude, KernelAccessProviderHarness::Opencode]});
    assert_eq!(
        snapshot["event"]["prompts"][0]["requester"],
        snapshot["interaction"]["requester"]
    );
    assert_eq!(
        snapshot["event"]["prompts"][0]["requester"],
        snapshot["event"]["prompts"][1]["requester"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "7b666b1bc31a3d6dbdcb41e9cec18f97dd98e6bf93b6bddba1ed1b27ecc43ef2"
    );
    assert_eq!(
        serde_json::from_value::<KernelEvent>(serde_json::to_value(&event).unwrap()).unwrap(),
        event
    );
    // An old event stays compatible but supplies no trusted requester identity.
    let mut old = snapshot["event"]["prompts"][0].clone();
    old.as_object_mut().unwrap().remove("requester");
    assert!(serde_json::from_value::<PasskeyPrompt>(old)
        .unwrap()
        .requester
        .is_none());
    // The requester is output-only and cannot be forged by the Unix caller.
    assert!(
        serde_json::from_value::<crate::local::RequestKernelAccessRequest>(
            serde_json::json!({"holder_pid":42,"requester":requester})
        )
        .is_err()
    );
}
