//! Local 1016: a secret card names the Vault entry its secret input fills (additive).
use super::*;

#[test]
fn a_secret_card_names_its_entry_and_an_old_client_ignores_it() {
    let wire = serde_json::json!({
        "id": "credential-secret-1",
        "agent_id": "agent",
        "asking_task_id": "task",
        "kind": "choice",
        "level": "critical",
        "title": "Mailbox token",
        "message": "Paste the token.",
        "choices": [{"id": "cancel", "label": "Cancel", "reply": "cancel", "style": "danger"}],
        "custom_choice": {"id": "secret", "label": "Secret", "input_kind": "secret"},
        "default_on_timeout": "cancel",
        "requested_at_ms": 1,
        "secret_request": {
            "credential": {
                "id": "cf-mailbox-token",
                "source": {"type": "vault", "key": "cf-mailbox-token"},
                "injection": {"kind": "env"}
            },
            "overwrite": false
        }
    });
    let card: crate::session::RuntimeInteraction = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        card.secret_request()
            .map(|request| request.credential.id.as_str()),
        Some("cf-mailbox-token")
    );
    assert_eq!(serde_json::to_value(&card).unwrap(), wire);
    // Without the field the card is the card it was: a secret box, no entry named.
    let mut plain = wire;
    plain.as_object_mut().unwrap().remove("secret_request");
    let plain: crate::session::RuntimeInteraction = serde_json::from_value(plain).unwrap();
    assert!(plain.secret_request().is_none());
}
