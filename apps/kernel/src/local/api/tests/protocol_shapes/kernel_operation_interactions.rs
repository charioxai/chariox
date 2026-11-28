use super::*;

#[test]
fn kernel_operation_interaction_subject_is_versioned_and_uses_the_existing_reply() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
        "decision",
        "operation",
        "Install App?",
        "Reviewed release",
        vec![
            crate::session::RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
            // Protocol 392: a critical approval needs the passkey.
            crate::session::RuntimeInteractionChoice::new("approve", "Approve", "allow", None)
                .requiring_passkey(),
        ],
    )
    .with_card_type("app_install");
    let mut wire = serde_json::to_value(interaction).unwrap();
    wire["requested_at_ms"] = serde_json::json!(1);
    assert_eq!(
        wire,
        serde_json::json!({"surface":"card", "card_type":"app_install",
            "id":"decision", "kernel_operation_id":"operation", "kind":"permission", "level":"warning",
            "title":"Install App?", "message":"Reviewed release", "choices":[{"id":"deny","label":"Cancel","reply":"deny"},{"id":"approve","label":"Approve","reply":"allow","requires_passkey":true}],
            "timeout_sec":300, "requested_at_ms":1,
        })
    );
    let request =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: "session".into(),
            interaction_id: "decision".into(),
            choice_id: "approve".into(),
            custom_reply: None,
            passkey: Some(crate::local::ApprovalPasskey::new("passkey")),
            passkey_remember_minutes: Some(5),
        });
    assert!(!format!("{request:?}").contains("\"passkey\""));
    assert_eq!(
        serde_json::to_value(&request).unwrap()["RespondToInteraction"]["passkey_remember_minutes"],
        5
    );
    let snapshot =
        serde_json::json!({"interaction":wire,"request":serde_json::to_value(request).unwrap()});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "cc79a6a9f33a7e5883fdce7bc106cae0476723312cd6264ea4c176f13ca4a568"
    );
}

#[test]
fn an_interaction_answer_is_acknowledged_without_the_room_state() {
    let ack = LocalDaemonResponse::InteractionResponded {
        interaction_id: "decision".into(),
        choice_id: "approve".into(),
        outcome: crate::local::InteractionOutcome::Answered,
    };
    let wire = serde_json::json!({"InteractionResponded":{
        "interaction_id":"decision","choice_id":"approve","outcome":"answered",
    }});
    assert_eq!(serde_json::to_value(&ack).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(wire).unwrap(),
        ack
    );
    // The reply that carried the whole session is gone: it no longer parses.
    assert!(
        serde_json::from_value::<LocalDaemonResponse>(serde_json::json!({
            "InteractionResponded":{"interaction_id":"decision","session":{}}
        }))
        .is_err()
    );
}
