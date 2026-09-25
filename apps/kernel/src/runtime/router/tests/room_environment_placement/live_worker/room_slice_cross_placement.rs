use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{
    RemoteRoomComputerObservationCall, RelayPeerRequest, RelayPeerResponse,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn room_agent_on_second_home_local_slice_uses_the_room_environment_without_direct_slice_access() {
    run_test(
        agent_on_second_home_local_slice_uses_the_room_environment_without_direct_slice_access,
    );
}

async fn agent_on_second_home_local_slice_uses_the_room_environment_without_direct_slice_access() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    fixture.create_slice().await;
    fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .set_status(
            "desktop",
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .expect("headed Room Environment slice should be running");

    let agent_slice_created = dispatch_json(
        &fixture.home,
        json!({"CreateSlice": {
            "name":"room-agent",
            "base":"clean",
            "display_mode":"headless",
            "worker_kernel_ref":"room-agent-slice-worker"
        }}),
    )
    .await
    .expect("create a second local slice for the Room agent");
    let agent_slice_id = agent_slice_created["SliceCreated"]["slice"]["id"]
        .as_str()
        .expect("Room agent slice ID")
        .to_string();
    let agent_worker_kernel_id = "room-agent-slice-worker";
    let agent_worker_machine_id = format!("slice:{agent_slice_id}");
    {
        let app = fixture.home.app.lock().await;
        app.slices()
            .set_relay_endpoint(
                &agent_slice_id,
                Some(crate::slice::SliceRelayEndpoint {
                    url: format!("ws://{}", fixture.address),
                    private: false,
                }),
                crate::session::unix_epoch_ms(),
            )
            .expect("second local slice should use the fixture relay");
        app.slices()
            .set_worker_presence(
                &agent_slice_id,
                Some(agent_worker_kernel_id.to_string()),
                Some(agent_worker_machine_id.clone()),
                vec!["managed-dev-stub".to_string()],
                crate::session::unix_epoch_ms(),
            )
            .expect("second local slice should identify its own worker kernel");
        app.slices()
            .set_status(
                &agent_slice_id,
                crate::slice::SliceStatus::Running,
                crate::session::unix_epoch_ms(),
            )
            .expect("Room agent slice should be running");
    }
    let browser_slice = fixture
        .home
        .runtime_state
        .resolve_slice("desktop")
        .expect("resolve headed Room Environment slice");
    let agent_slice = fixture
        .home
        .runtime_state
        .resolve_slice(&agent_slice_id)
        .expect("resolve separate Room agent slice");
    assert_eq!(
        browser_slice.owner_kernel_id,
        fixture.home_state.config.daemon_id,
        "the browser slice is owned by the home kernel"
    );
    assert_eq!(
        browser_slice.backend,
        crate::slice::SliceBackendKind::LocalDocker,
        "the headed Room Environment must use a local home-owned slice"
    );
    assert_eq!(
        browser_slice.display_mode,
        crate::slice::SliceDisplayMode::Headed
    );
    assert_eq!(
        agent_slice.owner_kernel_id,
        fixture.home_state.config.daemon_id,
        "the agent slice is owned by the same home kernel"
    );
    assert_eq!(
        agent_slice.backend,
        crate::slice::SliceBackendKind::LocalDocker,
        "the Room agent must use a second local slice"
    );
    assert_eq!(
        browser_slice.owner_machine_id,
        fixture.home_state.config.host_machine_id,
        "the headed Room Environment belongs to the home machine"
    );
    assert_eq!(
        agent_slice.owner_machine_id,
        browser_slice.owner_machine_id,
        "both local slices belong to the same home machine"
    );
    let browser_worker_machine_id = format!("slice:{}", browser_slice.id);
    assert_eq!(
        browser_slice.worker_machine_id.as_deref(),
        Some(browser_worker_machine_id.as_str())
    );
    assert_eq!(
        agent_slice.worker_machine_id.as_deref(),
        Some(agent_worker_machine_id.as_str())
    );
    assert_ne!(
        browser_slice.id, agent_slice.id,
        "the Room agent must not run in the browser slice"
    );
    assert_ne!(
        browser_slice.worker_kernel_id,
        agent_slice.worker_kernel_id,
        "the Room agent and Environment use different local slice workers"
    );

    let (agent_worker_state, agent_worker) = start_agent_slice_worker(
        &mut fixture,
        agent_worker_kernel_id,
        &agent_worker_machine_id,
        &agent_slice_id,
    )
    .await;
    wait_for_agent_slice_worker(&fixture, agent_worker_kernel_id).await;

    let screen_tool = fixture._worker_state.root.join("cross-slice-screen.sh");
    std::fs::write(
        &screen_tool,
        concat!(
            "#!/bin/sh\n",
            "set -eu\n",
            "[ \"${1:-}\" = status ] || exit 2\n",
            "printf 'available=true\\nscreen=1280x800\\nmode=desktop\\n'\n",
        ),
    )
    .expect("write the Computer helper fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&screen_tool, std::fs::Permissions::from_mode(0o700))
            .expect("Computer helper should be executable");
    }
    let _screen_tool_env = ScopedEnvironmentVariable::set(
        "CHARIOX_SLICE_SCREEN_TOOL",
        screen_tool.as_os_str(),
    );

    let check = std::panic::AssertUnwindSafe(async {
        let room = fixture.rooms[0].clone();
        let binding = dispatch_json(
            &fixture.home,
            json!({"BindRoomEnvironmentSlice": {
                "session_id":room,
                "slice_ref":"desktop"
            }}),
        )
        .await
        .expect("bind the Room Environment to the home machine's headed slice");
        assert_eq!(
            binding["RoomEnvironmentSlice"]["binding"]["slice_id"],
            browser_slice.id
        );
        let started = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room,
                "viewport": {
                    "css_width":1280,
                    "css_height":800,
                    "device_scale_factor":1,
                    "desktop_pixel_width":1280,
                    "desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect("start the one Room Environment on the browser slice");
        let environment = &started["RoomEnvironmentUpdated"]["environment"];
        assert_eq!(environment["lifecycle"], "ready");
        let environment_id = environment["environment_id"]
            .as_str()
            .expect("Room Environment identity")
            .to_string();
        let stable_tab_id = environment["focused_tab_id"]
            .as_str()
            .expect("stable Room Tab")
            .to_string();

        let placement = fixture.placement();
        let spawned = dispatch_json(
            &fixture.home,
            json!({"SpawnAgent": {
                "session_id":room,
                "provider":"managed-dev-stub",
                "model":"runtime-mcp-idle",
                "slice_ref":agent_slice_id,
                "worktree_placement":placement
            }}),
        )
        .await
        .expect("place the Room agent through home admission on the second local slice");
        let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
            .as_str()
            .expect("home agent identity")
            .to_string();
        assert_eq!(spawned["AgentSpawned"]["agent"]["session_id"], room);
        let remote_execution: crate::agent::RemoteAgentBinding = serde_json::from_value(
            spawned["AgentSpawned"]["agent"]["remote_execution"].clone(),
        )
        .expect("home-owned remote worker binding");
        assert_eq!(remote_execution.worker_kernel_id, agent_worker_kernel_id);
        assert_eq!(remote_execution.worker_machine_id, agent_worker_machine_id);
        assert_ne!(
            remote_execution.worker_machine_id,
            browser_slice.worker_machine_id.as_deref().unwrap_or_default(),
            "the provider run must stay in the distinct agent slice"
        );
        let attached_agent_slice = fixture
            .home
            .runtime_state
            .resolve_slice(&agent_slice_id)
            .expect("resolve the attached Room agent slice");
        assert!(
            attached_agent_slice.agent_ids.contains(&home_agent_id),
            "home admission should attach the Room agent to its selected local slice"
        );
        let attached_browser_slice = fixture
            .home
            .runtime_state
            .resolve_slice("desktop")
            .expect("resolve the Room Environment slice after agent placement");
        assert!(
            !attached_browser_slice.agent_ids.contains(&home_agent_id),
            "the Room agent must not attach to the browser slice"
        );

        let (relay_config, expected_profile, remote_extension_manifest) = {
            let app = fixture.home.app.lock().await;
            let agent = app
                .agents()
                .get_agent(&home_agent_id)
                .expect("Room agent remains home-owned");
            (
                app.relay_config_for_remote_execution(&remote_execution),
                crate::transport::relay_peer::RelayAgentExecutionProfile::from(&agent),
                app.remote_extension_manifest_for_agent(&agent)
                    .expect("home should project its Browser and Computer capability"),
            )
        };
        assert!(remote_extension_manifest.room_browser_available);
        let response = send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(agent_worker_kernel_id.to_string()),
                daemon_alias: None,
            },
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                expected_profile,
                prompt: "exercise the home-owned Room Environment".to_string(),
                hidden_system_context: String::new(),
                attachments: Vec::new(),
                workflow_context: None,
                git_context: None,
                required_mcps: Vec::new(),
                required_skills: None,
                remote_extension_manifest: remote_extension_manifest.clone(),
                provider_launch_credential: None,
            },
        )
        .await
        .expect("launch the Room provider on the agent slice");
        let RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id, ..
        } = response
        else {
            panic!("unexpected leased prompt response: {response:?}");
        };
        fixture
            .home
            .app
            .lock()
            .await
            .agents()
            .set_remote_execution_active_worker_provider_run_id(
                &home_agent_id,
                Some(provider_run_id.clone()),
            )
            .expect("record the active provider run on the home agent");

        let token = agent_worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&provider_run_id)
            .expect("agent slice provider runtime token");
        let advertised = agent_worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for tool in [
            "slice_open_url",
            "slice_browser_status",
            "slice_screen_status",
        ] {
            assert!(
                advertised.contains(tool),
                "home Room capability did not project {tool} to the agent slice"
            );
        }
        let url = "https://home-local-slice-cross-placement.test/";
        let opened = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_open_url",
                json!({"url":url}),
            )
            .await
            .expect("Browser call should cross home admission");
        assert!(opened.ok, "{:?}", opened.payload);
        assert_eq!(opened.payload["session_id"], room);
        assert_eq!(opened.payload["agent_id"], home_agent_id);
        assert_eq!(opened.payload["browser"]["url"], url);

        let status = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_browser_status",
                json!({}),
            )
            .await
            .expect("Browser status should come from the Room Environment");
        assert!(status.ok, "{:?}", status.payload);
        assert_eq!(status.payload["environment_id"], environment_id);
        assert_eq!(status.payload["tab_id"], stable_tab_id);
        assert_eq!(status.payload["url"], url);

        let screen_status = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_screen_status",
                json!({"session_id":fixture.rooms[1], "slice_id":agent_slice_id}),
            )
            .await
            .expect("Computer status should resolve through the home Room binding");
        assert!(screen_status.ok, "{:?}", screen_status.payload);
        assert_eq!(screen_status.payload["source"], "computer_controller");
        assert_eq!(screen_status.payload["session_id"], room);
        assert_eq!(screen_status.payload["slice_id"], browser_slice.id);
        assert_eq!(screen_status.payload["agent_id"], home_agent_id);
        assert_eq!(screen_status.payload["screen"], "1280x800");

        // The remote agent receives home-authorized Room Browser and Computer
        // calls, not arbitrary browser-slice file/process handles. The
        // controller, Computer observation, and screenshot peers are bound to
        // the home kernel and must reject direct requests from the agent slice.
        let browser_worker_target = ClientTarget {
            daemon_id: Some(
                browser_slice
                    .worker_kernel_id
                    .clone()
                    .expect("browser slice worker kernel"),
            ),
            daemon_alias: None,
        };
        let direct_controller = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            browser_worker_target.clone(),
            RelayPeerRequest::RoomBrowserController {
                session_id: room.clone(),
                slice_id: browser_slice.id.clone(),
                command: RoomBrowserControllerCommand::Acquire,
            },
        )
        .await;
        let direct_controller = direct_controller
            .expect_err("the browser worker must reject a direct agent-slice peer");
        assert!(
            direct_controller
                .to_string()
                .contains("browser_controller_scope_denied"),
            "direct controller access was not denied: {direct_controller}"
        );
        let direct_computer = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            browser_worker_target.clone(),
            RelayPeerRequest::ObserveRoomComputer {
                session_id: room.clone(),
                slice_id: browser_slice.id.clone(),
                call: RemoteRoomComputerObservationCall::ScreenStatus,
            },
        )
        .await;
        let direct_computer = direct_computer
            .expect_err("the browser worker must reject a direct Computer observation");
        assert!(
            direct_computer
                .to_string()
                .contains("Room Computer read peer or binding scope was denied"),
            "direct Computer access was not denied: {direct_computer}"
        );
        let direct_screenshot_read = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            browser_worker_target,
            RelayPeerRequest::ReadRoomScreenshotChunk {
                session_id: room.clone(),
                slice_id: browser_slice.id.clone(),
                artifact_id: "browser-slice-private-artifact".to_string(),
                offset: 0,
                max_bytes: 128,
            },
        )
        .await;
        let direct_screenshot_read = direct_screenshot_read
            .expect_err("the browser worker must reject a direct screenshot artifact read");
        assert!(
            direct_screenshot_read
                .to_string()
                .contains("Room Computer read peer or binding scope was denied"),
            "direct screenshot-file access was not denied: {direct_screenshot_read}"
        );

        let home_environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(&room)
            .expect("home retains the Room Environment and stable Tab");
        assert_eq!(home_environment.environment_id, environment_id);
        assert_eq!(home_environment.focused_tab_id.as_deref(), Some(stable_tab_id.as_str()));
        assert_eq!(
            home_environment
                .tabs
                .iter()
                .find(|tab| tab.tab_id == stable_tab_id)
                .expect("stable Room Tab")
                .url,
            url
        );
    })
    .catch_unwind()
    .await;

    let provider_cleanup = agent_worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    let controller_cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    provider_cleanup.expect("stop provider process on the agent slice");
    controller_cleanup.expect("stop Browser Controller on the Environment slice");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

