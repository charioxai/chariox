//! Local 1016 (row 652): the owner view's publication may carry the viewer's delivered Browser stats and one picture, and its
//! status counts the pictures the watching agents asked for (additive: an older client's publication and status still parse).
use super::*;

#[test]
fn an_owner_view_publication_carries_viewer_stats_and_a_picture_and_an_old_one_still_parses() {
    let wire = serde_json::json!({ "OwnerView": {
        "operation": "publish",
        "generation": "0123456789abcdef0123456789abcdef",
        "tab_id": "e400ab77-217b-4509-b64b-c1cf40551a90",
        "rows": { "navigator": ["Room"], "settings": [] },
        "browser": {
            "viewer": { "kind": "owner_tab", "device_pixel_ratio": 2.0 },
            "path": "relay",
            "window_ms": 1000,
            "panes": [{
                "tab_id": "host-tab-1",
                "mode": "mirror",
                "received_fps": 60.0,
                "painted_fps": 59.5,
                "region_painted_fps": 59.5,
                "dropped_fps": 0.0,
                "still_updates_fps": 0.0,
                "input_to_paint_ms": { "p50": 31.0, "max": 48.0, "count": 3 },
                "totals": { "received": 600, "painted": 595, "dropped": 0, "still_updates": 1 }
            }]
        },
        "snapshot": { "request": 1, "tab_id": "host-tab-1", "source": "region_video", "jpeg_base64": "/9j/4A==" }
    }});
    let request: LocalDaemonRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    let mut headless = wire.clone();
    headless["OwnerView"]["browser"]["viewer"]["kind"] = "headless".into();
    serde_json::from_value::<LocalDaemonRequest>(headless).unwrap();
    for source in ["viewer_canvas", "document_picture"] {
        let mut other = wire.clone();
        other["OwnerView"]["snapshot"]["source"] = source.into();
        serde_json::from_value::<LocalDaemonRequest>(other).unwrap();
    }
    // An older owner tab publishes rows only.
    let mut old = wire.clone();
    for key in ["browser", "snapshot"] {
        old["OwnerView"].as_object_mut().unwrap().remove(key);
    }
    let old_request: LocalDaemonRequest = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&old_request).unwrap(), old);
    // The publication stays strict: an unknown field is refused.
    let mut extra = wire;
    extra["OwnerView"]["browser"]["panes"][0]["click"] = serde_json::json!([1, 2]);
    assert!(serde_json::from_value::<LocalDaemonRequest>(extra).is_err());
}

#[test]
fn an_owner_view_status_counts_picture_requests_and_defaults_without_them() {
    let wire = serde_json::json!({ "OwnerViewStatus": {
        "generation": "g",
        "tab_id": null,
        "watchers": [{ "agent_id": "agent", "name": "agent-1" }],
        "snapshot_wanted": 2
    }});
    let response: LocalDaemonResponse = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), wire);
    let mut old = wire;
    old["OwnerViewStatus"]
        .as_object_mut()
        .unwrap()
        .remove("snapshot_wanted");
    let LocalDaemonResponse::OwnerViewStatus {
        snapshot_wanted, ..
    } = serde_json::from_value(old).unwrap()
    else {
        panic!("expected status")
    };
    assert_eq!(snapshot_wanted, 0);
}
