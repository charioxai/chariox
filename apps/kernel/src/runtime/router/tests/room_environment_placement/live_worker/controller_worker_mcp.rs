use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

#[test]
fn worker_computer_secret_uses_home_room_authority() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _env_guard = crate::env_lock::lock();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("computer secret test runtime")
                .block_on(worker_computer_secret_scenario());
        })
        .expect("computer secret test thread")
        .join()
        .expect("computer secret test thread should not panic");
}

async fn worker_computer_secret_scenario() {
    let mut fixture = LiveWorker::start_configured_with_home_vault(
        false,
        true,
        Some(crate::config::CredentialVaultBackend::ProcessMemory),
    )
    .await;
    let check = std::panic::AssertUnwindSafe(check_worker_computer_secret(&mut fixture))
        .catch_unwind()
        .await;
    let controller_cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    let provider_cleanup = fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    controller_cleanup.expect("stop worker controller after computer secret test");
    provider_cleanup.expect("stop worker provider after computer secret test");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

async fn check_worker_computer_secret(fixture: &mut LiveWorker) {
    const SECRET: &str = "remote-room-computer-secret-canary";
    let screen_tool = fixture._worker_state.root.join("computer-secret-screen.sh");
    let screen_log = fixture
        ._worker_state
        .root
        .join("computer-secret-screen.log");
    std::fs::write(
        &screen_tool,
        "#!/bin/sh\nset -eu\n[ \"${1:-}\" = computer-secret-paste-stdin ]\ninput=$(cat)\n[ \"$input\" = \"$CHARIOX_REMOTE_ROOM_COMPUTER_SECRET\" ]\nprintf 'computer-secret-input-ok\\n' >> \"$CHARIOX_REMOTE_ROOM_COMPUTER_LOG\"\n",
    )
    .expect("computer secret screen helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&screen_tool, std::fs::Permissions::from_mode(0o700))
            .expect("computer secret screen helper permissions");
    }
    let _environment = ScopedEnvironment::set([
        (
            "CHARIOX_HOME",
            fixture.home_state.root.as_os_str().to_os_string(),
        ),
        ("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1".into()),
        ("CHARIOX_REMOTE_ROOM_COMPUTER_SECRET", SECRET.into()),
        (
            "CHARIOX_REMOTE_ROOM_COMPUTER_LOG",
            screen_log.as_os_str().to_os_string(),
        ),
        (
            "CHARIOX_SLICE_SCREEN_TOOL",
            screen_tool.as_os_str().to_os_string(),
        ),
    ]);
    crate::credential::CharioxCredentialRegistry::user()
        .expect("isolated credential registry")
        .upsert(crate::config::UserCredentialConfig {
            id: "remote-desktop-login".to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Env {
                name: "CHARIOX_REMOTE_ROOM_COMPUTER_SECRET".to_string(),
            },
            allowed_hosts: Vec::new(),
            allowed_uses: vec![crate::config::UserCredentialUse::Computer],
            injection: crate::config::UserCredentialInjectionConfig::Computer,
            metadata: None,
        })
        .expect("register the isolated computer credential");

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
        .expect("test slice running");
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id":room, "slice_ref":"desktop"
        }}),
    )
    .await
    .expect("bind the home Room to its worker slice");
    dispatch_json(
        &fixture.home,
        json!({"StartRoomEnvironment": {
            "session_id":room, "viewport": {
                "css_width":1280, "css_height":800, "device_scale_factor":1,
                "desktop_pixel_width":1280, "desktop_pixel_height":800
            }
        }}),
    )
    .await
    .expect("start the home-owned Room environment");

    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":room, "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased Room agent on the worker");
    let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .expect("home agent id")
        .to_string();
    let leased_agent_id = spawned["AgentSpawned"]["agent"]["remote_execution"]["leased_agent_id"]
        .as_str()
        .expect("leased agent id")
        .to_string();
    let remote_execution: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .expect("remote execution binding");
    let worker_relay_config = fixture
        .home
        .app
        .lock()
        .await
        .relay_config_for_remote_execution(&remote_execution);
    let response = send_peer_request_via_temporary_connection(
        &worker_relay_config,
        ClientTarget {
            daemon_id: Some("environment-worker".to_string()),
            daemon_alias: None,
        },
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt: "launch the worker provider for the computer secret test".to_string(),
            hidden_system_context: String::new(),
            attachments: Vec::new(),
            workflow_context: None,
            git_context: None,
            required_mcps: Vec::new(),
            required_skills: None,
            remote_extension_manifest: Default::default(),
        },
    )
    .await
    .expect("launch the worker provider through the relay");
    let RelayPeerResponse::LeasedPromptSubmitted {
        provider_run_id: worker_provider_run_id,
        ..
    } = response
    else {
        panic!("unexpected leased prompt response: {response:?}")
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
        .expect("record the active worker provider run");
    let worker_session_id = fixture
        .worker
        .app
        .lock()
        .await
        .providers()
        .get_run(&worker_provider_run_id)
        .expect("worker provider run")
        .session_id()
        .to_string();
    let token = fixture
        .worker
        .runtime_state
        .runtime_mcp_auth_token_for_provider_run(&worker_provider_run_id)
        .expect("worker provider runtime MCP token");

    let runtime = fixture.worker.runtime_state.clone();
    let call = tokio::spawn(async move {
        runtime
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "paste_secret_to_computer",
                json!({"credential_id":"remote-desktop-login"}),
            )
            .await
    });
    let interaction_id = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let session = fixture
                .home
                .runtime_state
                .session_snapshot_projection(&room, 0)
                .expect("home Room projection")
                .session;
            if let Some(interaction) = session
                .active_interactions()
                .iter()
                .find(|interaction| interaction.title() == Some("Computer credential input"))
            {
                break interaction.id().to_string();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("computer credential approval appears on home");
    fixture
        .home
        .runtime_state
        .resolve_runtime_interaction(&room, &interaction_id, "allow", None)
        .await
        .expect("approve the computer credential on home");
    let result = call
        .await
        .expect("computer secret task joins")
        .expect("worker computer secret uses home Room authority");

    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["target"], "desktop_focus");
    assert_eq!(
        result.payload["actor_id"],
        crate::session::agent_environment_actor_id(&home_agent_id)
    );
    assert!(
        !serde_json::to_string(&result.payload)
            .expect("serialize runtime tool payload")
            .contains(SECRET),
        "secret material must not cross the runtime tool result"
    );
    assert_eq!(
        std::fs::read_to_string(&screen_log).expect("worker secret input log"),
        "computer-secret-input-ok\n"
    );
    let home_environment = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .expect("home Room keeps the action ledger");
    let action = home_environment
        .actions
        .iter()
        .find(|action| action.kind == "secret_input")
        .expect("home Room records computer secret input");
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );
    assert_eq!(action.arguments, None);
    assert!(
        fixture
            .worker
            .runtime_state
            .room_environment_snapshot(&worker_session_id)
            .is_err(),
        "worker must not create parallel Room authority"
    );

    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":room,"agent_id":home_agent_id}}),
    )
    .await
    .expect("destroy the leased agent after the computer secret test");
    dispatch_json(
        &fixture.home,
        json!({"StopRoomEnvironment":{"session_id":room}}),
    )
    .await
    .expect("stop the home Room environment after the computer secret test");
}