async fn start_agent_slice_worker(
    fixture: &mut LiveWorker,
    worker_kernel_id: &str,
    worker_machine_id: &str,
    slice_id: &str,
) -> (TestState, Arc<CommandRouter>) {
    let mut worker_state = TestState::new();
    worker_state.config.relay_url = fixture.home_state.config.relay_url.clone();
    worker_state.config.relay_token = fixture.home_state.config.relay_token.clone();
    worker_state.config.relay_heartbeat_ms = 50;
    worker_state.config.daemon_id = worker_kernel_id.to_string();
    worker_state.config.daemon_alias = Some(format!("slice:{slice_id}:worker:test"));
    worker_state.config.host_machine_id = worker_machine_id.to_string();
    let worker = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(worker_state.config.clone())
                .expect("second local slice worker bootstrap"),
        )),
        2,
    ));
    let relay_state = worker.app.lock().await.relay_client_state();
    fixture.tasks.push(tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router(
            Arc::clone(&worker),
            relay_state,
            fixture.shutdown.subscribe(),
        ),
    ));
    (worker_state, worker)
}

async fn wait_for_agent_slice_worker(fixture: &LiveWorker, worker_kernel_id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if send_peer_request_via_temporary_connection(
                &fixture.home_state.config,
                ClientTarget {
                    daemon_id: Some(worker_kernel_id.to_string()),
                    daemon_alias: None,
                },
                RelayPeerRequest::Ping {
                    value: "room-slice-cross-placement-ready".to_string(),
                },
            )
            .await
            .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("second local slice worker should join the relay");
}

struct ScopedEnvironmentVariable {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl ScopedEnvironmentVariable {
    fn set(name: &'static str, value: &std::ffi::OsStr) -> Self {
        let previous = std::env::var_os(name);
        std::env::set_var(name, value);
        Self { name, previous }
    }
}

impl Drop for ScopedEnvironmentVariable {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}
