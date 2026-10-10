//! MP-08 / MP-10 / MP-11 A07: local 477 protected owner hand-off shapes.
use super::*;
use crate::local::{
    HandoffOutcome, HandoffResponseAction, HandoffStatus, HandoffValue, RespondToHandoffRequest,
};
use crate::session::{
    HandoffChangeLine, HandoffChangeOp, HandoffKind, HandoffReason, HandoffTarget, RuntimeHandoff,
    RuntimeInteraction, RuntimeInteractionChoice,
};

fn handoff() -> RuntimeHandoff {
    RuntimeHandoff {
        kind: HandoffKind::Secret,
        reason: HandoffReason::AutomationDisallowed,
        agent_id: "agent".into(),
        task_id: "task".into(),
        obligation_id: "obligation".into(),
        explanation: "The site disallows automated sign-in".into(),
        target: HandoffTarget {
            tab_id: "tab".into(),
            generation: 2,
            document_id: "doc".into(),
            node_ref: "backend:9".into(),
            origin: "https://example.test".into(),
            path: "/login".into(),
            label: "Password".into(),
        },
        change: vec![HandoffChangeLine {
            op: HandoffChangeOp::Add,
            text: "signed in".into(),
        }],
        expires_at_ms: 900_000,
        save_to_vault_offered: true,
    }
}

#[test]
fn mp08_mp10_mp11_owner_handoff_protocol_477_snapshot_and_hash() {
    use sha2::{Digest, Sha256};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 477);
    let interaction = RuntimeInteraction::for_kernel_operation(
        "handoff-obligation",
        "handoff-obligation",
        "Agent hand-off",
        "Enter the secret",
        vec![
            RuntimeInteractionChoice::new("done", "Done in browser", "done", None),
            RuntimeInteractionChoice::new("cancel", "Cancel", "cancel", None),
        ],
    )
    .with_timeout_sec(900)
    .with_handoff(handoff());
    let mut projected = serde_json::to_value(&interaction).unwrap();
    projected["requested_at_ms"] = serde_json::json!(1);
    let request = LocalDaemonRequest::RespondToHandoff(RespondToHandoffRequest {
        session_id: "room".into(),
        interaction_id: "handoff-obligation".into(),
        action: HandoffResponseAction::EnterValue {
            value: HandoffValue::new("fixture-value"),
            save_to_vault_key: Some("example-login".into()),
        },
    });
    let response = LocalDaemonResponse::HandoffResolved {
        outcome: HandoffOutcome {
            handoff_id: "handoff-obligation".into(),
            status: HandoffStatus::Completed,
            action: "enter_value".into(),
            reason_code: None,
            saved_to_vault: true,
        },
    };
    let actions = ["click", "done", "cancel"].map(|kind| {
        serde_json::to_value(
            serde_json::from_value::<HandoffResponseAction>(serde_json::json!({"kind":kind}))
                .unwrap(),
        )
        .unwrap()
    });
    let snapshot = serde_json::json!({"interaction":projected,"request":request,"response":response,"actions":actions});
    assert_eq!(
        snapshot["interaction"]["handoff"]["target"]["origin"],
        "https://example.test"
    );
    assert!(
        format!("{request:?}").contains("[REDACTED]")
            && !format!("{request:?}").contains("fixture-value"),
        "the protected value never appears in debug output"
    );
    assert!(serde_json::from_value::<HandoffResponseAction>(
        serde_json::json!({"kind":"done","custom_reply":"x"})
    )
    .is_err());
    assert_eq!(
        format!("{:x}", Sha256::digest(snapshot.to_string().as_bytes())),
        "a6259cfa0565785778583aa0ffdd4adf03585d0baad1f2901e88bb1a8c953193"
    );
}
