use super::*;
use serde_json::{json, Value};

fn decision() -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        "decision",
        "operation",
        "Install App?",
        "Reviewed release",
        vec![RuntimeInteractionChoice::new(
            "deny", "Cancel", "deny", None,
        )],
    )
}

#[test]
fn operation_subject_roundtrips_without_an_agent_or_timeout_approval() {
    let interaction = decision();
    let wire = serde_json::to_value(&interaction).unwrap();
    assert!(wire.get("agent_id").is_none());
    assert_eq!(wire["kernel_operation_id"], "operation");
    assert_eq!(wire["kind"], "permission");
    assert!(wire.get("default_on_timeout").is_none());
    assert!(wire.get("custom_choice").is_none());
    assert_eq!(
        serde_json::from_value::<RuntimeInteraction>(wire).unwrap(),
        interaction
    );
}

#[test]
fn invalid_or_ambiguous_subjects_are_rejected_at_the_wire_boundary() {
    let original = serde_json::to_value(decision()).unwrap();
    for fields in [
        json!({}),
        json!({"agent_id":"agent", "kernel_operation_id":"operation"}),
        json!({"agent_id":"agent", "kernel_operation_id":null}),
        json!({"agent_id":null, "kernel_operation_id":"operation"}),
        json!({"kernel_operation_id":""}),
        json!({"kernel_operation_id":"  "}),
        json!({"kernel_operation_id":"line\nbreak"}),
        json!({"agent_id":4}),
        json!({"kernel_operation_id":"x".repeat(129)}),
    ] {
        let mut wire = original.as_object().unwrap().clone();
        wire.remove("kernel_operation_id");
        wire.extend(fields.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<RuntimeInteraction>(Value::Object(wire)).is_err(),
            "{fields}"
        );
    }
}

#[test]
fn native_restamping_replaces_the_subject_and_preserves_the_agent_wire() {
    let interaction = decision().with_agent_id("agent");
    assert_eq!(interaction.agent_id(), Some("agent"));
    assert_eq!(interaction.kernel_operation_id(), None);
    let wire = serde_json::to_value(&interaction).unwrap();
    assert!(wire.get("kernel_operation_id").is_none());
    assert_eq!(wire["agent_id"], "agent");
    assert_eq!(
        serde_json::from_value::<RuntimeInteraction>(wire).unwrap(),
        interaction
    );
}
