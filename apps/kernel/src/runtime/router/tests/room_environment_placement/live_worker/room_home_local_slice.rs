use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RemoteRoomBrowserRuntimeToolCall};
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn home_room_agent_uses_home_local_slice_environment_browser_computer_and_web_view() {
    run_test(check_home_room_agent_uses_home_local_slice_environment_browser_computer_and_web_view);
}

async fn check_home_room_agent_uses_home_local_slice_environment_browser_computer_and_web_view() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (_screen_environment, screen_log) =
        install_room_pointer_screen_tool(&fixture._worker_state.root);
    let check = std::panic::AssertUnwindSafe(async {
        let (room, attachment_id, viewer_public_key, environment_slice) =
            prepare_home_local_room_display(&fixture).await;
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

        let spawned = dispatch_json(
            &fixture.home,
            json!({"SpawnAgent": {
                "session_id": &room,
                "provider":"managed-dev-stub",
                "model":"runtime-mcp-idle"
            }}),
        )
        .await
        .expect("spawn the Room agent on the home kernel");
        let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
            .as_str()
            .expect("home Room agent ID")
            .to_string();
        assert_eq!(spawned["AgentSpawned"]["agent"]["session_id"], room);
        assert!(
            spawned["AgentSpawned"]["agent"]["remote_execution"].is_null(),
            "the Room agent must remain on the home kernel"
        );
        let after_spawn = fixture
            .home
            .runtime_state
            .resolve_slice("desktop")
            .expect("resolve the home-owned Environment slice");
        assert!(
            !after_spawn.agent_ids.contains(&home_agent_id),
            "the home Room agent must not be duplicated on the Environment slice"
        );

        let launched = dispatch_json(
            &fixture.home,
            json!({"LaunchProviderRun": {
                "session_id": &room,
                "agent_id": &home_agent_id,
                "adapter_key":"managed-dev-stub",
                "provider":"managed-dev-stub",
                "account_profile":"default",
                "model":"runtime-mcp-idle",
                "variant":null,
                "structured_endpoint":null,
                "provider_session_id":null,
                "native_tui":false
            }}),
        )
        .await
        .expect("launch the home-kernel Room agent provider run");
        assert!(
            launched.get("ProviderRunLaunchAccepted").is_some(),
            "{launched}"
        );
        let provider_run_id = {
            let app = fixture.home.app.lock().await;
            let agent = app
                .agents()
                .get_agent(&home_agent_id)
                .expect("Room agent remains home-owned");
            assert!(agent.remote_execution().is_none());
            app.providers()
                .get_latest_run_for_agent(&room, &home_agent_id)
                .expect("provider run belongs to the home Room agent")
                .id()
                .to_string()
        };
        let token = fixture
            .home
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&provider_run_id)
            .expect("home Room provider runtime token");
        let advertised = fixture
            .home
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for tool in ["slice_open_url", "slice_browser_status", "slice_mouse"] {
            assert!(advertised.contains(tool), "Room agent is missing {tool}");
        }

        let url = "https://home-local-slice.room-environment.test/";
        let browser = fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_open_url", json!({"url":url}))
            .await
            .expect("public Browser tool call must use home Room admission");
        assert!(browser.ok, "{:?}", browser.payload);
        assert_eq!(browser.payload["session_id"], room);
        assert_eq!(browser.payload["agent_id"], home_agent_id);
        assert_eq!(
            browser.payload["actor_id"],
            format!("agent:{home_agent_id}")
        );
        assert_eq!(browser.payload["browser"]["url"], url);
        let browser_action_id = browser.payload["action_id"]
            .as_str()
            .expect("Browser action ID")
            .to_string();

        let computer = fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "slice_mouse",
                json!({"action":"move","x":8,"y":8}),
            )
            .await
            .expect("public Computer tool call must use home Room admission");
        assert!(computer.ok, "{:?}", computer.payload);
        assert_eq!(computer.payload["session_id"], room);
        assert_eq!(computer.payload["environment_id"], environment_id);
        assert_eq!(computer.payload["agent_id"], home_agent_id);
        assert_eq!(
            computer.payload["actor_id"],
            format!("agent:{home_agent_id}")
        );
        assert_eq!(computer.payload["action_kind"], "pointer_move");
        let computer_action_id = computer.payload["action_id"]
            .as_str()
            .expect("Computer action ID")
            .to_string();
        assert_eq!(
            std::fs::read_to_string(&screen_log).expect("home-owned slice pointer helper log"),
            "move 8 8\n"
        );

        let status = fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_browser_status", json!({}))
            .await
            .expect("home Room agent reads the same Browser Tab");
        assert!(status.ok, "{:?}", status.payload);
        assert_eq!(status.payload["environment_id"], environment_id);
        assert_eq!(status.payload["tab_id"], stable_tab_id);
        assert_eq!(status.payload["url"], url);

        let before_denial = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{"session_id":&room}}),
        )
        .await
        .expect("read the public Environment state after Browser and Computer calls");
        let before_environment = &before_denial["RoomEnvironmentState"]["environment"];
        assert_eq!(before_environment["session_id"], room);
        assert_eq!(before_environment["environment_id"], environment_id);
        assert_eq!(before_environment["focused_tab_id"], stable_tab_id);
        let tabs = before_environment["tabs"].as_array().expect("Room Tabs");
        assert_eq!(tabs.len(), 1, "the Room must retain one stable Browser Tab");
        let stable_tab = tabs
            .iter()
            .find(|tab| tab["tab_id"] == stable_tab_id)
            .expect("public Room state contains the original Tab");
        assert_eq!(stable_tab["url"], url);
        let actions = before_environment["actions"]
            .as_array()
            .expect("Room action ledger");
        let actor_id = format!("agent:{home_agent_id}");
        for (action_id, action_kind) in [
            (&browser_action_id, "navigate"),
            (&computer_action_id, "pointer_move"),
        ] {
            let matching = actions
                .iter()
                .filter(|action| action["action_id"].as_str() == Some(action_id.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "each public tool action is recorded once"
            );
            assert_eq!(matching[0]["actor_id"], actor_id);
            assert_eq!(matching[0]["kind"], action_kind);
            assert_eq!(matching[0]["state"], "completed");
        }
        let action_count = actions.len();

        let wrong_room = send_peer_request_via_temporary_connection(
            &fixture._worker_state.config,
            ClientTarget {
                daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
                context: crate::transport::relay_peer::RemoteExtensionInvocationContext {
                    home_kernel_id: fixture.home_state.config.daemon_id.clone(),
                    home_session_id: fixture.rooms[1].clone(),
                    home_agent_id: home_agent_id.clone(),
                    leased_agent_id: "forged-foreign-room-lease".to_string(),
                    worker_provider_run_id: "forged-foreign-room-run".to_string(),
                    worker_kernel_id: Some(fixture._worker_state.config.daemon_id.clone()),
                    worker_machine_id: Some(fixture._worker_state.config.host_machine_id.clone()),
                },
                call: RemoteRoomBrowserRuntimeToolCall {
                    tool_name: "slice_open_url".to_string(),
                    arguments: json!({"url":"https://foreign-room.must-not-open/"}),
                },
            },
        )
        .await
        .expect_err("the home kernel must reject the agent in a foreign Room");
        assert!(
            wrong_room
                .to_string()
                .contains("agent does not belong to invocation session"),
            "foreign-Room request must fail at home admission: {wrong_room}"
        );

        let after_denial = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentState":{"session_id":&room}}),
        )
        .await
        .expect("read public Room state after the foreign-Room denial");
        let after_environment = &after_denial["RoomEnvironmentState"]["environment"];
        assert_eq!(after_environment["environment_id"], environment_id);
        assert_eq!(after_environment["focused_tab_id"], stable_tab_id);
        assert_eq!(
            after_environment["actions"].as_array().unwrap().len(),
            action_count
        );
        assert_eq!(
            after_environment["tabs"][0]["url"], url,
            "a foreign-Room request must not change the shared Browser Tab"
        );

        let binding = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentSlice":{"session_id":&room}}),
        )
        .await
        .expect("read public Room Environment slice binding");
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
        .expect("public Web View request must use the same Room Environment slice");
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
    })
    .catch_unwind()
    .await;

    let provider_cleanup = fixture
        .home
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
    provider_cleanup.expect("stop the home Room provider process");
    controller_cleanup.expect("stop the Environment slice Browser Controller");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

async fn prepare_home_local_room_display(
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
    .expect("bind the Room Environment to its home-owned local slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id":&room,
            "client_id":"home-local-slice-viewer",
            "capability_level":"FullTerminal"
        }}),
    )
    .await
    .expect("attach the Web View client to the Room");
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

fn install_room_pointer_screen_tool(
    worker_root: &std::path::Path,
) -> (ScopedScreenToolEnvironment, std::path::PathBuf) {
    let screen_tool = worker_root.join("home-local-room-pointer-screen.sh");
    let screen_log = worker_root.join("home-local-room-pointer-screen.log");
    std::fs::write(
        &screen_tool,
        concat!(
            "#!/bin/sh\n",
            "set -eu\n",
            "[ \"$1\" = move ]\n",
            "printf '%s\\n' \"$*\" >> \"$CHARIOX_TEST_ROOM_POINTER_LOG\"\n",
        ),
    )
    .expect("write the Room Computer helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&screen_tool, std::fs::Permissions::from_mode(0o700))
            .expect("make the Room Computer helper executable");
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
