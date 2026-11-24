//! A machine without a slice service refuses CreateSlice through the router in
//! words a grant holder and a sudo turn both read, as a permanent refusal.
use super::tests::fixture;
use super::vault_tests::elevated;
use super::*;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};

#[tokio::test]
async fn grant_and_sudo_callers_learn_the_machine_has_no_slice_service() {
    crate::test_support::isolated_env_test!();
    // What a managed bootstrap hands a kernel on a machine without a slice broker.
    std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");
    std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_FD");
    std::env::set_var("CHARIOX_SLICE_DOCKER_BROKER_REQUIRED", "1");
    let f = fixture();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        source_slice_ref: None,
        name: "linux-dev".into(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".into(),
        display_mode: crate::slice::SliceDisplayMode::Headed,
        display_backend: Default::default(),
        workspace_id: Some(session.workspace_id().into()),
        worktree_id: Some(session.worktree_id().into()),
        workspace_mount: None,
        development: None,
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: Some(crate::local::SliceCreateBase::Clean),
    });

    let mut grant = KernelCommand::from_local_request("slice-by-grant", None, None, &request);
    grant.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    grant.caller.caller_id = f.state.insert_access_grant_for_test(session.id());

    let turn = elevated(&f);
    let mut sudo = KernelCommand::from_local_request(
        format!("{}:slice", turn.entry_id),
        turn.prompt_id.clone(),
        Some(turn.entry_id.clone()),
        &request,
    );
    sudo.caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    sudo.provider_run_id = turn.provider_run_id.clone();
    sudo.caller.caller_id = turn.entry_id.clone();
    sudo.caller.user_id = Some(turn.owner_user_id.clone());
    sudo.caller.metaagent_id = Some(turn.agent_id.clone());

    for command in [grant, sudo] {
        assert!(command.external_grant_id().is_some());
        let error = f
            .router
            .dispatch(command, request.clone())
            .await
            .expect_err("a machine without a slice service refuses CreateSlice");
        let refusal = crate::transport::kernel_protocol::map_kernel_error(&error);
        assert_eq!(refusal.code, "slice_service_missing", "{refusal:?}");
        assert!(!refusal.retryable, "{refusal:?}");
        assert!(
            refusal.message.contains(
                "This machine cannot run slices: its Chariox release has no slice service. \
                 Create the slice on a machine that runs slices."
            ),
            "{refusal:?}"
        );
    }
    assert!(f.state.list_slices().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}
