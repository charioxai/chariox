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
        .configure_browser_environment_downloads(room, tab_id)
        .await
        .expect("download configuration reaches the bound worker controller");
    assert!(downloads.enabled);

    let upload_path = fixture._worker_state.root.join("relay-upload.txt");
    std::fs::write(&upload_path, b"relay upload").expect("write bounded upload fixture");
    let upload = runtime
        .upload_browser_environment_files(room, &upload_field, vec![upload_path.clone()])
        .await
        .expect("file upload reaches the bound worker controller");
    assert_eq!(upload.file_count, 1);
    assert_eq!(upload.total_bytes, 12);

    let permission = runtime
        .set_browser_environment_permission(
            room,
            tab_id,
            crate::runtime::browser_controller_permission::BrowserPermissionName::Geolocation,
            crate::runtime::browser_controller_permission::BrowserPermissionSetting::Denied,
        )
        .await
        .expect("permission decision reaches the bound worker controller");
    assert_eq!(permission.permission, "geolocation");
    assert_eq!(permission.setting, "denied");

    let popup_environment = dispatch_json(
        &fixture.home,
        json!({"StartRoomEnvironment": {
            "session_id": room,
            "viewport": {
                "css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                "desktop_pixel_width": 1280, "desktop_pixel_height": 800
            }
        }}),
    )
    .await
    .expect("popup target reconciles through the bound worker");
    assert!(
        popup_environment["RoomEnvironmentUpdated"]["environment"]["tabs"]
            .as_array()
            .expect("Room tabs")
            .iter()
            .any(|tab| {
                tab["url"] == "https://popup.worker.test/" && tab["title"] == "Worker popup"
            })
    );

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
