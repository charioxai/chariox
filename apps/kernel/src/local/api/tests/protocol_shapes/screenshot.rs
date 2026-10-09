use super::*;
#[test]
fn protocol_443_visible_region_capture_shapes() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 489);
    let value = serde_json::json!({"CaptureVisibleRegion":{"capture_id":"capture-1","surface":{"kind":"kernel_browser","tab_id":"host-tab-1","generation":2},"region":{"x":1,"y":2,"width":3,"height":4,"viewport_width":640,"viewport_height":400,"frame_width":1280,"frame_height":800}}});
    let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), value);
    let event = serde_json::json!({"VisibleRegionCaptured":{"capture":{"capture_id":"capture-1","surface":{"kind":"user_app_view","view_id":"view-1","generation":2},"captured_at_ms":12,"width":6,"height":8,"media_type":"image/png","display_name":"chariox-capture-12.png","sha256":"abc","data_base64":"AAAA"}}});
    let response: LocalDaemonResponse = serde_json::from_value(event.clone()).unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), event);
    let room = serde_json::json!({"CaptureVisibleRegion":{"capture_id":"capture-1","surface":{"kind":"room","session_id":"room-1","attachment_id":"attachment-1","runtime_generation":3,"viewport_revision":4},"region":value["CaptureVisibleRegion"]["region"]}});
    let app = serde_json::json!({"CaptureVisibleRegion":{"capture_id":"capture-1","surface":{"kind":"user_app_view","view_id":"view-1","generation":2},"region":value["CaptureVisibleRegion"]["region"]}});
    for request in [&room, &app] {
        let parsed: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *request);
    }
    let snapshot = serde_json::json!({"requests":[value,room,app],"event":event});
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "43747e012a1ccfe848fe9befd9712435e2a3bac8e7f492fca8fce618da653d1d"
    );
    let mut invalid = value.clone();
    invalid["CaptureVisibleRegion"]["surface"]["user_id"] = "another-owner".into();
    assert!(serde_json::from_value::<LocalDaemonRequest>(invalid).is_err());
}
