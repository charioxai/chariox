//! MP-08 / MP-10 / MP-11: focused recovery and conflict regressions; not live acceptance.
use super::*;
fn fixture() -> (
    crate::test_support::TestWorktree,
    ProjectEnvironmentStore,
    ProjectEnvironment,
) {
    let root = crate::test_support::TestWorktree::new("envp05-apply");
    let project = crate::session::RuntimeProject::new(
        "apply-project",
        "owner",
        root.path().to_string_lossy(),
        "Apply",
        crate::session::RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let env = store.snapshot(&project).unwrap();
    (root, store, env)
}
fn requirement(id: &str) -> Requirement {
    Requirement {
        requirement_id: id.into(),
        title: id.into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::Software {
            identity: "node".into(),
            version_constraint: None,
            platform: None,
            install_scope: EnvironmentInstallScope::System,
            install_source: None,
            detect_only: true,
        },
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    }
}
fn target(generation: &str) -> EnvironmentTargetBinding {
    EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: generation.into(),
        slice_ref: None,
    }
}
fn observed(
    env: &ProjectEnvironment,
    id: &str,
    status: EnvironmentObservationStatus,
) -> MachineObservation {
    MachineObservation {
        environment_id: env.lineage.environment_id.clone(),
        requirement_id: id.into(),
        target: target("one"),
        revision_digest: env.content_digest.clone(),
        status,
        checked_at_ms: Some(crate::session::unix_epoch_ms()),
        measured_facts: empty_environment_facts(),
        reason_code: "measured".into(),
        safe_summary: "Measured target fact".into(),
        check_operation_id: None,
        stale: false,
    }
}
#[test]
fn envp05a_conflict_blocks_dependents_but_not_unrelated_scoped_work() {
    let (_root, _store, mut env) = fixture();
    let node = requirement("global-node");
    let mut dependent = requirement("build");
    dependent.depends_on = vec!["global-node".into()];
    let mut public = requirement("public");
    public.spec = RequirementSpec::Variables {
        name: "PUBLIC_SITE_URL".into(),
        locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
            name: "PUBLIC_SITE_URL".into(),
        },
    };
    env.project_requirements = vec![dependent, public, node];
    let observations = vec![
        observed(&env, "global-node", EnvironmentObservationStatus::Conflict),
        observed(&env, "public", EnvironmentObservationStatus::Ready),
    ];
    let plan = build_apply_plan(&env, target("one"), &[], &observations).unwrap();
    assert_eq!(plan.selected_items, ["global-node", "build", "public"]);
    assert_eq!(
        plan.conflicts
            .iter()
            .map(|r| r.requirement_id.as_str())
            .collect::<Vec<_>>(),
        ["global-node", "build"]
    );
    assert!(plan.required_opt_ins.is_empty());
}
#[test]
fn envp05a_selection_closes_optional_service_dependencies_and_rejects_cycle() {
    let (_root, _store, mut env) = fixture();
    let mut runtime = requirement("runtime");
    runtime.required = false;
    let mut service = requirement("service");
    service.spec = RequirementSpec::Services {
        identity: "preview".into(),
        target_binding: None,
        probe: None,
        health_expectation: None,
        depends_on: vec!["runtime".into()],
    };
    env.project_requirements = vec![service, runtime];
    assert_eq!(apply_selection(&env, &[]).unwrap(), ["runtime", "service"]);
    env.project_requirements[1].depends_on = vec!["service".into()];
    assert!(apply_selection(&env, &[]).is_err());
    assert!(apply_selection(&env, &["foreign".into()]).is_err());
}
#[test]
fn envp05a_plan_never_trusts_stale_or_foreign_target_readiness() {
    let (_root, _store, mut env) = fixture();
    let mut dependent = requirement("dependent");
    dependent.depends_on = vec!["tool".into()];
    env.project_requirements = vec![requirement("tool"), dependent];
    for foreign in [false, true] {
        let mut observation = observed(&env, "tool", EnvironmentObservationStatus::Ready);
        if foreign {
            observation.target = target("other");
        } else {
            observation.stale = true;
        }
        let plan = build_apply_plan(&env, target("one"), &[], &[observation]).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].requirement_id, "dependent");
    }
}
#[test]
fn envp05a_restart_retains_receipt_and_uncertain_item_without_replay() {
    let (_root, store, env) = fixture();
    let receipt = EnvironmentReceipt {
        receipt_id: "committed".into(),
        requirement_id: Some("public".into()),
        target: target("one"),
        revision_digest: env.content_digest.clone(),
        artifact_digest: "metadata-only".into(),
        recorded_at_ms: 100,
    };
    let operation = EnvironmentOperation {
        operation_id: "apply".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target("one"),
        kind: EnvironmentOperationKind::Apply,
        phase: EnvironmentOperationPhase::Validating,
        selected_items: vec!["public".into(), "build".into()],
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: 90,
        updated_at_ms: 100,
        cancellation: None,
        receipts: vec![receipt.clone()],
        recovery_state: EnvironmentRecoveryState::Pending,
    };
    let a = ApplyAttempt {
        plan_id: "plan".into(),
        operation,
        folder_binding_digest: folder_bindings_digest(&env),
        active_requirement: Some("build".into()),
        uncertain_requirements: vec![],
    };
    let _lock = store.lock(&env.local_project_id).unwrap();
    store
        .save_applies(
            &env.local_project_id,
            &EnvironmentApplies {
                plans: vec![],
                attempts: vec![a],
            },
        )
        .unwrap();
    let recovered = store
        .recover_applies(&env.local_project_id, &target("two"))
        .unwrap();
    let a = &recovered.attempts[0];
    assert_eq!(a.operation.phase, EnvironmentOperationPhase::Failed);
    assert_eq!(a.operation.receipts, vec![receipt]);
    assert_eq!(a.uncertain_requirements, ["build"]);
    assert!(a.active_requirement.is_none());
    assert!(matches!(
        a.operation.recovery_state,
        EnvironmentRecoveryState::Resumable { .. }
    ));
    let again = store
        .recover_applies(&env.local_project_id, &target("two"))
        .unwrap();
    assert_eq!(again.attempts[0].operation, a.operation);
    let mut journal = again.clone();
    let old = journal.attempts[0].operation.clone();
    let mut retry = journal.attempts[0].clone();
    retry.operation.attempt += 1;
    retry.operation.target = target("two");
    let current = retry.operation.clone();
    journal.attempts.push(retry);
    assert!(
        latest_apply_attempt(&mut journal, &old).is_none(),
        "late old completion cannot select a historical attempt"
    );
    assert!(latest_apply_attempt(&mut journal, &current).is_some());
    store.remove(&env.local_project_id).unwrap();
    assert!(store
        .load_applies(&env.local_project_id)
        .unwrap()
        .attempts
        .is_empty());
}
