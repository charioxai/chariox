//! MP-08 / MP-10 / MP-11: public revision review regressions.
use super::*;
// MP-08 / MP-10 / MP-11: applicability selectors are not installation conflicts.
#[test]
fn review0821_broad_and_narrow_software_selectors_are_compatible() {
    let root = crate::test_support::TestWorktree::new("review0821-platform");
    let project = RuntimeProject::new(
        "platform",
        "owner",
        root.path().to_str().unwrap(),
        "Platform",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    let broad:Requirement=serde_json::from_value(serde_json::json!({"requirement_id":"broad","title":"Node","scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":"22","platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap();
    let mut narrow = broad.clone();
    narrow.requirement_id = "narrow".into();
    narrow.scope = RequirementScope::Folder {
        folder_id: draft.folders[0].folder_id.clone(),
    };
    if let RequirementSpec::Software { platform, .. } = &mut narrow.spec {
        *platform = Some(EnvironmentPlatform {
            os: "linux".into(),
            architecture: "x86_64".into(),
        });
    }
    draft.project_requirements = vec![broad];
    draft.folders[0].requirements = vec![narrow];
    environment_revision_diff(&current, &draft)
        .expect("broad/narrow declarations need identical software on their intersection");
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: current.revision,
        expected_content_digest: current.content_digest.clone(),
        draft: draft.clone(),
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store
        .save_revision_locked(&current, &save, "owner")
        .unwrap();
    if let RequirementSpec::Software {
        version_constraint, ..
    } = &mut draft.folders[0].requirements[0].spec
    {
        *version_constraint = Some("24".into());
    }
    assert!(
        environment_revision_diff(&current, &draft).is_err(),
        "incompatible versions still conflict"
    );
}
