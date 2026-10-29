// MP-08 / MP-10 / MP-11: content identity includes attached reviewable proposals.
use super::*;
#[test]
fn review931_5_attached_evidence_updates_content_digest() {
    let root = TestWorktree::new("envp02a-review5");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = crate::session::RuntimeProject::new(
        "project",
        "owner",
        "/plain",
        "Plain",
        crate::session::RuntimeProjectKind::Named,
    );
    let first = store.snapshot(&project).unwrap();
    let operation: EnvironmentOperation = serde_json::from_value(serde_json::json!({
        "operation_id":"detect","attempt":1,"local_project_id":"project","revision_digest":"revision",
        "target":{"machine_id":"machine","target_instance_generation":"kernel","slice_ref":null},
        "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],
        "created_at_ms":1,"updated_at_ms":1,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
    })).unwrap();
    let mut cache = EnvironmentDetectionCache {
        project_id: "project".into(),
        evidence_digest: "changed".into(),
        proposals: vec![],
        operation,
        modeled_folders: Default::default(),
    };
    store.save_detection(&cache).unwrap();
    let changed = store.snapshot(&project).unwrap();
    assert_ne!(first.content_digest, changed.content_digest);
    assert_eq!(
        changed.content_digest,
        store.snapshot(&project).unwrap().content_digest
    );
    cache.proposals.push(EnvironmentProposal {
        proposal_id: "proposal".into(),
        requirement: Requirement {
            requirement_id: "requirement".into(),
            title: "Example setting".into(),
            scope: RequirementScope::Folder {
                folder_id: changed.folders[0].folder_id.clone(),
            },
            origins: vec![],
            spec: RequirementSpec::Variables {
                name: "EXAMPLE_SETTING".into(),
                locator: ProjectEnvironmentLocator::Missing,
            },
            depends_on: vec![],
            platform_variants: vec![],
            required: false,
            legacy_entry: None,
        },
    });
    store.save_detection(&cache).unwrap();
    let proposed = store.snapshot(&project).unwrap();
    assert_eq!(proposed.evidence_digest, changed.evidence_digest);
    assert_ne!(
        proposed.content_digest, changed.content_digest,
        "attached proposals also change content identity independently of evidence"
    );
    cache.proposals[0].requirement.spec = RequirementSpec::Variables {
        name: "CHANGED_SETTING".into(),
        locator: ProjectEnvironmentLocator::Missing,
    };
    store.save_detection(&cache).unwrap();
    assert_ne!(
        store.snapshot(&project).unwrap().content_digest,
        proposed.content_digest,
        "proposal semantics change identity even with stable proposal IDs"
    );
}
