// MP-08 / MP-10 / MP-11: provider results are proposals, never readiness or saved requirements.
use super::*;
#[test]
fn review931_1_validated_classification_and_hints_reach_proposals() {
    let root = TestWorktree::new("envp02a-review1");
    write(&root, "package.json", "{}");
    write(
        &root,
        "src/index.ts",
        "process.env.PUBLIC_SITE_URL;\nprocess.env.API_KEY;\n",
    );
    let folders = vec![folder(&root)];
    let mut detection = detect_environment(&folders, "environment").unwrap();
    let (input, _) = detection.discovery_input("project", &folders, &["folder".into()].into());
    let mut entries = input.references.clone();
    entries
        .iter_mut()
        .find(|e| e.name == "PUBLIC_SITE_URL")
        .unwrap()
        .classification = ProjectEnvironmentClassification::NonSecret;
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: "project".into(),
        evidence_digest: input.evidence_digest.clone(),
        entries,
        private_files: vec![],
        toolchain_hints: vec!["node".into()],
        package_hints: vec!["sk-proj-private-fixture-never-retain".into()],
        service_hints: vec!["postgres".into()],
    };
    let validated = parse_project_environment_discovery_output(
        &serde_json::to_string(&manifest).unwrap(),
        &input,
    )
    .unwrap();
    detection.merge_manifest("environment", &input, &validated);
    assert!(detection.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Variables { name, locator: ProjectEnvironmentLocator::Missing } if name == "PUBLIC_SITE_URL")));
    assert!(detection.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, vault: None } if name == "API_KEY")));
    for name in ["PUBLIC_SITE_URL", "API_KEY"] {
        let p = detection
            .proposals
            .iter()
            .find(|p| p.requirement.title == name)
            .unwrap();
        assert!(p.requirement.origins.iter().any(|o| matches!(o, RequirementOrigin::DetectedMetadata { source, .. } if source == "provider_classification")));
        assert!(p.requirement.origins.iter().any(|o| matches!(o, RequirementOrigin::Detected { relative_path, .. } if relative_path == "src/index.ts")));
        assert!(!p.requirement.required);
    }
    assert!(detection.proposals.iter().any(|p| matches!(&p.requirement.spec, RequirementSpec::Services { identity, probe: None, target_binding: None, .. } if identity == "postgres")));
    assert!(!serde_json::to_string(&detection.proposals)
        .unwrap()
        .contains("private-fixture"));
    let cache: EnvironmentDetectionCache = serde_json::from_value(serde_json::json!({
        "project_id":"project", "evidence_digest": detection.evidence_digest, "proposals": detection.proposals,
        "operation": { "operation_id":"detect","attempt":1,"local_project_id":"project","revision_digest":"revision",
            "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
            "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],
            "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"} }
    })).unwrap();
    let mut unchanged = detect_environment(&folders, "environment").unwrap();
    unchanged.reuse_model_metadata(&cache, &["folder".into()].into());
    assert_eq!(
        unchanged.proposals, cache.proposals,
        "unchanged detection retains validated model classifications and hints"
    );
    let mut oversized = validated.clone();
    oversized.toolchain_hints = (0..1000).map(|i| format!("toolchain-{i}")).collect();
    let mut bounded = detect_environment(&folders, "environment").unwrap();
    bounded.merge_manifest("environment", &input, &oversized);
    assert!(
        bounded.proposals.len() < 100,
        "free-form output cannot flood the review projection"
    );
    assert!(bounded
        .skips
        .iter()
        .any(|item| item.reason_code == "utility_hints_bounded"));
}
