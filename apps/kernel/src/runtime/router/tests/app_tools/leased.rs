//! MP-08/MP-10/MP-11 D1: home Apps retain home agent authority across a lease.
use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind, RemoteExtensionInvocationMetadata};

#[test]
fn mp08_mp10_mp11_leased_app_uses_home_grant_and_actor() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("chariox-leased-app-{:016x}", rand::random::<u64>())),
    );
    std::fs::create_dir(&scratch.0).unwrap();
    let (router, app_state, store, catalog, context) = {
        let _entered = runtime.enter();
        let mut config = DaemonConfig::for_tests();
        config.room_agent_tools = true;
        config.user_config.state.path = Some(scratch.0.join("state.db").to_string_lossy().into());
        let home_kernel_id = config.daemon_id.clone();
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let store = app.durable_state_store();
        let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
        let session = app
            .sessions_mut()
            .create_session(
                CreateSessionRequest::new(scratch.0.to_string_lossy(), scratch.0.to_string_lossy())
                    .with_owner_user_id("alice"),
            )
            .unwrap();
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub").with_owner_user_id("alice"),
            )
            .unwrap();
        crate::test_support::admit_room_test_turn(&mut app, session.id(), agent.id());
        let app_state = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(app_state.clone(), 4)
            .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
                "worker-kernel",
            ));
        runtime
            .block_on(router.runtime_state.grant_agent_extension(
                agent.id(),
                ExtensionGrant::app("installed"),
                "alice",
            ))
            .unwrap();
        runtime
            .block_on(app_state.lock())
            .agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "worker-agent".into(),
                    active_worker_provider_run_id: Some("worker-run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
            home_kernel_id,
            home_session_id: session.id().into(),
            home_agent_id: agent.id().into(),
            leased_agent_id: "worker-agent".into(),
            worker_provider_run_id: "worker-run".into(),
            worker_kernel_id: Some("worker-kernel".into()),
            worker_machine_id: Some("worker-machine".into()),
        };
        (router, app_state, store, catalog, context)
    };
    let fixture = Fixture::compile().unwrap();
    let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let (process, observed) = fixture.spawn_blocking(Mode::ToolEcho, &package).unwrap();
    let (starting, _control) = crate::runtime::app_worker::AppWorkerOwner::start_blocking(
        process,
        &package,
        catalog,
        Arc::new(RejectBroker),
        PeerLimits::default(),
        runtime.handle().clone(),
    )
    .unwrap();
    let mut registered = starting
        .await_registered_blocking(std::time::Duration::from_secs(3))
        .unwrap();
    let proof = store
        .confirm_app_activation(
            "alice",
            registered.catalog().clone(),
            registered.take_activation_budget().unwrap(),
        )
        .unwrap();
    let (owner, handle) = registered.activate_blocking(proof).unwrap();
    router
        .runtime_state
        .app_control()
        .publish_app_worker("alice", handle)
        .unwrap();
    let agent = runtime
        .block_on(app_state.lock())
        .agents()
        .get_agent(&context.home_agent_id)
        .unwrap();
    let manifest = runtime
        .block_on(app_state.lock())
        .remote_extension_manifest_for_agent(&agent)
        .unwrap();
    let tool = manifest
        .tools
        .iter()
        .find(|t| t.kind == ExtensionKind::App)
        .expect("MP-08 D1: leased catalog must include the home App")
        .clone();
    let call = |context, tool: crate::extension::RemoteExtensionTool| {
        runtime.block_on(
            router
                .runtime_state
                .dispatch_forwarded_home_extension_tool_call(
                    context,
                    RemoteExtensionInvocationMetadata::new("worker-run", &tool.tool_name, None),
                    tool,
                    serde_json::json!({"text":"leased home App"}),
                ),
        )
    };
    let result = call(context.clone(), tool.clone())
        .expect("MP-08 D1: the home kernel serves the leased App call");
    assert!(result.ok);
    let requests = observed.tool_requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["context"]["agent_id"], context.home_agent_id);
    assert_eq!(requests[0]["context"]["actor"]["kind"], "agent");
    assert_eq!(requests[0]["context"]["actor"]["id"], context.home_agent_id);
    assert_eq!(requests[0]["context"]["room_id"], context.home_session_id);
    assert!(requests[0]["context"].get("user_id").is_none());
    let mut forged = context.clone();
    forged.leased_agent_id = "other-worker-agent".into();
    assert!(call(forged, tool.clone()).is_err());
    let mut stale = context.clone();
    stale.worker_provider_run_id = "replaced-run".into();
    assert!(call(stale, tool.clone()).is_err());
    let mut widened = tool.clone();
    widened.version_hash = Some("forged-version".into());
    assert!(call(context.clone(), widened).is_err());
    // Remove the binding without an unrelated relay sync in this source fixture.
    runtime
        .block_on(app_state.lock())
        .agents()
        .revoke_extension(&context.home_agent_id, ExtensionKind::App, "installed")
        .unwrap();
    assert!(call(context, tool).is_err());
    assert_eq!(observed.tool_invocations(), 1);
    owner.shutdown_blocking();
    assert!(observed.was_reaped());
    runtime
        .block_on(router.runtime_state.shutdown_cleanup())
        .unwrap();
}
