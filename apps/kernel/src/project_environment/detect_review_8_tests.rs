//! MP-08 / MP-10 / MP-11: provenance saturation must still return persistable proposals.
use super::*;

fn cache(detection: EnvironmentDetection) -> EnvironmentDetectionCache {
    EnvironmentDetectionCache {
        project_id: "project".into(), evidence_digest: detection.evidence_digest,
        proposals: detection.proposals, modeled_folders: Default::default(),
        operation: serde_json::from_value(serde_json::json!({
            "operation_id":"detect","attempt":1,"local_project_id":"project","revision_digest":"revision",
            "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
            "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":detection.skips,
            "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
        })).unwrap(),
    }
}
#[test]
fn review931_7_repeated_origins_remain_usable_and_persistable() {
    let root = TestWorktree::new("envp02a-review7-origins");
    write(&root, "package.json", "{}");
    write(&root, "source.ts", &"process.env.API_KEY;\n".repeat(40_000));
    let detection = detect_environment(&[folder(&root)], "environment").unwrap();
    let cache = cache(detection);
    let store = ProjectEnvironmentStore::new(root.path());
    store
        .save_detection(&cache)
        .expect("bounded evidence must not discard all Detect results");
    let proposal = cache.proposals.iter().find(|p| matches!(&p.requirement.spec, RequirementSpec::Secrets { name, .. } if name == "API_KEY")).unwrap();
    assert_eq!(proposal.requirement.origins.len(), 128);
    for (i, origin) in proposal.requirement.origins.iter().enumerate() {
        assert!(
            matches!(origin, RequirementOrigin::Detected { line: Some(line), evidence_digest, .. } if *line == i as u32 + 1 && evidence_digest.len() == 64)
        );
    }
    assert!(cache
        .operation
        .per_item_results
        .iter()
        .any(|r| r.reason_code == "origin_limit" && r.safe_summary.contains("39872")));
    assert_eq!(store.load_detection("project").unwrap(), Some(cache));
}
#[test]
fn review931_7_total_proposal_bytes_are_bounded_during_collection() {
    let root = TestWorktree::new("envp02a-review7-bytes");
    write(&root, "package.json", "{}");
    let path = format!(
        "{}/source.ts",
        (0..5)
            .map(|i| format!("{i}{}", "x".repeat(180)))
            .collect::<Vec<_>>()
            .join("/")
    );
    let source = (0..64)
        .flat_map(|i| (0..128).map(move |_| format!("process.env.ENV_NAME_{i};\n")))
        .collect::<String>();
    write(&root, &path, &source);
    let detection = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(!detection.proposals.is_empty());
    assert!(
        serde_json::to_vec(&detection.proposals).unwrap().len() <= 2 * 1024 * 1024,
        "serialized proposals: {} bytes",
        serde_json::to_vec(&detection.proposals).unwrap().len()
    );
    assert!(
        detection
            .skips
            .iter()
            .any(|r| r.reason_code == "proposal_byte_limit"),
        "byte-budget omissions must be disclosed"
    );
    let cache = cache(detection);
    // Persist outside the evidence root so a cache write cannot change that input.
    let store = TestWorktree::new("envp02a-review7-bytes-store");
    ProjectEnvironmentStore::new(store.path())
        .save_detection(&cache)
        .unwrap();
    let repeated = detect_environment(&[folder(&root)], "environment").unwrap();
    assert!(
        cache.proposals == repeated.proposals,
        "unchanged evidence retains the same bounded proposals"
    );
    assert_eq!(cache.evidence_digest, repeated.evidence_digest);
}

#[test]
fn review931_7_model_hints_and_reuse_cannot_escape_the_proposal_budget() {
    let root = TestWorktree::new("envp02a-review7-model-bytes");
    let folders = vec![folder(&root)];
    let mut detection = detect_environment(&folders, "environment").unwrap();
    let (mut input, _) = detection.discovery_input("project", &folders, &["folder".into()].into());
    input.changed_paths = (0..32).map(|i| (format!("folder-{i}"), vec![])).collect();
    let hints: Vec<_> = (0..32)
        .map(|i| format!("hint-{i}-{}", "x".repeat(990)))
        .collect();
    let manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: "project".into(),
        evidence_digest: input.evidence_digest.clone(),
        entries: vec![],
        private_files: vec![],
        toolchain_hints: hints.clone(),
        package_hints: hints.clone(),
        service_hints: hints,
    };
    detection.merge_manifest("environment", &input, &manifest);
    assert!(
        serde_json::to_vec(&detection.proposals).unwrap().len() <= 2 * 1024 * 1024,
        "serialized proposals: {} bytes",
        serde_json::to_vec(&detection.proposals).unwrap().len()
    );
    assert!(detection
        .skips
        .iter()
        .any(|r| r.reason_code == "proposal_byte_limit"));
    let cache = cache(detection);
    let store = TestWorktree::new("envp02a-review7-model-store");
    ProjectEnvironmentStore::new(store.path())
        .save_detection(&cache)
        .unwrap();
    let mut repeated = detect_environment(&folders, "environment").unwrap();
    repeated.reuse_model_metadata(&cache, &input.changed_paths.keys().cloned().collect());
    assert!(
        repeated.proposals == cache.proposals,
        "cached bounded hints survive an unchanged Detect"
    );
}