struct ScopedEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl ScopedEnvironment {
    fn set<const N: usize>(values: [(&'static str, std::ffi::OsString); N]) -> Self {
        let mut previous = Vec::with_capacity(N);
        for (name, value) in values {
            previous.push((name, std::env::var_os(name)));
            std::env::set_var(name, value);
        }
        Self(previous)
    }
}

impl Drop for ScopedEnvironment {
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

pub(super) async fn check(fixture: &LiveWorker, placement: Value) {
    let room = fixture.rooms[0].clone();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent": {
            "session_id":room,
            "provider":"managed-dev-stub",
            "model":"default",
            "slice_ref":"desktop",
            "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased Room agent on the browser worker");
    let home_agent_id = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .expect("home agent id")
        .to_string();
    let leased_agent_id = spawned["AgentSpawned"]["agent"]["remote_execution"]["leased_agent_id"]
        .as_str()
        .expect("leased agent id")
        .to_string();
    let remote_execution: crate::agent::RemoteAgentBinding =
        serde_json::from_value(spawned["AgentSpawned"]["agent"]["remote_execution"].clone())
            .expect("remote execution binding");
    let worker_relay_config = {
        let app = fixture.home.app.lock().await;
        app.relay_config_for_remote_execution(&remote_execution)
    };

    let check_result = std::panic::AssertUnwindSafe(async {
        let response = send_peer_request_via_temporary_connection(
            &worker_relay_config,
            ClientTarget {
                daemon_id: Some("environment-worker".to_string()),
                daemon_alias: None,
            },
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id: leased_agent_id.clone(),
                prompt: "launch the worker provider for the runtime MCP drill".to_string(),
                hidden_system_context: String::new(),
                attachments: Vec::new(),
                workflow_context: None,
                git_context: None,
                required_mcps: Vec::new(),
                required_skills: None,
                remote_extension_manifest: Default::default(),
            },
        )
        .await
        .expect("submit the leased prompt through the authenticated relay");
        let RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: worker_provider_run_id,
            ..
        } = response
        else {
            panic!("unexpected leased prompt response: {response:?}")
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
            .expect("record the acknowledged worker provider on the home binding");
        let worker_session_id = {
            let app = fixture.worker.app.lock().await;
            let run = app
                .providers()
                .get_run(&worker_provider_run_id)
                .expect("worker provider run");
            run.session_id().to_string()
        };
        let token = fixture
            .worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&worker_provider_run_id)
            .expect("worker provider runtime MCP token");
        let advertised = fixture
            .worker
            .runtime_state
            .runtime_tool_specs_for_auth_token(&token)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<std::collections::BTreeSet<_>>();
        for expected in [
            "slice_browser_status",
            "slice_open_url",
            "slice_browser_click",
            "slice_browser_fill",
            "slice_browser_submit",
            "slice_browser_dialog",
            "slice_browser_events",
            "slice_browser_downloads",
            "slice_browser_upload",
            "slice_browser_permission",
            "slice_browser_find",
            "slice_browser_text",
            "slice_browser_wait_for_text",
            "slice_browser_wait_for_selector",
            "slice_browser_wait_for_idle",
        ] {
            assert!(
                advertised.contains(expected),
                "missing worker tool {expected}"
            );
        }
        let denied = send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            ClientTarget {
                daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardRoomBrowserRuntimeTool {
                context: crate::transport::relay_peer::RemoteExtensionInvocationContext {
                    home_kernel_id: fixture.home_state.config.daemon_id.clone(),
                    home_session_id: room.clone(),
                    home_agent_id: home_agent_id.clone(),
                    leased_agent_id: leased_agent_id.clone(),
                    worker_provider_run_id: worker_provider_run_id.clone(),
                    worker_kernel_id: Some("environment-worker".to_string()),
                    worker_machine_id: Some("slice:slice-1".to_string()),
                },
                call: crate::transport::relay_peer::RemoteRoomBrowserRuntimeToolCall {
                    tool_name: "slice_browser_status".to_string(),
                    arguments: json!({}),
                },
            },
        )
        .await
        .expect_err("a non-worker relay sender must not exercise the Room browser");
        assert!(
            denied
                .to_string()
                .contains("relay sender does not match the bound worker kernel"),
            "{denied}"
        );
        let url = "https://worker-agent.worker.test/path?runtime=mcp";
        let result = fixture
            .worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(&token, "slice_open_url", json!({"url":url}))
            .await
            .expect("worker provider browser MCP call forwards to the home Room");
        assert!(result.ok, "{:?}", result.payload);
        assert_eq!(result.payload["session_id"], room);
        assert_eq!(result.payload["agent_id"], home_agent_id);
        assert_eq!(result.payload["actor_id"], format!("agent:{home_agent_id}"));
        assert!(result.payload["action_id"]
            .as_str()
            .is_some_and(|action_id| !action_id.is_empty()));
        assert_eq!(result.payload["browser"]["url"], url);

        let environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(&room)
            .expect("home Room after worker-provider navigation");
        let focused = environment
            .focused_tab_id
            .as_deref()
            .and_then(|focused| environment.tabs.iter().find(|tab| tab.tab_id == focused))
            .expect("focused home Room tab");
        assert_eq!(focused.url, url);
        assert!(
            fixture
                .worker
                .runtime_state
                .room_environment_snapshot(&worker_session_id)
                .is_err(),
            "worker MCP forwarding must not create parallel Room authority"
        );
    })
    .catch_unwind()
    .await;

    let cleanup = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":room,"agent_id":home_agent_id}}),
    )
    .await;
    if let Err(panic) = check_result {
        let _ = cleanup;
        std::panic::resume_unwind(panic);
    }
    cleanup.expect("destroy the leased worker agent after the MCP drill");
}
