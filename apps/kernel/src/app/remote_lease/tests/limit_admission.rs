use super::*;
use crate::extension::RemoteExtensionManifest;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

// A limit the home lowered while the leased agent's turn runs does not fail the update: the
// running turn keeps its process, and the next prompt admission relaunches at the limit.
#[tokio::test]
async fn a_limit_told_during_a_leased_turn_applies_to_the_next_prompt() {
    let (mut app, lease) = leased_agent_fixture(false);
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&lease.backing_agent_id).unwrap(),
    );
    let running = app
        .providers_mut()
        .launch_run_detached(
            LaunchProviderRequest::new(
                &lease.backing_session_id,
                "dev-stub",
                &lease.provider,
                &lease.account_profile,
                lease.model.as_deref().unwrap_or("default"),
            )
            .with_agent_id(&lease.backing_agent_id)
            .with_variant(lease.effort.clone())
            .with_execution_mode(AgentExecutionMode::Build)
            .with_permission_level(AgentPermissionLevel::Yolo),
        )
        .unwrap();
    assert_eq!(
        (running.execution_mode(), running.permission_level()),
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo),
        "the run launched at the open limit"
    );
    sync_active_prompt(&mut app, &lease);

    let updated = RemoteLeaseRuntime::new(&mut app)
        .update_leased_agent_config(
            &lease.id,
            AgentExecutionMode::Plan,
            AgentPermissionLevel::Required,
        )
        .expect("a limit told during the turn is accepted");
    assert_eq!(updated.execution_mode, Some(AgentExecutionMode::Plan));
    assert_eq!(
        updated.permission_level,
        Some(AgentPermissionLevel::Required)
    );
    assert_eq!(
        app.providers().get_run(running.id()).unwrap().state(),
        ProviderRunState::Running,
        "the running turn keeps its process"
    );

    app.prompt_owner_cancel_active_prompt_only(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap();
    assert!(app
        .prompt_owner_active_prompt_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap()
        .is_none());
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let (admitted_run_id, _) = router
        .relay_submit_leased_prompt(
            &lease.id,
            profile,
            None,
            "continue",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            RemoteExtensionManifest::default(),
            None,
        )
        .await
        .unwrap();
    let app = app.lock().await;
    let admitted = app.providers().get_run(&admitted_run_id).unwrap();
    assert_ne!(admitted_run_id, running.id());
    assert_eq!(admitted.execution_mode(), AgentExecutionMode::Plan);
    assert_eq!(admitted.permission_level(), AgentPermissionLevel::Required);
    assert_eq!(
        app.providers().get_run(running.id()).unwrap().state(),
        ProviderRunState::Ended
    );
}
