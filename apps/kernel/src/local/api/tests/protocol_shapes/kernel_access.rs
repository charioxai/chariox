use super::*;
use crate::local::{KernelConnectionClass, PasskeyPrompt, PasskeyPromptKind};
use crate::runtime::state::{critical_approval_audit_payload, PASSKEY_ALREADY_ANSWERED};
use crate::transport::kernel_protocol::KernelEvent;

#[test]
fn sudo_protocol_487_window_extension_and_session_status_shapes() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let turn = crate::local::KernelSudoTurn {
        entry_id: "sudo:one".into(),
        session_id: "s".into(),
        agent_id: "a".into(),
        owner_user_id: "local".into(),
        terminal_id: "terminal".into(),
        requester: None,
        prompt_id: Some("turn-one".into()),
        provider_run_id: Some("run-one".into()),
        task_id: Some("sudo:one".into()),
        duration_minutes: 120,
        expires_at_ms: Some(7_200_000),
        revision: 2,
        warning_sent: false,
        deadline: Some(std::time::Instant::now()),
        placement: None,
    };
    let extend = LocalDaemonRequest::ExtendKernelSudo(crate::local::ExtendKernelSudoRequest {
        session_id: "s".into(),
        attachment_id: "terminal".into(),
        entry_id: "sudo:one".into(),
        revision: 2,
    });
    let mut session = crate::session::RuntimeSession::new(
        "s",
        None,
        "workspace",
        "worktree",
        "machine",
        "daemon",
    );
    session.set_sudo_windows(vec![turn.clone()]);
    let snapshot = serde_json::json!({"kind": PasskeyPromptKind::Sudo, "turn": turn, "extend": extend,
        "extended": LocalDaemonResponse::KernelSudoExtended { turn: turn.clone() },
        "sudo_windows": serde_json::to_value(&session).unwrap()["sudo_windows"]});
    // The monotonic deadline is kernel memory only and never serialized.
    assert!(snapshot["turn"].get("deadline").is_none());
    assert!(snapshot["turn"].get("token").is_none());
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "256bc3e160c1813a1255ad56fbf2ab952598b25e81d9e7db13933eaec552f956"
    );
}

#[test]
fn kernel_access_lifetime_config_is_versioned() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let prompt = |session_alias: Option<&str>, interaction_id: &str| PasskeyPrompt {
        kind: PasskeyPromptKind::CriticalApproval,
        sudo_request: None,
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
        term: None,
        lifetime_minutes: None,
        max_lifetime_minutes: None,
    };
    let event = KernelEvent::PasskeyPromptsChanged {
        kernel_now_ms: 1234,
        closed: vec![],
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
            "event": "passkey_prompts_changed", "kernel_now_ms": 1234, "closed": [],
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
    let closed = serde_json::to_value(KernelEvent::PasskeyPromptsChanged {
        prompts: vec![],
        kernel_now_ms: 1234,
        closed: vec![],
    })
    .unwrap();
    assert_eq!(
        closed,
        serde_json::json!({"event": "passkey_prompts_changed", "prompts": [], "kernel_now_ms": 1234, "closed": []})
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
        "d764a65dbd4879191cd96b35887b4aa69859628ab4363ff95413ae3fc28761b3"
    );
}

