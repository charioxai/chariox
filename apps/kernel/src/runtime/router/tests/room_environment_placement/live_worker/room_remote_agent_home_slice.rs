use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteExtensionInvocationContext,
    RemoteRoomBrowserRuntimeToolCall,
};
use crate::transport::room_browser_controller::RoomBrowserControllerCommand;
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn remote_room_agent_uses_home_local_environment_browser_computer_and_web_view() {
    run_test(check_remote_room_agent_uses_home_local_environment_browser_computer_and_web_view);
}

async fn check_remote_room_agent_uses_home_local_environment_browser_computer_and_web_view() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (_screen_environment, screen_log) =
        install_room_pointer_screen_tool(&fixture._worker_state.root);
    let (agent_worker_state, agent_worker) = start_remote_agent_worker(&mut fixture).await;
    let mut home_agent_id = None;
    let check = std::panic::AssertUnwindSafe(async {
        wait_for_remote_agent_worker(&fixture).await;
        let (room, attachment_id, viewer_public_key, environment_slice) =
            prepare_home_slice_room_display(&fixture).await;
        assert_eq!(
            environment_slice.owner_kernel_id, fixture.home_state.config.daemon_id,
            "the headed Environment slice is owned by the home kernel"
        );
        assert_eq!(
            environment_slice.owner_machine_id,
            fixture.home_state.config.host_machine_id
        );
        assert_eq!(
            environment_slice.backend,
            crate::slice::SliceBackendKind::LocalDocker
        );
        assert_eq!(
            environment_slice.display_mode,
            crate::slice::SliceDisplayMode::Headed
        );

        let started = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id": &room,
                "viewport": {
                    "css_width":1280, "css_height":800, "device_scale_factor":1,
                    "desktop_pixel_width":1280, "desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect("start the Room Environment on its home-owned local slice");
        let initial = &started["RoomEnvironmentUpdated"]["environment"];
        assert_eq!(initial["session_id"], room);
        assert_eq!(initial["lifecycle"], "ready");
        let environment_id = initial["environment_id"]
            .as_str()
            .expect("Room Environment ID")
            .to_string();
        let stable_tab_id = initial["focused_tab_id"]
            .as_str()
            .expect("stable Room Tab ID")
            .to_string();

        let leased = launch_remote_room_provider(
            &mut fixture,
            &agent_worker,
            "exercise the home-slice Room Environment from a remote kernel",
            &mut home_agent_id,
        )
        .await;
        assert!(
            leased.remote_extension_manifest.room_browser_available,
            "home admission must advertise the bound Room Environment"
        );
        assert_ne!(
            leased.remote_execution.worker_kernel_id,
            environment_slice
                .worker_kernel_id
                .as_deref()
                .unwrap_or_default(),
            "the remote agent kernel must not be the home slice Environment worker"
        );
        assert_ne!(
            leased.remote_execution.worker_machine_id,
            environment_slice
                .worker_machine_id
                .as_deref()
                .unwrap_or_default(),
            "the remote agent must not be represented as another local slice"
        );
        assert_eq!(
            fixture
                .home
                .runtime_state
                .resolve_slice("desktop")
                .expect("home Environment slice")
                .agent_ids,
            Vec::<String>::new(),
            "the remote Room agent must not be attached to the Environment slice"
        );

        let token = agent_worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&leased.worker_provider_run_id)
            .expect("remote provider runtime tool token");
        let advertised = agent_worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for tool in ["slice_open_url", "slice_browser_status", "slice_mouse"] {
            assert!(
                advertised.contains(tool),
                "leased Room agent is missing {tool}"
            );
        }

        let url = "https://remote-agent.home-local-slice.test/";
        let browser = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_open_url", json!({"url":url}))
            .await
            .expect("remote Browser tool must reach the home Environment through admission");
        assert!(browser.ok, "{:?}", browser.payload);
        assert_eq!(browser.payload["session_id"], room);
        assert_eq!(browser.payload["agent_id"], leased.home_agent_id);
        assert_eq!(
            browser.payload["actor_id"],
            crate::session::agent_environment_actor_id(&leased.home_agent_id)
        );
        assert_eq!(browser.payload["browser"]["url"], url);
        let browser_action_id = browser.payload["action_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .expect("home-admitted Browser action ID")
            .to_string();

        let computer = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_mouse",
                json!({"action":"move","x":8,"y":8}),
            )
            .await
            .expect("remote Computer tool must reach the same home Environment through admission");
        assert!(computer.ok, "{:?}", computer.payload);
        assert_eq!(computer.payload["session_id"], room);
        assert_eq!(computer.payload["environment_id"], environment_id);
        assert_eq!(computer.payload["agent_id"], leased.home_agent_id);
        assert_eq!(
            computer.payload["actor_id"],
            crate::session::agent_environment_actor_id(&leased.home_agent_id)
        );
        assert_eq!(computer.payload["action_kind"], "pointer_move");
        let computer_action_id = computer.payload["action_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .expect("home-admitted Computer action ID")
            .to_string();
        assert_eq!(
            std::fs::read_to_string(&screen_log).expect("home slice pointer helper log"),
            "move 8 8\n"
        );

        let status = agent_worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_browser_status", json!({}))
            .await
            .expect("remote agent reads the same stable Browser Tab");
        assert!(status.ok, "{:?}", status.payload);
        assert_eq!(status.payload["environment_id"], environment_id);
        assert_eq!(status.payload["tab_id"], stable_tab_id);
        assert_eq!(status.payload["url"], url);

        let before_denial = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{"session_id":&room}}),
        )
        .await
        .expect("read public home Room state after remote Browser and Computer calls");
        let before_environment = &before_denial["RoomEnvironmentState"]["environment"];
        assert_eq!(before_environment["session_id"], room);
        assert_eq!(before_environment["environment_id"], environment_id);
        assert_eq!(before_environment["focused_tab_id"], stable_tab_id);
        let tabs = before_environment["tabs"]
            .as_array()
            .expect("public Room Tabs");
        assert_eq!(
            tabs.len(),
            1,
            "the Environment retains one stable Browser Tab"
        );
        let stable_tab = tabs
            .iter()
            .find(|tab| tab["tab_id"] == stable_tab_id)
            .expect("public Room state retains the original Tab");
        assert_eq!(stable_tab["url"], url);
        let actions = before_environment["actions"]
            .as_array()
            .expect("public action ledger");
        for (action_id, action_kind) in [
            (&browser_action_id, "navigate"),
            (&computer_action_id, "pointer_move"),
        ] {
            let matching = actions
                .iter()
                .filter(|action| action["action_id"].as_str() == Some(action_id.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "each remote action is recorded once");
            assert_eq!(
                matching[0]["actor_id"],
                crate::session::agent_environment_actor_id(&leased.home_agent_id)
            );
            assert_eq!(matching[0]["kind"], action_kind);
            assert_eq!(matching[0]["state"], "completed");
        }
        let action_count = actions.len();

        let environment_worker_target = ClientTarget {
            daemon_id: Some(fixture._worker_state.config.daemon_id.clone()),
            daemon_alias: None,
        };
        let direct_controller = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            environment_worker_target.clone(),
            RelayPeerRequest::RoomBrowserController {
                session_id: room.clone(),
                slice_id: environment_slice.id.clone(),
                command: RoomBrowserControllerCommand::Acquire,
            },
        )
        .await
        .expect_err("remote worker must not directly control the home slice Browser process");
        assert!(
            direct_controller
                .to_string()
                .contains("browser_controller_scope_denied"),
            "direct Browser process access must be denied: {direct_controller}"
        );
        let direct_screenshot = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            environment_worker_target,
            RelayPeerRequest::ReadRoomScreenshotChunk {
                session_id: room.clone(),
                slice_id: environment_slice.id.clone(),
                artifact_id: "unowned-home-slice-artifact".to_string(),
                offset: 0,
                max_bytes: 128,
            },
        )
        .await
        .expect_err("remote worker must not read home slice screenshot files directly");
        assert!(
            direct_screenshot
                .to_string()
                .contains("Room Computer read peer or binding scope was denied"),
            "direct Environment artifact access must be denied: {direct_screenshot}"
        );

        let wrong_room = send_peer_request_via_temporary_connection(
            &agent_worker_state.config,
            ClientTarget {
                daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
                context: RemoteExtensionInvocationContext {
                    home_kernel_id: fixture.home_state.config.daemon_id.clone(),
                    home_session_id: fixture.rooms[1].clone(),
                    home_agent_id: leased.home_agent_id.clone(),
                    leased_agent_id: leased.remote_execution.leased_agent_id.clone(),
                    worker_provider_run_id: leased.worker_provider_run_id.clone(),
                    worker_kernel_id: Some(leased.remote_execution.worker_kernel_id.clone()),
                    worker_machine_id: Some(leased.remote_execution.worker_machine_id.clone()),
                },
                call: RemoteRoomBrowserRuntimeToolCall {
                    tool_name: "slice_open_url".to_string(),
                    arguments: json!({"url":"https://wrong-room.must-not-open/"}),
                },
            },
        )
        .await
        .expect_err("home admission must reject a foreign-Room Browser substitution");
        assert!(
            wrong_room
                .to_string()
                .contains("agent does not belong to invocation session"),
            "foreign-Room request must fail at home admission: {wrong_room}"
        );

        let after_denials = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{"session_id":&room}}),
        )
        .await
        .expect("read public Room state after direct peer denials");
        let after_environment = &after_denials["RoomEnvironmentState"]["environment"];
        assert_eq!(after_environment["environment_id"], environment_id);
        assert_eq!(after_environment["focused_tab_id"], stable_tab_id);
        assert_eq!(
            after_environment["actions"].as_array().unwrap().len(),
            action_count
        );
        assert_eq!(after_environment["tabs"][0]["url"], url);

        let binding = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentSlice":{"session_id":&room}}),
        )
        .await
        .expect("read public Environment slice binding");
        let binding = &binding["RoomEnvironmentSlice"]["binding"];
        assert_eq!(binding["session_id"], room);
        assert_eq!(binding["slice_id"], environment_slice.id);
        let display = dispatch_json(
            &fixture.home,
            json!({"GetSliceDisplayEndpoint": {
                "slice_ref":"desktop",
                "session_id": &room,
                "attachment_id": attachment_id,
                "viewer_public_key": viewer_public_key
            }}),
        )
        .await
        .expect("public Web View request must resolve through the same home Environment");
        let endpoint = &display["SliceDisplayEndpoint"]["endpoint"];
        assert_eq!(endpoint["slice_id"], binding["slice_id"]);
        assert_eq!(endpoint["kind"], "selkies");
        assert_eq!(endpoint["stream_protocol"], "chariox-display-v1");
        assert_eq!(
            endpoint["peer_public_key"],
            fixture._worker_state.config.relay_public_key
        );
        let stream_id = endpoint["stream_id"].as_str().expect("Web View stream ID");
        assert_eq!(
            endpoint["url"],
            format!("ws://{}/display/{stream_id}/stream", fixture.address)
        );
        assert!(
            agent_worker
                .runtime_state
                .room_environment_snapshot(&room)
                .is_err(),
            "the remote kernel must not create parallel Room Environment authority"
        );
    })
    .catch_unwind()
    .await;

    let provider_cleanup = agent_worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    if let Some(home_agent_id) = home_agent_id {
        let destroyed = dispatch_json(
            &fixture.home,
            json!({"DestroyAgent": {
                "session_id": fixture.rooms[0],
                "agent_id": home_agent_id
            }}),
        )
        .await;
        destroyed.expect("remove the remote Room agent after provider cleanup");
    }
    let controller_cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    provider_cleanup.expect("stop the remote Room provider process");
    controller_cleanup.expect("stop the home slice Browser Controller");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

