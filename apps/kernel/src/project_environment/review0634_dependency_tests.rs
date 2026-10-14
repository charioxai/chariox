//! MP-08 / MP-10 / MP-11: one dependency graph across Save and Check.
use super::*;

fn cycle(service: bool) -> Vec<Requirement> {
    let mut a = requirement("a");
    let mut b = requirement("b");
    if service {
        a.spec = serde_json::from_value(serde_json::json!({"kind":"services","identity":"public-service","depends_on":["b"],"probe":null,"health_expectation":null,"target_binding":null})).unwrap();
    } else {
        a.depends_on = vec!["b".into()];
    }
    b.depends_on = vec!["a".into()];
    let mut downstream = requirement("downstream");
    downstream.depends_on = vec!["a".into()];
    vec![a, b, downstream, requirement("independent")]
}

#[test]
fn review0634_save_rejects_requirement_and_service_dependency_cycles() {
    for service in [false, true] {
        let (_root, store, env) = environment();
        let mut draft = environment_draft(&env);
        draft.project_requirements = cycle(service);
        let request = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: env.local_project_id.clone(),
            expected_revision: env.revision,
            expected_content_digest: env.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        let result = store.save_revision_locked(&env, &request, "owner");
        assert!(
            result.is_err(),
            "Save accepted a cycle (Services dependency: {service})"
        );
        assert!(result.unwrap_err().to_string().contains("cycle"));
        assert!(environment_revision_diff(&env, &request.draft).is_err());
    }
}

#[test]
fn review0634_check_does_not_publish_persisted_cycles_or_dependents_ready() {
    for service in [false, true] {
        let (_root, _store, mut env) = environment();
        env.project_requirements = cycle(service);
        let target = EnvironmentTargetBinding {
            machine_id: "machine".into(),
            target_instance_generation: "boot".into(),
            slice_ref: None,
        };
        let mut observations = env
            .project_requirements
            .iter()
            .map(|r| {
                observation(
                    &env,
                    &r.requirement_id,
                    &target,
                    crate::session::unix_epoch_ms(),
                )
            })
            .collect::<Vec<_>>();
        invalidate_check_dependencies(&env, &mut observations, &target, Some("check"));
        for o in &observations[..3] {
            assert_ne!(
                o.status,
                EnvironmentObservationStatus::Ready,
                "cyclic requirement {} published Ready (Services dependency: {service})",
                o.requirement_id
            );
        }
        assert_eq!(
            observations[3].status,
            EnvironmentObservationStatus::Ready,
            "healthy independent requirement must stay Ready"
        );
    }
}
