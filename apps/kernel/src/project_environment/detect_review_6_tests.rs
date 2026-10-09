// MP-08 / MP-10 / MP-11: opt-in after deterministic detection must run once.
use super::*;
#[test]
fn review931_6_new_model_opt_in_is_not_hidden_by_ready_cache() {
    let root = TestWorktree::new("envp02a-review6");
    write(&root, "brief.md", "Campaign brief\n");
    let folders = vec![folder(&root)];
    let detection = detect_environment(&folders, "environment").unwrap();
    let mut operation: EnvironmentOperation = serde_json::from_value(serde_json::json!({
        "operation_id":"detect","attempt":1,"local_project_id":"project","revision_digest":"revision",
        "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
        "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],
        "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
    })).unwrap();

    operation.selected_items = vec!["folder".into()];
    operation.per_item_results = vec![EnvironmentItemResult {
        requirement_id: "detect:model".into(),
        status: EnvironmentObservationStatus::NeedsYourInput,
        reason_code: "deterministic_only".into(),
        safe_summary: "Deterministic detection".into(),
        receipt_ids: vec![],
    }];
    let mut cache = EnvironmentDetectionCache {
        project_id: "project".into(),
        evidence_digest: detection.evidence_digest.clone(),
        proposals: detection.proposals.clone(),
        operation,
        modeled_folders: Default::default(),
    };
    assert_eq!(
        detection.model_folders(Some(&cache), &["folder".into()], &folders),
        ["folder".into()].into()
    );
    cache.modeled_folders = detection.folder_digests.clone();
    assert!(
        detection
            .model_folders(Some(&cache), &["folder".into()], &folders)
            .is_empty(),
        "completed opt-in is reused"
    );
    write(&root, "brief.md", "Changed campaign brief\n");
    let changed = detect_environment(&folders, "environment").unwrap();
    assert_eq!(
        changed.model_folders(Some(&cache), &["folder".into()], &folders),
        ["folder".into()].into(),
        "changed opted-in metadata reruns the utility"
    );
}

#[test]
fn review931_6_model_cache_preserves_legacy_rollback_file() {
    let root = TestWorktree::new("envp02a-review6-rollback");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "project",
        "owner",
        "/plain",
        "Plain",
        crate::session::RuntimeProjectKind::Named,
    );
    store.snapshot(&project).unwrap();
    let legacy = serde_json::json!({
        "project_id":"project", "evidence_digest":"legacy", "proposals":[],
        "operation":{
            "operation_id":"detect","attempt":1,"local_project_id":"project","revision_digest":"revision",
            "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
            "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],
            "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
        }
    });
    let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
    let legacy_path = store.path("project").with_extension("detect.json");
    std::fs::write(&legacy_path, &legacy_bytes).unwrap();
    let mut cache = store.load_detection("project").unwrap().unwrap();
    assert!(
        cache.modeled_folders.is_empty(),
        "upgrade treats old cache coverage as unknown"
    );
    cache
        .modeled_folders
        .insert("folder".into(), "digest".into());
    store.save_detection(&cache).unwrap();
    assert_eq!(
        std::fs::read(&legacy_path).unwrap(),
        legacy_bytes,
        "published rollback reader retains its original schema"
    );
    assert_eq!(store.load_detection("project").unwrap(), Some(cache));
    let model_path = store.path("project").with_extension("detect-model.json");
    assert!(model_path.is_file());
    store.remove("project").unwrap();
    assert!(
        !legacy_path.exists() && !model_path.exists(),
        "Project deletion removes both cache generations"
    );
}
