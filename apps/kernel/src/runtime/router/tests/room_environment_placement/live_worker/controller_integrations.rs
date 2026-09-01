use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let tab_id = status["tab_id"].as_str().expect("focused Room tab");

    let snapshot = runtime
        .capture_browser_environment_snapshot(room, tab_id)
        .await
        .expect("nested browser structure crosses the bound worker relay");
    let frame = snapshot
        .dom_documents
        .iter()
        .find(|document| document.owner_element_ref.is_some())
        .expect("nested frame document");
    let frame_owner = frame
        .owner_element_ref
        .as_deref()
        .expect("nested frame owner reference");
    assert!(snapshot
        .dom_nodes
        .iter()
        .any(|node| node.element_ref == frame_owner));
    let frame_button = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.node_name == "BUTTON"
                && node
                    .bounds
                    .is_some_and(|bounds| bounds.x == 20.0 && bounds.y == 110.0)
        })
        .expect("button inside nested frame");
    let shadow = snapshot.shadow_roots.first().expect("open shadow root");
    let shadow_button = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.parent_ref.as_deref() == Some(shadow.element_ref.as_str())
                && node.node_name == "BUTTON"
        })
        .expect("button inside shadow root");
    let upload_field = snapshot
        .dom_nodes
        .iter()
        .find(|node| {
            node.node_name == "INPUT"
                && node
                    .attributes
                    .get("type")
                    .is_some_and(|kind| kind == "file")
        })
        .expect("worker upload field")
        .element_ref
        .clone();

    let agent_id = status["agent_id"].as_str().expect("Room agent id");
    assert!(runtime
        .runtime_tool_specs_for_auth_token(token)
        .iter()
        .any(|spec| spec.name == "slice_browser_dialog"));
    for name in [
        "slice_browser_events",
        "slice_browser_downloads",
        "slice_browser_upload",
        "slice_browser_permission",
    ] {
        assert!(
            runtime
                .runtime_tool_specs_for_auth_token(token)
                .iter()
                .any(|spec| spec.name == name),
            "bound Room runtime MCP omitted {name}"
        );
    }
    let clicked = runtime
        .perform_browser_environment_locator_action_as_agent(
            room,
            agent_id,
            &shadow_button.element_ref,
            crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
            crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
        )
        .await
        .expect("shadow-root action crosses the bound worker relay");
    assert_eq!(clicked.value.element_ref, shadow_button.element_ref);
    let frame_clicked = runtime
        .perform_browser_environment_locator_action_as_agent(
            room,
            agent_id,
            &frame_button.element_ref,
            crate::runtime::browser_controller_action::BrowserLocatorAction::Click,
            crate::runtime::browser_controller_action::MAX_BROWSER_ACTION_TIMEOUT_MS,
        )
        .await
        .expect("nested-frame action crosses the bound worker relay");
    assert_eq!(frame_clicked.value.element_ref, frame_button.element_ref);

    let dialog = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_dialog",
            json!({"action":"accept","prompt_text":"approved by home"}),
        )
        .await
        .expect("public dialog tool reaches the bound worker controller");
    assert!(dialog.ok, "{:?}", dialog.payload);
    assert_eq!(dialog.payload["browser"]["action"], "accept");

    let downloads = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_downloads", json!({}))
        .await
        .expect("runtime MCP download configuration reaches the bound worker controller");
    assert!(downloads.ok, "{:?}", downloads.payload);
    assert_eq!(downloads.payload["enabled"], true);
    assert_eq!(downloads.payload["tab_id"], tab_id);

    let upload_path = fixture._worker_state.root.join("relay-upload.txt");
    std::fs::write(&upload_path, b"relay upload").expect("write bounded upload fixture");
    let upload = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_upload",
            json!({"field_id": upload_field, "files": [upload_path.clone()]}),
        )
        .await
        .expect("runtime MCP file upload reaches the bound worker controller");
    assert!(upload.ok, "{:?}", upload.payload);
    assert_eq!(upload.payload["file_count"], 1);
    assert_eq!(upload.payload["total_bytes"], 12);
    assert!(
        !upload.payload.to_string().contains("relay-upload.txt"),
        "upload paths must not return through runtime MCP"
    );

    let permission = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_permission",
            json!({"permission": "geolocation", "setting": "denied"}),
        )
        .await
        .expect("runtime MCP permission decision reaches the bound worker controller");
    assert!(permission.ok, "{:?}", permission.payload);
    assert_eq!(permission.payload["permission"], "geolocation");
    assert_eq!(permission.payload["setting"], "denied");

    let physical = std::fs::read_to_string(fixture._worker_state.root.join("chromium-state.json"))
        .expect("worker browser state");
    let physical: Value = serde_json::from_str(&physical).expect("worker browser state JSON");
    assert_eq!(physical["shadowClicked"], true);
    assert_eq!(physical["frameClicked"], true);
    assert_eq!(physical["dialog"]["accept"], true);
    assert_eq!(physical["dialog"]["promptText"], "approved by home");
    assert_eq!(physical["downloads"]["behavior"], "allowAndName");
    assert_eq!(physical["upload"]["backendNodeId"], 104);
    assert_eq!(physical["upload"]["fileCount"], 1);
    assert_eq!(physical["permission"]["setting"], "denied");
    assert_eq!(physical["permission"]["origin"], "https://worker.test");

    std::fs::remove_file(upload_path).expect("remove upload fixture");
}
