//! Full shared runtime MCP routing against the fixed native ABI fixture. No
//! provider process, Node runtime, OS sandbox or public activation is simulated.
use super::*;
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    worker_peer::{Broker, BrokerFuture, BrokerRequest, PeerLimits},
    worker_process::test_fixture::{Fixture, Mode},
};

struct RejectBroker;
impl Broker for RejectBroker {
    fn handle(&self, _request: BrokerRequest) -> BrokerFuture {
        Box::pin(async {
            Err(chariox_app_runtime::wire::RemoteError {
                code: "FIXTURE_UNSUPPORTED".into(),
                message: "Fixed fixture".into(),
                retryable: None,
            })
        })
    }
}
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn actual_app_tools_follow_current_binding_for_ordinary_and_meta_provider_runs() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("chariox-app-mcp-{:016x}", rand::random::<u64>())),
    );
    std::fs::create_dir(&scratch.0).unwrap();
    let (router, store, catalog, agents, tokens) = {
        let _entered = runtime.enter();
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let store = app.durable_state_store();
        let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
        let session = app
            .sessions_mut()
            .create_session(
                CreateSessionRequest::new(scratch.0.to_string_lossy(), scratch.0.to_string_lossy())
                    .with_owner_user_id("alice"),
            )
            .unwrap();
        let mut agents = Vec::new();
        let mut tokens = Vec::new();
        for meta in [false, true] {
            let mut agent = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_owner_user_id("alice")
                        .with_permission_level_override(
                            crate::provider::AgentPermissionLevel::Yolo,
                        ),
                )
                .unwrap();
            if meta {
                agent = app
                    .agents_mut()
                    .activate_agent_meta_mode(agent.id(), None)
                    .unwrap();
            }
            let run = launch_test_provider(
                &mut app,
                session.id(),
                agent.id(),
                "dev-stub",
                "dev-stub",
                "app-fixture",
            );
            agents.push(agent.id().to_owned());
            tokens.push(run.runtime_mcp_auth_token().unwrap().to_owned());
        }
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        (router, store, catalog, agents, tokens)
    };
    let name = catalog.app_catalog().tools().next().unwrap().name.clone();
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
    for (agent, token) in agents.iter().zip(&tokens) {
        let control = router.runtime_state.app_control();
        let permits = (0..8)
            .map(|_| control.try_admit().unwrap())
            .collect::<Vec<_>>();
        let unrelated_specs = runtime
            .block_on(router.runtime_tool_specs_for_auth_token_async(token.clone()))
            .expect("App saturation must not block an agent with no bound App");
        assert!(!unrelated_specs.is_empty());
        assert!(!unrelated_specs.iter().any(|tool| tool.name == name));
        drop(permits);
        assert!(!router
            .runtime_tool_specs_for_auth_token(token)
            .iter()
            .any(|tool| tool.name == name));
        let granted = runtime
            .block_on(router.runtime_state.grant_agent_extension(
                agent,
                crate::extension::ExtensionGrant::app("installed"),
                "alice",
            ))
            .unwrap();
        let collision = control
            .app_extension_tools_for_agent(
                &granted,
                &std::collections::BTreeSet::from([name.clone()]),
            )
            .expect_err("App names must not replace an existing runtime tool");
        assert!(matches!(
            collision,
            crate::durable_state::app_tools::AppToolsError::Catalog(
                chariox_app_runtime::app_catalog::CatalogError::NameCollision
            )
        ));
        let permits = (0..8)
            .map(|_| control.try_admit().unwrap())
            .collect::<Vec<_>>();
        assert!(
            runtime
                .block_on(router.runtime_tool_specs_for_auth_token_async(token.clone()))
                .is_err(),
            "a failed App projection must not succeed with an incomplete catalog"
        );
        drop(permits);
        let specs = runtime
            .block_on(router.runtime_tool_specs_for_auth_token_async(token.clone()))
            .unwrap();
        assert_eq!(specs.iter().filter(|tool| tool.name == name).count(), 1);
        let result = runtime
            .block_on(router.dispatch_authenticated_runtime_tool_call(
                token,
                &name,
                serde_json::json!({"text":"bound agent"}),
            ))
            .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload, serde_json::json!({"ok":true}));
    }
    assert_eq!(observed.tool_invocations(), 2);
    runtime
        .block_on(router.runtime_state.revoke_agent_extension(
            &agents[0],
            crate::extension::ExtensionKind::App,
            "installed",
            "alice",
        ))
        .unwrap();
    assert!(!router
        .runtime_tool_specs_for_auth_token(&tokens[0])
        .iter()
        .any(|tool| tool.name == name));
    assert!(runtime
        .block_on(router.dispatch_authenticated_runtime_tool_call(
            &tokens[0],
            &name,
            serde_json::json!({"text":"revoked"})
        ))
        .is_err());
    assert_eq!(observed.tool_invocations(), 2);
    owner.shutdown_blocking();
    let permits = (0..8)
        .map(|_| router.runtime_state.app_control().try_admit().unwrap())
        .collect::<Vec<_>>();
    assert!(!runtime
        .block_on(router.runtime_tool_specs_for_auth_token_async(tokens[1].clone()))
        .expect("a saved offline binding needs no App database admission")
        .iter()
        .any(|tool| tool.name == name));
    drop(permits);
    assert!(!router
        .runtime_tool_specs_for_auth_token(&tokens[1])
        .iter()
        .any(|tool| tool.name == name));
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
}
