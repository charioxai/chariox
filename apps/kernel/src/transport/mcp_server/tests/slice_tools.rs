use super::*;

#[cfg(unix)]
struct ControllerMcpFixture {
    root: std::path::PathBuf,
    previous_env: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(unix)]
impl ControllerMcpFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-controller-mcp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("test root should be created");
        let previous_env = [
            "CHARIOX_BROWSER_CONTROLLER_SCRIPT",
            "CHARIOX_BROWSER_CONTROLLER_NODE",
            "CHARIOX_SLICE_SCREEN_TOOL",
            "CHARIOX_HOME",
            "CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT",
            "CHARIOX_CONTROLLER_MCP_TEST_SECRET",
        ]
        .into_iter()
        .map(|name| (name, std::env::var_os(name)))
        .collect();
        Self { root, previous_env }
    }
}

#[cfg(unix)]
impl Drop for ControllerMcpFixture {
    fn drop(&mut self) {
        for (name, previous) in &self.previous_env {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn mcp_tools_list_exposes_slice_tools_only_for_slice_provider_tokens() {
    let mut config = DaemonConfig::for_tests();
    config.host_machine_id = "slice:slice-test".to_string();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await
    .expect("tools/list should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("tools list body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("tools list body json");
    let tools = value["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_screenshot"));
    assert!(tools.iter().any(|tool| tool["name"] == "slice_screenshot"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_find_text"));
    assert!(tools.iter().any(|tool| tool["name"] == "slice_mouse"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_status"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_status"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_wait_for_text"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_wait_for_idle"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_dialog"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_dialog"));
}

#[cfg(unix)]
#[tokio::test]
async fn mcp_tools_call_dispatches_slice_screen_fallbacks_inside_slice_kernel() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-slice-mcp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let tool = root.join("slice-screen.sh");
    std::fs::write(
        &tool,
        "#!/usr/bin/env bash\nset -euo pipefail\ncase \"${1:-}\" in\n  status) printf 'display=:99\\nscreen=1280x800\\nviewer=http://127.0.0.1:6080/vnc.html\\n' ;;\n  open-url) printf '{\"action_kind\":\"navigate\"}' ;;\n  browser-wait-selector) printf '{\"action_kind\":\"selector\",\"ok\":true}' ;;\n  browser-wait-idle) printf '{\"action_kind\":\"idle\",\"ok\":true}' ;;\n  *) exit 2 ;;\nesac\n",
    )
    .expect("fake screen tool should be written");
    let mut permissions = std::fs::metadata(&tool)
        .expect("fake tool metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool, permissions).expect("fake tool should be executable");
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &tool);

    let mut config = DaemonConfig::for_tests();
    config.host_machine_id = "slice:slice-test".to_string();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "slice_screen_status",
                "arguments": {}
            }
        }),
    )
    .await
    .expect("slice status call should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("slice status body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("slice status body json");
    assert_eq!(value["result"]["isError"], false, "{value:#}");
    assert_eq!(
        value["result"]["structuredContent"]["slice_id"],
        "slice-test"
    );
    assert_eq!(value["result"]["structuredContent"]["display"], ":99");
    assert_eq!(value["result"]["structuredContent"]["screen"], "1280x800");

    for (id, name, arguments) in [
        (
            2,
            "slice_open_url",
            serde_json::json!({"url": "https://example.test/legacy"}),
        ),
        (
            3,
            "slice_browser_wait_for_selector",
            serde_json::json!({"selector": "#legacy", "timeout_ms": 500}),
        ),
        (
            4,
            "slice_browser_wait_for_idle",
            serde_json::json!({"timeout_ms": 500}),
        ),
    ] {
        let fallback_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .await
        .expect("legacy fallback call should return an MCP response");
        let fallback_body = fallback_response
            .into_body()
            .collect()
            .await
            .expect("legacy fallback body should collect")
            .to_bytes();
        let fallback_value: Value =
            serde_json::from_slice(&fallback_body).expect("legacy fallback body json");
        assert_eq!(
            fallback_value["result"]["isError"], false,
            "{fallback_value:#}"
        );
        assert!(fallback_value["result"]["structuredContent"]["stdout"]
            .as_str()
            .is_some_and(|stdout| stdout.contains("action_kind")));
    }
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn mcp_browser_status_uses_the_room_owned_controller_instead_of_one_shot_cdp() {
    run_mcp_server_large_stack_test(
        "mcp-browser-tools-use-the-room-owned-controller",
        mcp_browser_status_uses_the_room_owned_controller_instead_of_one_shot_cdp_inner,
    );
}

#[cfg(unix)]
async fn mcp_browser_status_uses_the_room_owned_controller_instead_of_one_shot_cdp_inner() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::env_lock::lock();
    let fixture = ControllerMcpFixture::new();
    let controller = fixture.root.join("browser-controller.sh");
    let controller_pid = fixture.root.join("browser-controller.pid");
    let controller_log = fixture.root.join("controller.log");
    let one_shot = fixture.root.join("slice-screen.sh");
    let one_shot_log = fixture.root.join("one-shot.log");
    let controller_script = format!(
        r#"#!/bin/sh
set -eu
printf '%s\n' "$$" > '{}'
document=loader-a
url=https://example.test/dashboard
while IFS= read -r request; do
  id=${{request#*:}}
  id=${{id%%,*}}
  case "$request" in
    *'"method":"health"'*)
      printf '{{"id":%s,"ok":true,"result":{{"state":"ready","process_id":%s,"diagnostic_code":null}}}}\n' "$id" "$$"
      ;;
    *'"method":"browser.reconcile"'*)
      printf 'reconcile\n' >> '{}'
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"event_cursor":1,"tabs":[{{"target_id":"target-a","document_id":"%s","url":"%s","title":"Dashboard"}}],"focused_target_id":"target-a","viewport":{{"css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800}}}}}}\n' "$id" "$document" "$url"
      ;;
    *'"method":"browser.snapshot"'*)
      printf 'snapshot\n' >> '{}'
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"target_id":"target-a","document_id":"loader-a","snapshot_revision":1,"accessibility_nodes":[{{"node_ref":"backend:103","parent_ref":null,"child_refs":[],"role":"textbox","name":"Email","description":"","value":"","ignored":false,"disabled":false,"focused":true}},{{"node_ref":"backend:104","parent_ref":null,"child_refs":[],"role":"button","name":"Continue","description":"","value":"","ignored":false,"disabled":false,"focused":false}},{{"node_ref":"backend:105","parent_ref":null,"child_refs":[],"role":"link","name":"Help","description":"","value":"","ignored":false,"disabled":false,"focused":false}}],"dom_nodes":[{{"node_ref":"backend:103","parent_ref":null,"node_type":1,"node_name":"INPUT","text":"","attributes":{{"id":"email","name":"email","type":"email","placeholder":"Email"}},"bounds":{{"x":10,"y":20,"width":200,"height":30}}}},{{"node_ref":"backend:104","parent_ref":null,"node_type":1,"node_name":"BUTTON","text":"Continue","attributes":{{"id":"continue"}},"bounds":{{"x":10,"y":60,"width":100,"height":30}}}},{{"node_ref":"backend:105","parent_ref":null,"node_type":1,"node_name":"A","text":"Help","attributes":{{"id":"help","href":"/help"}},"bounds":{{"x":10,"y":100,"width":50,"height":20}}}}]}}}}\n' "$id"
      ;;
    *'"method":"browser.action"'*)
      if printf '%s' "$request" | grep -q '"kind":"fill"'; then action=fill;
      elif printf '%s' "$request" | grep -q '"kind":"submit"'; then action=submit;
      else action=click; fi
      printf '%s\n' "$action" >> '{}'
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"target_id":"target-a","document_id":"loader-a","action_kind":"%s","dialog_opened":false,"attempts":1,"elapsed_ms":7}}}}\n' "$id" "$action"
      ;;
    *'"method":"browser.dialog"'*)
      if printf '%s' "$request" | grep -q '"action":"accept"'; then action=accept; else action=dismiss; fi
      printf 'dialog-%s\n' "$action" >> '{}'
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"target_id":"target-a","document_id":"loader-a","action":"%s"}}}}\n' "$id" "$action"
      ;;
    *'"method":"browser.navigate"'*)
      printf 'navigate\n' >> '{}'
      document=loader-b
      url=https://example.test/settings
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"target_id":"target-a","document_id":"loader-b","url":"https://example.test/settings"}}}}\n' "$id"
      ;;
    *'"method":"browser.wait"'*)
      if printf '%s' "$request" | grep -q '"kind":"selector"'; then kind=selector; else kind=idle; fi
      printf 'wait-%s\n' "$kind" >> '{}'
      printf '{{"id":%s,"ok":true,"result":{{"browser_generation":1,"target_id":"target-a","document_id":"loader-b","kind":"%s","ok":true,"elapsed_ms":7}}}}\n' "$id" "$kind"
      ;;
    *'"method":"shutdown"'*)
      printf '{{"id":%s,"ok":true,"result":{{"state":"stopped","process_id":null,"diagnostic_code":null}}}}\n' "$id"
      exit 0
      ;;
  esac