#[test]
fn local_kernel_access_protocol_451_has_no_session_scope_or_bearer() {
    use crate::local::{
        KernelAccessGrant, ListKernelAccessGrantsRequest, RequestKernelAccessRequest,
        RevokeKernelAccessGrantRequest,
    };
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let grant = KernelAccessGrant {
        grant_id: "g".into(),
        owner_user_id: "local".into(),
        holder_pid: 42,
        holder_executable: "/usr/bin/agent".into(),
        holder_harness: Some(crate::local::KernelAccessProviderHarness::Claude),
        term: crate::local::KernelAccessTerm::Minutes(30),
        expires_at_ms: Some(1_801_000),
    };
    // A holder the kernel cannot attribute to a provider harness says nothing about it.
    assert_eq!(
        serde_json::to_value(&grant).unwrap()["holder_harness"],
        "claude"
    );
    assert!(serde_json::to_value(KernelAccessGrant {
        holder_harness: None,
        ..grant.clone()
    })
    .unwrap()
    .get("holder_harness")
    .is_none());
    let until_exit_grant = KernelAccessGrant {
        grant_id: "g2".into(),
        term: crate::local::KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
        ..grant.clone()
    };
    let requests = [
        LocalDaemonRequest::RequestKernelAccess(RequestKernelAccessRequest {
            holder_pid: 42,
            term: Some(crate::local::KernelAccessTerm::Minutes(30)),
        }),
        LocalDaemonRequest::RequestKernelAccess(RequestKernelAccessRequest {
            holder_pid: 42,
            term: Some(crate::local::KernelAccessTerm::UntilHolderExits),
        }),
        LocalDaemonRequest::RequestKernelAccess(RequestKernelAccessRequest {
            holder_pid: 42,
            term: None,
        }),
        LocalDaemonRequest::ListKernelAccessGrants(ListKernelAccessGrantsRequest {}),
        LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: Some("g".into()),
        }),
        LocalDaemonRequest::RevokeKernelAccessGrant(RevokeKernelAccessGrantRequest {
            grant_id: None,
        }),
        LocalDaemonRequest::RenewKernelSudoRequester(
            crate::local::RenewKernelSudoRequesterRequest {},
        ),
    ];
    let responses = [
        LocalDaemonResponse::KernelAccessGranted {
            grant: grant.clone(),
        },
        LocalDaemonResponse::KernelAccessGrantsListed {
            grants: vec![grant, until_exit_grant],
            sudo_turns: vec![],
        },
        LocalDaemonResponse::KernelAccessRevoked { revoked: 1 },
        LocalDaemonResponse::KernelSudoRequesterRenewed { renewed: true },
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
    assert_eq!(
        serde_json::to_value(LocalDaemonRequest::RenewKernelSudoRequester(
            crate::local::RenewKernelSudoRequesterRequest {}
        ))
        .unwrap(),
        serde_json::json!({"RenewKernelSudoRequester": {}})
    );
    // 568 replaced the lifetime by a term; the old field is refused, not ignored.
    assert!(
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({"RequestKernelAccess": {
            "holder_pid": 42, "lifetime_minutes": 30
        }}))
        .is_err(),
        "568 rejects the removed lifetime_minutes"
    );
    // Wire spelling of the term, and of the answer that chooses it.
    assert_eq!(
        serde_json::to_value(crate::local::KernelAccessTerm::Minutes(30)).unwrap(),
        serde_json::json!({"minutes": 30})
    );
    assert_eq!(
        serde_json::to_value(crate::local::KernelAccessTerm::UntilHolderExits).unwrap(),
        serde_json::json!("until_holder_exits")
    );
    assert_eq!(
        crate::local::KernelAccessTerm::from_reply("until_exit"),
        Some(crate::local::KernelAccessTerm::UntilHolderExits)
    );
    assert_eq!(
        crate::local::KernelAccessTerm::from_reply("45"),
        Some(crate::local::KernelAccessTerm::Minutes(45))
    );
    assert_eq!(crate::local::KernelAccessTerm::from_reply("soon"), None);
    let snapshot = serde_json::json!({ "requests": requests, "responses": responses,
        "kinds": [PasskeyPromptKind::AccessGrant, PasskeyPromptKind::AccessExtension] });
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "546509802affc87d8240d5c2bf3154d4f783ba5c049afc57af16a095a4f36619"
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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let turn: crate::local::KernelSudoTurn = serde_json::from_value(serde_json::json!({"entry_id":"sudo:external","session_id":"s","agent_id":"a","owner_user_id":"local","terminal_id":"host-terminal","prompt_id":"prompt","provider_run_id":"run","requester":{"grant_id":"grant","owner_user_id":"local","holder_pid":123,"holder_executable":"/fixture/external","term":{"minutes":30},"expires_at_ms":123456}})).unwrap();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&serde_json::to_value(&turn).unwrap()).unwrap())
        ),
        "fa17f728b3b00e7af2382cab1ed18696367b397f0c1fcb0eadcdd91c55163d4c"
    );
}

