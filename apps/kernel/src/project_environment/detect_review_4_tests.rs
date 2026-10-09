// MP-08 / MP-10 / MP-11: discardable cache cannot brick Project Get/Detect.
use super::*;
#[test]
fn review931_4_invalid_detection_cache_is_absent_and_replaceable() {
    let root = TestWorktree::new("envp02a-review4");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "project",
        "owner",
        "/plain",
        "Plain",
        crate::session::RuntimeProjectKind::Named,
    );
    let before = store.snapshot(&project).unwrap();
    let cache_path = store.path("project").with_extension("detect-model.json");
    for bytes in [b"invalid".as_slice(), br#"{"new_field":true}"#.as_slice()] {
        std::fs::write(&cache_path, bytes).unwrap();
        assert!(store.load_detection("project").unwrap().is_none());
        assert_eq!(
            store.snapshot(&project).unwrap().content_digest,
            before.content_digest
        );
    }
    std::fs::File::create(&cache_path)
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert!(store.load_detection("project").unwrap().is_none());
    let mut operation: EnvironmentOperation = serde_json::from_value(serde_json::json!({
        "operation_id":"detect","attempt":1,"local_project_id":"other","revision_digest":"revision",
        "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
        "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],
        "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
    })).unwrap();
    let mut cache = EnvironmentDetectionCache {
        project_id: "project".into(),
        evidence_digest: "fresh".into(),
        proposals: vec![],
        operation: operation.clone(),
        modeled_folders: Default::default(),
    };
    store.save_detection(&cache).unwrap();
    assert!(
        store.load_detection("project").unwrap().is_none(),
        "wrong operation binding is discarded"
    );
    operation.local_project_id = "project".into();
    cache.operation = operation;
    store.save_detection(&cache).unwrap();
    assert_eq!(
        store.load_detection("project").unwrap(),
        Some(cache),
        "fresh detection replaces the invalid cache"
    );
}
