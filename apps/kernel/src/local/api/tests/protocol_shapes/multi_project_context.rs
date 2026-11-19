//! A client-built slice request, owner-managed transfer and launch target cross the kernel boundary as
//! `source_projects`, from the fixture the kernel-client test builds the same requests against
//! (`multi-project-context-protocol.test.ts`); the retired single-Project selection stays rejected.
use super::*;
use crate::local::{ManagedContextDevelopmentLaunchTarget, ManagedContextLaunchTarget};
use crate::managed_context::package::ManagedContextDevelopmentSelection;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../../../../../fixtures/multi-project-context-protocol.json"
    ))
    .unwrap()
}

#[test]
fn client_built_slice_request_selects_one_project() {
    let wire = fixture()["createSlice"].clone();
    let LocalDaemonRequest::CreateSlice(request) = serde_json::from_value(wire.clone()).unwrap()
    else {
        panic!("the fixture is a CreateSlice request");
    };
    let development = request.development.expect("the slice carries a Project");
    assert!(matches!(
        development,
        ManagedContextDevelopmentSelection::SourceProjects { .. }
    ));
    let (project_id, repositories) = development.single_source_project().expect("one Project");
    assert_eq!(project_id, "project-docs");
    assert_eq!(repositories.len(), 1);
    assert_eq!(
        repositories[0].worktree_id, None,
        "a null worktreeId is no worktree"
    );
    let again: ManagedContextDevelopmentSelection =
        serde_json::from_value(serde_json::to_value(&development).unwrap()).unwrap();
    assert_eq!(again, development);
}

#[test]
fn client_built_owner_transfer_carries_every_ticked_project() {
    let wire = fixture()["ownerManagedTransfer"].clone();
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    let LocalDaemonRequest::StartManagedContextTransfer(start) = &request else {
        panic!("the fixture is a StartManagedContextTransfer request");
    };
    let owner = start
        .owner_managed
        .as_ref()
        .expect("an owner-managed transfer");
    match &owner.context_selection.development_setup {
        crate::managed_context::owner_managed::OwnerManagedDevelopmentSelection::SourceProjects {
            projects,
        } => {
            let ids: Vec<_> = projects.iter().map(|project| project.project_id.as_str()).collect();
            assert_eq!(ids, ["project-app", "project-docs"]);
            assert_eq!(projects[0].repositories.len(), 2);
        }
        other => panic!("expected source projects, got {other:?}"),
    }
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
}

#[test]
fn launch_target_answers_one_entry_per_project() {
    let wire = fixture()["launchTarget"].clone();
    let target: ManagedContextLaunchTarget = serde_json::from_value(wire.clone()).unwrap();
    let ManagedContextDevelopmentLaunchTarget::FromSource { projects } = &target.development else {
        panic!("the fixture is a copy from source");
    };
    assert_eq!(projects.len(), 2);
    assert_ne!(projects[0].destination_root, projects[1].destination_root);
    assert_eq!(serde_json::to_value(target).unwrap(), wire);
}

#[test]
fn the_single_project_selection_is_retired() {
    for wire in fixture()["retiredRequests"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(wire.clone()).is_err(),
            "{wire} is retired"
        );
    }
}
