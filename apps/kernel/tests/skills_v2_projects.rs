//! MP-08 / MP-10 / MP-11: folderless Project wire/model regressions, not SV acceptance.
use chariox_kernel::{local::LocalDaemonRequest, session::RuntimeProject};
#[test]
fn skills_v2_folderless_project_has_no_empty_path_workspace() {
    let project: RuntimeProject = serde_json::from_value(serde_json::json!({
        "id":"project-empty","owner_user_id":"owner","workspace_id":"","workspace_ids":[],
        "name":"No folders","kind":"named","status":"active","created_at_ms":1,"updated_at_ms":1
    }))
    .unwrap();
    assert!(
        project.workspace_ids().is_empty(),
        "an empty Project is not a Workspace whose ID is the empty path"
    );
    let restored: RuntimeProject =
        serde_json::from_value(serde_json::to_value(project).unwrap()).unwrap();
    assert!(restored.workspace_ids().is_empty());
}
#[test]
fn skills_v2_folderless_project_create_request_is_a_normal_kernel_request() {
    let wire = serde_json::json!({"CreateProject":{"name":"No folders"}});
    let request = serde_json::from_value::<LocalDaemonRequest>(wire.clone())
        .expect("the kernel must expose standalone Project creation");
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    assert!(serde_json::from_value::<LocalDaemonRequest>(
        serde_json::json!({"CreateProject":{"name":"No folders","owner_user_id":"foreign"}})
    )
    .is_err());
}
#[test]
fn skills_v2_folderless_projects_use_the_shared_owner_and_membership_service() {
    let mut config = chariox_kernel::config::DaemonConfig::for_tests();
    let state = std::env::temp_dir().join(format!(
        "skills-v2-projects-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&state).unwrap();
    config.user_config_path = state.join("config.toml");
    config.user_config.state.path = Some(state.join("state.db").to_string_lossy().into());
    let mut service = chariox_kernel::session::SessionService::new(&config);
    let project = service
        .create_project("No folders".into(), "owner")
        .unwrap();
    assert!(project.workspace_ids().is_empty());
    assert_eq!(service.list_visible_projects("owner", false).len(), 1);
    assert!(service
        .list_visible_projects("collaborator", true)
        .is_empty());
    let updated = service
        .update_project_workspaces(project.id(), vec![], "owner")
        .unwrap();
    assert_eq!(updated.id(), project.id());
    assert!(updated.workspace_ids().is_empty());
    assert!(service
        .update_project_workspaces(project.id(), vec![], "collaborator")
        .is_err());
    assert!(service
        .create_project("No folders".into(), "owner")
        .is_err());
    assert!(service.create_project("".into(), "owner").is_err());
    // A provider still needs a working directory. Selecting one for a session
    // does not silently turn it into a folder of the folderless Project.
    let session = service
        .create_session(
            chariox_kernel::session::CreateSessionRequest::new(
                state.to_string_lossy().as_ref(),
                "worktree-folderless",
            )
            .with_owner_user_id("owner")
            .with_project_selection(
                chariox_kernel::session::SessionProjectSelection::Existing {
                    project_id: project.id().into(),
                },
            ),
        )
        .unwrap();
    assert_eq!(session.project_id(), project.id());
    assert!(service
        .get_project(project.id())
        .unwrap()
        .workspace_ids()
        .is_empty());
    std::fs::remove_dir_all(state).unwrap();
}