/// MP-08/MP-09/MP-10/MP-11 A10: the home's leased sudo fence is one peer
/// request bound to relay peer protocol 83; its wire carries no credential.
#[test]
fn leased_sudo_fence_is_bound_to_relay_peer_protocol_83() {
    use crate::transport::relay_peer::*;
    assert_eq!(
        RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    assert_eq!(
        crate::runtime::state::LEASED_SUDO_PEER_PROTOCOL_VERSION,
        RELAY_PEER_PROTOCOL_VERSION
    );
    let request = RelayPeerRequest::UpdateLeasedSudo {
        leased_agent_id: "leased-1".into(),
        home_prompt_id: "prompt-1".into(),
        grant: LeasedSudoGrant {
            entry_id: "sudo:0123".into(),
            revision: 2,
            remaining_ms: 3_600_000,
            initial: true,
        },
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({"kind":"update_leased_sudo",
            "leased_agent_id":"leased-1","home_prompt_id":"prompt-1",
            "grant":{"entry_id":"sudo:0123","revision":2,"remaining_ms":3_600_000,"initial":true}
        })
    );
    assert_eq!(
        serde_json::to_value(RelayPeerResponse::LeasedSudoUpdated).unwrap(),
        serde_json::json!({"kind":"leased_sudo_updated"})
    );
}

// MP-08 / MP-10 / MP-11: shared interaction/event wire identity, including u64 precision.
#[test]
fn access_requester_protocol_470_shape_and_hash() {
    use crate::local::{KernelAccessProviderHarness, KernelAccessRequester};
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requester: KernelAccessRequester = serde_json::from_value(serde_json::json!({"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"})).unwrap();
    let interaction: crate::session::RuntimeInteraction = serde_json::from_value(serde_json::json!({"id": "g-grant", "kernel_operation_id": "access-grant:g", "kind": "permission", "level": "warning", "title": "Grant external agent access", "message": "Display only", "choices": [{"id": "refuse", "label": "Refuse", "reply": "refuse"}, {"id": "approve", "label": "Approve", "reply": "approve", "requires_passkey": true}], "timeout_sec": 300, "requested_at_ms": 1000, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    assert_eq!(interaction.requester(), Some(&requester));
    let grant: PasskeyPrompt = serde_json::from_value(serde_json::json!({"kind": "access_grant", "session_id": "kernel-access", "interaction_id": "g-grant", "title": "External access", "message": "Display only", "approve_choice_id": "approve", "refuse_choice_id": "refuse", "requested_at_ms": 1000, "expires_at_ms": 301000, "term": {"minutes": 30}, "max_lifetime_minutes": 1440, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    let extension: PasskeyPrompt = serde_json::from_value(serde_json::json!({"kind": "access_extension", "session_id": "kernel-access", "interaction_id": "g-extension", "title": "External access", "message": "Display only", "approve_choice_id": "approve", "refuse_choice_id": "refuse", "requested_at_ms": 1000, "expires_at_ms": 301000, "term": {"minutes": 30}, "max_lifetime_minutes": 1440, "requester": {"executable": "/opt/codex", "pid": 42, "process_start_id": "18446744073709551615", "process_exec_version": 7, "provider_harness": "codex"}})).unwrap();
    let event = KernelEvent::PasskeyPromptsChanged {
        kernel_now_ms: 1234,
        closed: vec![],
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
        "7a8f7a3731b82ec085248149a6e969e6aa39652841a0aa34dee5a679186c6c67"
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

// MP-08 / MP-10 / MP-11: coordinator-allocated lifecycle and sudo presentation contract.
#[test]
fn sudo_presentation_and_authoritative_closure_protocol_538_shape_and_hash() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    assert_eq!(
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        crate::test_support::expected_relay_peer_protocol_version()
    );
    let snapshot = serde_json::json!({"sudo_request":{"requester_name":"Claude Code","agent_name":"Reviewer","session_name":"Payments","task_summary":"Review the release","task":"Review the release\nFull task","grant_id":"g","terminal_id":"t"},"kernel_now_ms":4001,"closed":[{"session_id":"s","interaction_id":"i","kind":"sudo","expires_at_ms":4000,"reason":"expired"},{"session_id":"s","interaction_id":"i","kind":"sudo","expires_at_ms":4000,"reason":"answered"},{"session_id":"s","interaction_id":"i","kind":"sudo","expires_at_ms":4000,"reason":"revoked"}]});
    let presentation: crate::local::SudoRequestPresentation =
        serde_json::from_value(snapshot["sudo_request"].clone()).unwrap();
    let closures: Vec<crate::local::PasskeyPromptClosure> =
        serde_json::from_value(snapshot["closed"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(presentation).unwrap(),
        snapshot["sudo_request"]
    );
    assert_eq!(serde_json::to_value(closures).unwrap(), snapshot["closed"]);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "4a8c957ea27a395af0494a6d25ce19333f4eb9804d63991e88b60acef08dbd3b"
    );
}