struct LeasedRoomProvider {
    home_agent_id: String,
    worker_provider_run_id: String,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    remote_execution: crate::agent::RemoteAgentBinding,
}

async fn prepare_home_slice_room_display(
    fixture: &LiveWorker,
) -> (String, String, String, crate::slice::SliceRecord) {
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
        .expect("mark the home-owned headed slice running");
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id":&room,
            "slice_ref":"desktop"
        }}),
    )
    .await
    .expect("bind the Room Environment to the home-owned local slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id":&room,
            "client_id":"remote-home-slice-viewer",
            "capability_level":"FullTerminal"
        }}),
    )
    .await
    .expect("attach a viewer for the public Web View request");
    let attachment_id = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("Room attachment ID")
        .to_string();
    let viewer_private_key = crate::transport::relay_crypto::generate_private_key_base64();
    let viewer_public_key =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private_key)
            .expect("Room viewer public key");
    let slice = fixture
        .home
        .runtime_state
        .resolve_slice("desktop")
        .expect("resolve the home-owned Environment slice");
    (room, attachment_id, viewer_public_key, slice)
}

async fn start_remote_agent_worker(fixture: &mut LiveWorker) -> (TestState, Arc<CommandRouter>) {
    let mut worker_state = TestState::new();
    worker_state.config.relay_url = fixture.home_state.config.relay_url.clone();
    worker_state.config.relay_token = fixture.home_state.config.relay_token.clone();
    worker_state.config.relay_heartbeat_ms = 50;
    worker_state.config.daemon_id = "agent-worker".to_string();
    worker_state.config.daemon_alias = Some("agent-worker".to_string());
    worker_state.config.host_machine_id = "agent-worker-machine".to_string();
    let worker = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(worker_state.config.clone())
                .expect("remote agent worker bootstrap"),
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

async fn wait_for_remote_agent_worker(fixture: &LiveWorker) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if send_peer_request_via_temporary_connection(
                &fixture.home_state.config,
                ClientTarget {
                    daemon_id: Some("agent-worker".to_string()),
                    daemon_alias: None,
                },
                RelayPeerRequest::Ping {
                    value: "room-remote-home-slice-ready".to_string(),
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
    .expect("independent remote agent kernel joins the relay");
}

async fn launch_remote_room_provider(
    fixture: &mut LiveWorker,
    agent_worker: &Arc<CommandRouter>,
    prompt: &str,
    home_agent_id_out: &mut Option<String>,
) -> LeasedRoomProvider {
    let room = fixture.rooms[0].clone();
    let worker_kernel_id = agent_worker.app.lock().await.config().daemon_id.clone();
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":room,
            "provider":"managed-dev-stub",
            "model":"runtime-mcp-idle",
            "kernel_ref":worker_kernel_id,
            "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn the Room agent on the separate remote worker kernel");
    let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .expect("home Room agent ID")
        .to_string();
    *home_agent_id_out = Some(home_agent_id.clone());
    let remote_execution: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .expect("home-owned remote worker binding");
    let leased_agent_id = remote_execution.leased_agent_id.clone();
    let (worker_relay_config, expected_profile, remote_extension_manifest) = {
        let app = fixture.home.app.lock().await;
        let agent = app
            .agents()
            .get_agent(&home_agent_id)
            .expect("home agent remains kernel-owned");
        (
            app.relay_config_for_remote_execution(&remote_execution),
            crate::transport::relay_peer::RelayAgentExecutionProfile::from(&agent),
            app.remote_extension_manifest_for_agent(&agent)
                .expect("project the home Room Environment capability to the remote agent"),
        )
    };
    let response = send_peer_request_via_temporary_connection(
        &worker_relay_config,
        ClientTarget {
            daemon_id: Some(worker_kernel_id),
            daemon_alias: None,
        },
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            expected_profile,
            prompt: prompt.to_string(),
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
    .expect("launch the remote provider through the relay lease");
    let RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: worker_provider_run_id,
        ..
    } = response
    else {
        panic!("unexpected leased provider response: {response:?}");
    };
    fixture
        .home
        .app
        .lock()
        .await
        .agents()
        .set_remote_execution_active_worker_provider_run_id(
            &home_agent_id,
            Some(worker_provider_run_id.clone()),
        )
        .expect("record the active remote provider run on the home agent");
    LeasedRoomProvider {
        home_agent_id,
        worker_provider_run_id,
        remote_extension_manifest,
        remote_execution,
    }
}

fn install_room_pointer_screen_tool(
    worker_root: &std::path::Path,
) -> (ScopedScreenToolEnvironment, std::path::PathBuf) {
    let screen_tool = worker_root.join("remote-home-slice-pointer-screen.sh");
    let screen_log = worker_root.join("remote-home-slice-pointer-screen.log");
    std::fs::write(
        &screen_tool,
        concat!(
            "#!/bin/sh\n",
            "set -eu\n",
            "[ \"$1\" = move ]\n",
            "printf '%s\\n' \"$*\" >> \"$CHARIOX_TEST_ROOM_POINTER_LOG\"\n",
        ),
    )
    .expect("write Room Computer helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&screen_tool, std::fs::Permissions::from_mode(0o700))
            .expect("make Room Computer helper executable");
    }
    let environment = ScopedScreenToolEnvironment::set([
        (
            "CHARIOX_SLICE_SCREEN_TOOL",
            screen_tool.as_os_str().to_os_string(),
        ),
        (
            "CHARIOX_TEST_ROOM_POINTER_LOG",
            screen_log.as_os_str().to_os_string(),
        ),
    ]);
    (environment, screen_log)
}

struct ScopedScreenToolEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl ScopedScreenToolEnvironment {
    fn set<const N: usize>(values: [(&'static str, std::ffi::OsString); N]) -> Self {
        let mut previous = Vec::with_capacity(N);
        for (name, value) in values {
            previous.push((name, std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        Self(previous)
    }
}

impl Drop for ScopedScreenToolEnvironment {
    fn drop(&mut self) {
        for (name, value) in self.0.drain(..).rev() {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}
