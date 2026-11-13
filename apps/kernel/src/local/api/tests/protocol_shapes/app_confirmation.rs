use super::*;

// APP-CONFIRM-01, local 1006 -> next: a person-confirmed App action is an `app-confirm-*` agent card that carries
// the kernel's own App and publisher frame, and its signed declaration names the compact view that previews the input.
#[test]
fn app_confirmation_shapes_are_pinned() {
    const { assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 1006) };
    let frame = crate::local::AppConfirmationFrame {
        installation_id: "campaign".into(),
        app_id: "com.chariox.campaign".into(),
        app_name: "Campaign".into(),
        publisher_id: "com.chariox".into(),
        publisher_name: "Chariox".into(),
        action_name: "pay".into(),
    };
    let interaction = crate::session::RuntimeInteraction::new(
        "app-confirm-1",
        "agent",
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Warning,
        Some("Confirm pay".into()),
        "Campaign by Chariox asks you to confirm pay.\nInput: {\"amount\":1200}",
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "confirm",
                "Confirm",
                "confirm",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        Some(300),
        Some("cancel".into()),
    )
    .with_app_confirmation(frame);
    let mut wire = serde_json::to_value(interaction).unwrap();
    wire["requested_at_ms"] = serde_json::json!(1);
    assert_eq!(
        wire["app_confirmation"],
        serde_json::json!({
            "installation_id": "campaign", "app_id": "com.chariox.campaign", "app_name": "Campaign",
            "publisher_id": "com.chariox", "publisher_name": "Chariox", "action_name": "pay"
        })
    );
    // The frame is the kernel's whole statement: a partial or extended frame is refused.
    for refused in [
        serde_json::json!({"installation_id": "campaign", "app_name": "Campaign"}),
        serde_json::json!({
            "installation_id": "campaign", "app_id": "a", "app_name": "A", "publisher_id": "p",
            "publisher_name": "P", "action_name": "pay", "icon": "x"
        }),
    ] {
        assert!(serde_json::from_value::<crate::local::AppConfirmationFrame>(refused).is_err());
    }
    let action: chariox_app_package::v2::UiActionDeclaration =
        serde_json::from_value(serde_json::json!({
            "name": "pay", "description": "Pay for the campaign", "inputSchema": {"type": "object"},
            "effect": "person_confirmed", "confirmationView": "review_payment"
        }))
        .unwrap();
    let action = serde_json::to_value(action).unwrap();
    assert_eq!(action["effect"], "person_confirmed");
    let snapshot = serde_json::json!({"action": action, "interaction": wire});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "6727ef315b0ddb737c86cfe86e676c082e0d8d919fcd60804e12af9393f8aa08"
    );
}
