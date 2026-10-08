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

#[tokio::test]
async fn published_script_with_app_shaped_name_reaches_script_dispatch() {
    let name = "app_record_0123456789abcdef0123456789abcdef";
    let scratch = Scratch(
        std::env::temp_dir().join(format!("chariox-script-mcp-{:016x}", rand::random::<u64>())),
    );
    let script_dir = crate::script::CharioxScriptRegistry::project_root(&scratch.0).join(name);
    std::fs::create_dir_all(&script_dir).unwrap();
    std::fs::write(
        script_dir.join("metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": name,
            "runtime": "python",
            "entrypoint": "script.py",
            "description": "Script with an App-shaped name",
            "input_schema": {"type": "object", "properties": {}},
            "definition_hash": "routing-test",
            "timeout_sec": 10
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        script_dir.join("script.py"),
        "def run():\n    return {\"executed\": True}\n",
    )
    .unwrap();
    let env_dir = crate::script::CharioxEnvironmentRegistry::project_root(&scratch.0);
    std::fs::create_dir_all(&env_dir).unwrap();
    std::fs::write(
        env_dir.join("routing-env.json"),
        r#"{"name":"routing-env","runtime":{"type":"python","python":"/usr/bin/python3"}}"#,
    )
    .unwrap();
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            scratch.0.to_string_lossy(),
            scratch.0.to_string_lossy(),
        ))
        .unwrap();
    let agent = spawn_test_agent(&mut app, session.id(), "script-routing", "dev-stub");
    app.agents()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script(name, "routing-env"),
        )
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "script-routing",
    );
    let token = run.runtime_mcp_auth_token().unwrap().to_owned();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    assert!(router
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|tool| tool.name == name));
    let result = router
        .dispatch_authenticated_runtime_tool_call(&token, name, serde_json::json!({}))
        .await
        .expect("a published script must not be refused as an unbound App tool");
    assert!(result.ok);
    assert_eq!(result.payload, serde_json::json!({"executed": true}));
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
    let (router, store, catalog, agents, tokens, runs, turn_id) = {
        let _entered = runtime.enter();
        // The release store refuses symlinked paths (macOS /var): resolve it.
        let root = scratch.0.canonicalize().unwrap();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.join("state.db").to_string_lossy().into());
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
        let mut agents = Vec::new();
        let mut tokens = Vec::new();
        let mut runs = Vec::new();
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
            runs.push(run.id().to_owned());
        }
        // The ordinary agent calls from inside a turn; the meta agent has none.
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-a",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(&agents[0]),
                "use the App",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("the agent's turn should start");
        };
        // The calling run received the turn.
        app.mark_active_prompt_delivery(
            session.id(),
            &agents[0],
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(runs[0].clone()),
            None,
        )
        .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        (
            router,
            store,
            catalog,
            agents,
            tokens,
            runs,
            prompt.id().to_owned(),
        )
    };
    let name = catalog.app_catalog().tools().next().unwrap().name.clone();
    let fixture = Fixture::compile().unwrap();
    let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    // As an install does: the verified release is stored, so a stopped App's
    // catalog can be read back without its worker.
    chariox_app_runtime::release_store::ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &package,
            &bytes,
            chariox_app_runtime::release_store::StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
    let (process, observed) = fixture.spawn_blocking(Mode::ToolEcho, &package).unwrap();
    let (starting, _control) = crate::runtime::app_worker::AppWorkerOwner::start_blocking(
        process,
        &package,
        catalog.clone(),
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
    let mut bound = Vec::new();
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
        bound.push(granted.clone());
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
    // Each call names its agent, and the turn it was made in, if any.
    let contexts = observed
        .tool_requests()
        .unwrap()
        .into_iter()
        .map(|request| request["context"].clone())
        .collect::<Vec<_>>();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[0]["agent_id"], agents[0].as_str());
    assert_eq!(contexts[0]["turn_id"], turn_id.as_str());
    assert_eq!(contexts[1]["agent_id"], agents[1].as_str());
    assert!(contexts[1].get("turn_id").is_none());
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
    let revoked = runtime
        .block_on(router.dispatch_authenticated_runtime_tool_call(
            &tokens[0],
            &name,
            serde_json::json!({"text":"revoked"}),
        ))
        .unwrap_err();
    // Refused as an App call the agent may no longer make, not passed on to
    // the workflow dispatcher ("no active workflow turn ...").
    assert!(
        matches!(
            &revoked,
            crate::error::DaemonError::LocalTransport { operation, message }
                if *operation == "app.tools" && message.contains("not currently available")
        ),
        "{revoked:?}"
    );
    assert_eq!(observed.tool_invocations(), 2);
    owner.shutdown_blocking();
    let lists = |token: &String| {
        runtime
            .block_on(router.runtime_tool_specs_for_auth_token_async(token.clone()))
            .unwrap()
            .iter()
            .any(|tool| tool.name == name)
    };
    // A listing while App admission is saturated leaves the cold App out; the
    // next listing shows it, so the App's start will refresh no one.
    let control = router.runtime_state.app_control();
    let permits = (0..8)
        .map(|_| control.try_admit().unwrap())
        .collect::<Vec<_>>();
    runtime
        .block_on(router.runtime_tool_specs_for_auth_token_async(tokens[1].clone()))
        .unwrap();
    assert!(control.app_unlisted("alice", "installed", &agents[1]));
    drop(permits);
    runtime.block_on(async {
        timeout(Duration::from_secs(5), async {
            while control.catalog_refresh_pending(&agents[1]) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the delayed refresh runs once a slot frees");
    });
    // No worker runs, but the App may start on demand: the bound agent still
    // lists its tools from the verified release (a call starts it); the agent
    // whose binding was revoked does not.
    assert!(lists(&tokens[1]));
    assert!(!control.app_unlisted("alice", "installed", &agents[1]));
    assert!(!lists(&tokens[0]));
    // The shared projection, which a leased agent's manifest uses too.
    let control = router.runtime_state.app_control();
    control.forget_app_dormant("alice", "installed");
    assert!(control
        .app_extension_tools_for_agent(&bound[1], &std::collections::BTreeSet::new())
        .unwrap()
        .iter()
        .any(|tool| tool.tool_name == name));
    // A user stop keeps them out until an explicit start.
    store
        .stop_app_worker_intent(
            "alice",
            "installed",
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    router
        .runtime_state
        .app_control()
        .forget_app_dormant("alice", "installed");
    assert!(!lists(&tokens[1]));
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());

    // The stopped App's tools are withdrawn. A bound agent calling one by its
    // stable name hears that the App is updating only while an approved
    // update of it is under way; an unbound agent never does.
    let call = |token: &String| {
        format!(
            "{:?}",
            runtime.block_on(router.dispatch_authenticated_runtime_tool_call(
                token,
                &name,
                serde_json::json!({"text":"withdrawn"}),
            ))
        )
    };
    assert!(!call(&tokens[1]).contains("APP_UPDATING"));
    assert!(!store.app_update_underway("alice", "installed").unwrap());
    crate::durable_state::app_state::fixture_stage_approved_tool_update(&store, "installed");
    assert!(store.app_update_underway("alice", "installed").unwrap());
    let updating = call(&tokens[1]);
    assert!(updating.contains("APP_UPDATING"), "{updating}");
    assert!(!call(&tokens[0]).contains("APP_UPDATING"));
    assert_eq!(observed.tool_invocations(), 2);

    // Bound while it is stopped, the other agent lists nothing for it yet.
    let changes = router.runtime_mcp_catalog_changes();
    let watches = [
        changes.subscribe(&runs[0]).unwrap(),
        changes.subscribe(&runs[1]).unwrap(),
    ];
    let before = watches.each_ref().map(|watch| watch.current().desired);
    runtime
        .block_on(router.runtime_state.grant_agent_extension(
            &agents[0],
            crate::extension::ExtensionGrant::app("installed"),
            "alice",
        ))
        .unwrap();
    assert_eq!(watches[0].current().desired, before[0]);
    // Starting it again refreshes, once, the catalogs of the bound agents
    // whose listing left it out or that were bound while it was stopped.
    let (process, restarted) = fixture.spawn_blocking(Mode::ToolEcho, &package).unwrap();
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
    let control = router.runtime_state.app_control();
    control.publish_app_worker("alice", handle).unwrap();
    runtime.block_on(async {
        router.runtime_state.refresh_started_app_catalogs();
        timeout(Duration::from_secs(5), async {
            while watches
                .iter()
                .zip(before)
                .any(|(watch, before)| watch.current().desired == before)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the started App refreshes both agents' catalogs");
    });
    assert!(lists(&tokens[0]) && lists(&tokens[1]));
    assert!(control.take_started_app_refreshes().is_empty());
    owner.shutdown_blocking();
    assert!(restarted.was_reaped());
}

#[test]
fn cancelling_the_turn_aborts_its_in_flight_app_tool() {
    for status in [
        crate::session::PromptStatus::Dispatching,
        crate::session::PromptStatus::Running,
    ] {
        check_app_call_turn_lifetime(status, false);
    }
}

/// MP-08/MP-11 A05: revoking the binding wakes a waiting call, not only its reply.
#[test]
fn capability_revoking_the_app_binding_wakes_its_in_flight_app_tool() {
    check_app_call_turn_lifetime(crate::session::PromptStatus::Running, true);
}

fn check_app_call_turn_lifetime(status: crate::session::PromptStatus, revoke: bool) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .enable_all()
        .build()
        .unwrap();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "chariox-app-turn-cancel-{:016x}",
        rand::random::<u64>()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let (router, store, catalog, session, agent, token, prompts) = {
        let _entered = runtime.enter();
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(scratch.0.join("state.db").to_string_lossy().into());
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
        let run = launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "native-tui-idle",
        );
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { .. } = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "use the App",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("turn starts")
        };
        let token = run.runtime_mcp_auth_token().unwrap().to_owned();
        let prompts = app.prompt_state_owner();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        (router, store, catalog, session, agent, token, prompts)
    };
    let name = catalog.app_catalog().tools().next().unwrap().name.clone();
    let fixture = Fixture::compile().unwrap();
    let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, agent.id())
        .unwrap();
    let mut pending = active.clone();
    pending.set_status(status);
    assert!(prompts.replace_active_prompt_if_matches(&session, agent.id(), &active, pending));
    let mode = if status == crate::session::PromptStatus::Dispatching {
        Mode::ToolEcho
    } else {
        Mode::ToolStall
    };
    let (process, observed) = fixture.spawn_blocking(mode, &package).unwrap();
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
    runtime
        .block_on(router.runtime_state.grant_agent_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app("installed"),
            "alice",
        ))
        .unwrap();
    let dispatch = router.runtime_state.clone();
    let mut call = runtime.spawn(async move {
        dispatch
            .dispatch_authenticated_runtime_tool_call(
                &token,
                &name,
                serde_json::json!({"text":"stall"}),
            )
            .await
    });
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while observed.tool_invocations() == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("handler starts");
    });
    if status == crate::session::PromptStatus::Dispatching {
        let result = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), &mut call).await
        });
        call.abort();
        owner.shutdown_blocking();
        assert!(
            matches!(result, Ok(Ok(Ok(ref reply))) if reply.ok),
            "dispatch acknowledgement must not be required for an in-turn call: {result:?}"
        );
        return;
    }
    assert!(
        !call.is_finished(),
        "stalled call must still be pending before turn cancellation"
    );
    // Keep the caller/connection alive: cancellation must observe the kernel
    // turn state, rather than depend on dropping the MCP dispatch future.
    if revoke {
        runtime
            .block_on(router.runtime_state.revoke_agent_extension(
                agent.id(),
                crate::extension::ExtensionKind::App,
                "installed",
                "alice",
            ))
            .unwrap();
    } else {
        assert!(prompts
            .begin_cancelling_active_prompt(&session, agent.id())
            .is_some());
    }
    let result = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut call).await
    });
    call.abort();
    let cancelled = runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while observed.tool_cancellations() == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    });
    owner.shutdown_blocking();
    assert!(observed.was_reaped());
    assert!(
        cancelled,
        "worker must receive Cancel before explicit test shutdown"
    );
    assert!(
        matches!(result, Ok(Ok(Err(DaemonError::LocalTransport { operation: "app.tools", ref message }))) if message == "App operation is not currently available to this agent"),
        "cancelled turn must terminate App call while MCP connection remains open: {result:?}"
    );
}
