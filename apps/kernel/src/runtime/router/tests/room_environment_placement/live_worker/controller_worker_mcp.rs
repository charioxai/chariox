use super::*;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;
use futures_util::FutureExt;

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
