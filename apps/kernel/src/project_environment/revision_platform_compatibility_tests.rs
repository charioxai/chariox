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

// MP-08 / MP-10 / MP-11: overlap includes operator-dependent wildcard prefix boundaries.
#[test]
fn wildcard_operator_overlap_survives_preview_and_save() {
    let root = crate::test_support::TestWorktree::new("wildcard-overlap");
    let project = RuntimeProject::new(
        "wildcard",
        "owner",
        root.path().to_str().unwrap(),
        "Wildcard",
        RuntimeProjectKind::Named,
    );
    let store = ProjectEnvironmentStore::new(root.path());
    let current = store.snapshot(&project).unwrap();
    for (a, b, overlap) in [
        (">22.x", "<24", true),
        (">22.x", "<23", false),
        (">22.3.x", "<22.5", true),
        (">22.3.x", "<22.4", false),
        ("^0.2.x", ">=0.3", false),
        ("<=22.x", ">22", true),
        ("~22.3", ">=22.4", false),
        ("22.x || 24.x", ">=23 <24", false),
    ] {
        let mut draft = environment_draft(&current);
        draft.project_requirements = [a,b].into_iter().enumerate().map(|(i,v)| serde_json::from_value(serde_json::json!({"requirement_id":format!("node-{i}"),"title":"Node","scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":v,"platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()).collect();
        assert_eq!(
            environment_revision_diff(&current, &draft).is_ok(),
            overlap,
            "Preview {a} / {b}"
        );
        let request = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        assert_eq!(
            store
                .save_revision_locked(&current, &request, "owner")
                .is_ok(),
            overlap,
            "Save {a} / {b}"
        );
    }
}

#[test]
fn r4_software_constraints_accept_narrowing_and_reject_contradictions() {
    for (broad_version, narrow_version, compatible) in [
        (None, Some("22"), true),
        (Some(">=20"), Some(">=22"), true),
        (Some(">=20 <24"), Some(">=22 <26"), true),
        (Some("^22"), Some("22.1"), true),
        (Some("~22.1"), Some("22.1.3"), true),
        (Some("20 || 22"), Some("22.*"), true),
        (Some("22"), Some("24"), false),
        (Some(">=22"), Some("<20"), false),
        (Some("~22.1"), Some("22.2"), false),
        (Some(">22.0.0.0"), Some("<22.0.0.1"), false),
    ] {
        let root = crate::test_support::TestWorktree::new("r4-version-scope");
        let project = RuntimeProject::new(
            "versions",
            "owner",
            root.path().to_str().unwrap(),
            "Versions",
            RuntimeProjectKind::Named,
        );
        let store = ProjectEnvironmentStore::new(root.path());
        let current = store.snapshot(&project).unwrap();
        let mut draft = environment_draft(&current);
        let broad: Requirement = serde_json::from_value(serde_json::json!({"requirement_id":"broad","title":"Node","scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":broad_version,"platform":null,"install_scope":"system","install_source":null,"detect_only":true},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap();
        let mut narrow = broad.clone();
        narrow.requirement_id = "narrow".into();
        narrow.scope = RequirementScope::Folder {
            folder_id: draft.folders[0].folder_id.clone(),
        };
        if let RequirementSpec::Software {
            version_constraint, ..
        } = &mut narrow.spec
        {
            *version_constraint = narrow_version.map(str::to_owned)
        }
        draft.project_requirements = vec![broad];
        draft.folders[0].requirements = vec![narrow];
        assert_eq!(
            environment_revision_diff(&current, &draft).is_ok(),
            compatible,
            "Preview {broad_version:?} / {narrow_version:?}"
        );
        let save = crate::local::SaveProjectEnvironmentRevisionRequest {
            project_id: project.id().into(),
            expected_revision: current.revision,
            expected_content_digest: current.content_digest.clone(),
            draft,
            accepted_proposal_ids: vec![],
            excluded_proposal_ids: vec![],
        };
        assert_eq!(
            store.save_revision_locked(&current, &save, "owner").is_ok(),
            compatible,
            "Save {broad_version:?} / {narrow_version:?}"
        );
    }
}
