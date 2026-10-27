use super::*;
use crate::slice::hosted_worker_test_support::*;

fn assert_prompt_context(machine: &str, worker: &str, slice: Option<&str>, expected: bool) {
    let environment = Environment::new(machine, worker, slice);
    let registry = PromptTemplateRegistry::new(environment.root.join("prompts"));
    registry.materialize_bundled_defaults().unwrap();
    let envelope = PromptAssemblyService::new(registry)
        .assemble_provider_turn(
            &provider_run("session", "agent"),
            "ordinary prompt",
            None,
            Vec::new(),
            PromptAssemblyMode::NormalProviderTurn,
        )
        .unwrap();
    assert_eq!(
        envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/slice"),
        expected
    );
}

#[test]
fn hosted_slice_context_prompt_uses_canonical_worker_and_parent_machine_without_room() {
    assert_prompt_context(MACHINE, &canonical_worker(0), Some(SLICE), true);
}
#[test]
fn hosted_slice_context_prompt_preserves_private_synthetic_machine() {
    assert_prompt_context("slice:private", "private-worker", None, true);
}
#[test]
fn hosted_slice_context_prompt_does_not_trust_friendly_alias_on_ordinary_machine() {
    assert_prompt_context(MACHINE, "ordinary-worker", Some(SLICE), false);
}
#[test]
fn hosted_slice_context_prompt_rejects_canonical_worker_bound_to_other_machine() {
    assert_prompt_context("machine-other", &canonical_worker(0), Some(SLICE), false);
}