done
"#,
        controller_pid.display(),
        controller_log.display(),
        controller_log.display(),
        controller_log.display(),
        controller_log.display(),
        controller_log.display(),
        controller_log.display()
    );
    std::fs::write(&controller, controller_script).expect("controller should be written");
    std::fs::write(
        &one_shot,
        format!(
            "#!/bin/sh\nprintf 'called %s\\n' \"$*\" >> '{}'\nexit 91\n",
            one_shot_log.display()
        ),
    )
    .expect("one-shot tool should be written");
    for path in [&controller, &one_shot] {
        let mut permissions = std::fs::metadata(path)
            .expect("test tool metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("test tool should be executable");
    }
    std::env::set_var("CHARIOX_BROWSER_CONTROLLER_SCRIPT", &controller);
    std::env::set_var("CHARIOX_BROWSER_CONTROLLER_NODE", "/bin/sh");
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &one_shot);
    std::env::set_var("CHARIOX_HOME", &fixture.root);
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    std::env::set_var(
        "CHARIOX_CONTROLLER_MCP_TEST_SECRET",
        "controller-mcp-secret-value",
    );
    crate::credential::CharioxCredentialRegistry::new(fixture.root.join("credentials"))
        .upsert(crate::config::UserCredentialConfig {
            id: "browser-login".to_string(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Env {
                name: "CHARIOX_CONTROLLER_MCP_TEST_SECRET".to_string(),
            },
            allowed_hosts: vec!["example.test".to_string()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: None,
        })
        .expect("test credential should be registered");

    let mut config = DaemonConfig::for_tests();
    config.host_machine_id = "slice:slice-test".to_string();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("controller-mcp".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("workflow node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("exercise controller MCP tools".to_string()),
    )
    .expect("workflow should invoke");
    let provider_run = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("workflow provider run should exist")
        .clone();
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();
    let session_id = session.id().to_string();

    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_status",
                "arguments": {}
            }
        }),
    )
    .await
    .expect("browser status request should return an MCP response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("browser status body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("browser status body json");
    assert_eq!(value["result"]["isError"], false, "{value:#}");
    assert_eq!(
        value["result"]["structuredContent"]["source"],
        "browser_controller"
    );
    assert_eq!(
        value["result"]["structuredContent"]["session_id"],
        session_id
    );
    assert_eq!(value["result"]["structuredContent"]["tab_id"], "tab-1");
    assert_eq!(
        value["result"]["structuredContent"]["url"],
        "https://example.test/dashboard"
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["url"],
        "https://example.test/dashboard"
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["host"],
        "example.test"
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["focusedElement"]["label"],
        "Email"
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["fields"][0]["field_id"]
            .as_str()
            .is_some_and(|field_id| field_id.starts_with("element-")),
        true
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["buttons"][0]["text"],
        "Continue"
    );
    assert_eq!(
        value["result"]["structuredContent"]["browser"]["links"][0]["text"],
        "Help"
    );
    assert!(
        !value.to_string().contains("backend:"),
        "controller-local node references must not escape through MCP"
    );
    let find_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_find",
                "arguments": {"query": "cont", "kind": "button"}
            }
        }),
    )
    .await
    .expect("browser find request should return an MCP response");
    let find_body = find_response
        .into_body()
        .collect()
        .await
        .expect("browser find body should collect")
        .to_bytes();
    let find_value: Value = serde_json::from_slice(&find_body).expect("browser find body json");
    assert_eq!(find_value["result"]["isError"], false, "{find_value:#}");
    assert_eq!(
        find_value["result"]["structuredContent"]["browser"]["matches"][0]["text"],
        "Continue"
    );
    assert_eq!(
        find_value["result"]["structuredContent"]["browser"]["matches"][0]["field_id"],
        value["result"]["structuredContent"]["browser"]["buttons"][0]["field_id"]
    );
    let mut field_id = value["result"]["structuredContent"]["browser"]["fields"][0]["field_id"]
        .as_str()
        .expect("status should expose an opaque field id")
        .to_string();
    let button_id = value["result"]["structuredContent"]["browser"]["buttons"][0]["field_id"]
        .as_str()
        .expect("status should expose an opaque button id")
        .to_string();
    for (id, name, arguments, action) in [
        (
            3,
            "slice_browser_fill",
            serde_json::json!({"field_id": field_id.clone(), "text": "person@example.test"}),
            "fill",
        ),
        (
            4,
            "slice_browser_click",
            serde_json::json!({"field_id": button_id}),
            "click",
        ),
        (
            5,
            "slice_browser_submit",
            serde_json::json!({"field_id": field_id.clone()}),
            "submit",
        ),
    ] {
        let action_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .await
        .expect("browser action should return an MCP response");
        let body = action_response
            .into_body()
            .collect()
            .await
            .expect("browser action body should collect")
            .to_bytes();
        let action_value: Value = serde_json::from_slice(&body).expect("browser action body json");
        assert_eq!(action_value["result"]["isError"], false, "{action_value:#}");
        assert_eq!(
            action_value["result"]["structuredContent"]["browser"]["action_kind"],
            action
        );
        assert_eq!(
            action_value["result"]["structuredContent"]["actor_id"],
            crate::session::agent_environment_actor_id(&agent_id)
        );
        assert!(action_value["result"]["structuredContent"]["action_id"]
            .as_str()
            .is_some_and(|action_id| action_id.starts_with("action-")));
    }
    let first_controller_pid = std::fs::read_to_string(&controller_pid)
        .expect("controller pid should exist")
        .trim()
        .parse::<u32>()
        .expect("controller pid should be numeric");
    assert!(std::process::Command::new("/bin/kill")
        .args(["-9", &first_controller_pid.to_string()])
        .status()
        .expect("controller kill should run")
        .success());
    let recovered_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 51,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_status",
                "arguments": {}
            }
        }),
    )
    .await
    .expect("browser status should recover the killed controller");
    let recovered_body = recovered_response
        .into_body()
        .collect()
        .await
        .expect("recovered browser status body should collect")
        .to_bytes();
    let recovered_value: Value =
        serde_json::from_slice(&recovered_body).expect("recovered browser status body json");
    assert_eq!(
        recovered_value["result"]["isError"], false,
        "{recovered_value:#}"
    );
    assert_eq!(
        recovered_value["result"]["structuredContent"]["tab_id"],
        "tab-1"
    );
    assert_eq!(
        recovered_value["result"]["structuredContent"]["runtime_generation"],
        1
    );
    let recovered_field_id = recovered_value["result"]["structuredContent"]["browser"]["fields"][0]
        ["field_id"]
        .as_str()
        .expect("recovered status should expose a new opaque field id")
        .to_string();
    assert_ne!(recovered_field_id, field_id);
    assert!(matches!(
        router
            .runtime_state()
            .resolve_room_environment_element_reference(&session_id, &field_id),
        Err(crate::session::EnvironmentError::StaleElementReference { .. })
    ));
    field_id = recovered_field_id;
    let recovered_controller_pid = std::fs::read_to_string(&controller_pid)
        .expect("recovered controller pid should exist")
        .trim()
        .parse::<u32>()
        .expect("recovered controller pid should be numeric");
    assert_ne!(recovered_controller_pid, first_controller_pid);
    let recovered_environment = router
        .runtime_state()
        .room_environment_snapshot(&session_id)
        .expect("Room Environment should recover");
    assert_eq!(recovered_environment.tabs.len(), 1);
    assert_eq!(recovered_environment.tabs[0].tab_id, "tab-1");
    let recovered_actor_ids = recovered_environment
        .actors
        .iter()
        .map(|actor| actor.actor_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        recovered_actor_ids.len(),
        recovered_environment.actors.len(),
        "controller recovery must not duplicate Actors"
    );
    assert!(recovered_actor_ids
        .contains(crate::session::agent_environment_actor_id(&agent_id).as_str()));
    assert_eq!(recovered_environment.actions.len(), 3);
    assert!(recovered_environment
        .actions
        .iter()
        .all(|action| action.state == crate::session::EnvironmentActionState::Completed));
    let text_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "slice_browser_text", "arguments": {}}
        }),
    )
    .await
    .expect("browser text should return an MCP response");
    let text_body = text_response
        .into_body()
        .collect()
        .await
        .expect("browser text body should collect")
        .to_bytes();
    let text_value: Value = serde_json::from_slice(&text_body).expect("browser text body json");
    assert_eq!(text_value["result"]["isError"], false, "{text_value:#}");
    assert!(text_value["result"]["structuredContent"]["text"]
        .as_str()
        .is_some_and(|text| text.contains("Continue")));
    let wait_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_wait_for_text",
                "arguments": {"text": "Continue", "timeout_ms": 100}
            }
        }),
    )
    .await
    .expect("browser wait should return an MCP response");
    let wait_body = wait_response
        .into_body()
        .collect()
        .await
        .expect("browser wait body should collect")
        .to_bytes();
    let wait_value: Value = serde_json::from_slice(&wait_body).expect("browser wait body json");
    assert_eq!(wait_value["result"]["isError"], false, "{wait_value:#}");
    assert_eq!(
        wait_value["result"]["structuredContent"]["browser"]["ok"],
        true
    );
    let missing_wait_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 70,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_wait_for_text",
                "arguments": {"text": "Missing", "timeout_ms": 100}
            }
        }),
    )
    .await
    .expect("missing browser text wait should return an MCP response");
    let missing_wait_body = missing_wait_response
        .into_body()
        .collect()
        .await
        .expect("missing browser text wait body should collect")
        .to_bytes();
    let missing_wait_value: Value =
        serde_json::from_slice(&missing_wait_body).expect("missing browser text wait body json");
    assert_eq!(
        missing_wait_value["result"]["structuredContent"]["browser"]["ok"],
        false
    );
    let secret_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "paste_secret_to_slice",
                "arguments": {
                    "credential_id": "browser-login",
                    "field_id": field_id,
                    "expected_url": "https://example.test/dashboard",
                    "expected_host": "example.test",
                    "submit": true
                }
            }
        }),
    )
    .await
    .expect("secret paste should return an MCP response");
    let secret_body = secret_response
        .into_body()
        .collect()
        .await
        .expect("secret paste body should collect")
        .to_bytes();
    let secret_value: Value = serde_json::from_slice(&secret_body).expect("secret paste body json");
    assert_eq!(secret_value["result"]["isError"], false, "{secret_value:#}");
    assert_eq!(
        secret_value["result"]["structuredContent"]["credential_id"],
        "browser-login"
    );
    assert_eq!(
        secret_value["result"]["structuredContent"]["submitted"],
        true
    );
    assert_eq!(
        secret_value["result"]["structuredContent"]["actor_id"],
        crate::session::agent_environment_actor_id(&agent_id)
    );
    assert!(
        !secret_value
            .to_string()
            .contains("controller-mcp-secret-value"),
        "secret material must not escape through MCP"
    );
    let dialog_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "slice_browser_dialog",
                "arguments": {"action": "dismiss"}
            }
        }),
    )
    .await
    .expect("browser dialog should return an MCP response");
    let dialog_body = dialog_response
        .into_body()
        .collect()
        .await
        .expect("browser dialog body should collect")
        .to_bytes();
    let dialog_value: Value =
        serde_json::from_slice(&dialog_body).expect("browser dialog body json");
    assert_eq!(dialog_value["result"]["isError"], false, "{dialog_value:#}");
    assert_eq!(
        dialog_value["result"]["structuredContent"]["browser"]["action"],
        "dismiss"
    );
    assert_eq!(
        dialog_value["result"]["structuredContent"]["actor_id"],
        crate::session::agent_environment_actor_id(&agent_id)
    );
    for (id, name, arguments, expected_kind) in [
        (
            10,
            "slice_open_url",
            serde_json::json!({"url": "https://example.test/settings"}),
            "navigate",
        ),
        (
            11,
            "slice_browser_wait_for_selector",
            serde_json::json!({"selector": "#settings", "timeout_ms": 500}),
            "selector",
        ),
        (
            12,
            "slice_browser_wait_for_idle",
            serde_json::json!({"timeout_ms": 500}),
            "idle",
        ),
    ] {
        let compatibility_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .await
        .expect("legacy browser call should return an MCP response");
        let body = compatibility_response
            .into_body()
            .collect()
            .await
            .expect("legacy browser response body should collect")
            .to_bytes();
        let compatibility_value: Value =
            serde_json::from_slice(&body).expect("legacy browser response body json");
        assert_eq!(
            compatibility_value["result"]["isError"], false,
            "{compatibility_value:#}"
        );
        assert_eq!(
            compatibility_value["result"]["structuredContent"]["source"],
            "browser_controller"
        );
        assert_eq!(
            compatibility_value["result"]["structuredContent"]["browser"]["action_kind"],
            expected_kind
        );
        if expected_kind == "navigate" {
            assert_eq!(
                compatibility_value["result"]["structuredContent"]["actor_id"],
                crate::session::agent_environment_actor_id(&agent_id)
            );
            assert!(
                compatibility_value["result"]["structuredContent"]["action_id"]
                    .as_str()
                    .is_some_and(|action_id| action_id.starts_with("action-"))
            );
        }
    }
    let environment = router
        .runtime_state()
        .room_environment_snapshot(&session_id)
        .expect("Room Environment should remain available");
    assert_eq!(
        environment
            .actions
            .iter()
            .map(|action| action.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["fill", "click", "submit", "fill", "dialog", "navigate"]
    );
    assert!(environment.actions.iter().all(|action| {
        action.actor_id == crate::session::agent_environment_actor_id(&agent_id)
            && action.state == crate::session::EnvironmentActionState::Completed
    }));
    assert_eq!(
        std::fs::read_to_string(&controller_log).expect("controller log should exist"),
        "reconcile\nsnapshot\nreconcile\nsnapshot\nfill\nclick\nsubmit\nreconcile\nsnapshot\nreconcile\nsnapshot\nreconcile\nsnapshot\nreconcile\nsnapshot\nsnapshot\nreconcile\nsnapshot\nfill\nreconcile\ndialog-dismiss\nreconcile\nnavigate\nreconcile\nreconcile\nwait-selector\nreconcile\nwait-idle\n"
    );
    assert!(
        !std::fs::read_to_string(&controller_log)
            .expect("controller log should remain readable")
            .contains("controller-mcp-secret-value"),
        "secret material must not be written to the controller log"
    );
    assert!(
        !one_shot_log.exists(),
        "the one-shot browser command must not run when the controller is enabled"
    );
}
