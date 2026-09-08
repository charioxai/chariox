use super::*;

#[test]
fn kernel_operation_interaction_subject_is_versioned_and_uses_the_existing_reply() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 295);
    let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
        "decision",
        "operation",
        "Install App?",
        "Reviewed release",
        vec![crate::session::RuntimeInteractionChoice::new(
            "deny", "Cancel", "deny", None,
        )],
    );
    let mut wire = serde_json::to_value(interaction).unwrap();
    wire["requested_at_ms"] = serde_json::json!(1);
    assert_eq!(
        wire,
        serde_json::json!({
            "id":"decision", "kernel_operation_id":"operation", "kind":"permission", "level":"warning",
            "title":"Install App?", "message":"Reviewed release", "choices":[{"id":"deny","label":"Cancel","reply":"deny"}],
            "timeout_sec":300, "requested_at_ms":1,
        })
    );
    let request =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: "session".into(),
            interaction_id: "decision".into(),
            choice_id: "deny".into(),
            custom_reply: None,
        });
    let snapshot =
        serde_json::json!({"interaction":wire,"request":serde_json::to_value(request).unwrap()});
    let digest = Sha256::digest(serde_json::to_vec(&snapshot).unwrap());
    assert_eq!(
        format!("{digest:x}"),
        "44814660095431daf00196903c16451439a297ffb01a8b8dc646bcb8c09d47d2"
    );
}
