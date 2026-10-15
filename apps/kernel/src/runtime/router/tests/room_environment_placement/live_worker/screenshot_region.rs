//! Actual home/worker/relay capture path with a deterministic protected desktop
//! helper. Existing Room tests retain their original screenshot transfer drill.
use super::*;
use base64::Engine as _;

#[test]
fn room_screenshot_region_relay_preserves_masks_and_rejects_stale_viewport() {
    crate::test_support::isolated_env_test!();
    run_test(region_drill);
}
async fn region_drill() {
    let _guard = crate::env_lock::lock();
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let source = fixture._worker_state.root.join("protected.png");
    let helper = fixture._worker_state.root.join("capture-region.sh");
    let mut pixels = vec![200_u8; 1280 * 800 * 3];
    // The desktop helper returns already protected pixels. Crop must never
    // substitute a raw App/browser-page screenshot for the protected desktop.
    for y in 40..60 {
        for x in 20..40 {
            let at = (y * 1280 + x) * 3;
            pixels[at..at + 3].fill(0);
        }
    }
    {
        let mut encoder = png::Encoder::new(std::fs::File::create(&source).unwrap(), 1280, 800);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    std::fs::write(&helper,"#!/bin/sh\nset -eu\ncase \"${1:-}\" in protected-*) python3 -c 'import json,sys; p=json.load(sys.stdin); assert not p[\"unknown\"]' ;; esac\ncase \"${1:-}\" in protected-screenshot) cp \"$CHARIOX_ROOM_SCREENSHOT_FIXTURE\" \"$2\" ;; status) printf 'available=true\\nscreen=1280x800\\nmode=desktop\\n' ;; *) exit 2 ;; esac\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &helper);
    std::env::set_var("CHARIOX_ROOM_SCREENSHOT_FIXTURE", &source);
    super::screenshot::create_running_slice(&fixture).await;
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice":{"session_id":room,"slice_ref":"desktop"}}),
    )
    .await
    .unwrap();
    dispatch_json(&fixture.home,json!({"StartRoomEnvironment":{"session_id":room,"viewport":{"css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800}}})).await.unwrap();
    let state = fixture
        .home
        .runtime_state
        .room_environment_snapshot(&room)
        .unwrap();
    let attached=dispatch_json(&fixture.home,json!({"AttachToSession":{"session_id":room,"client_id":"region-client","capability_level":"FullTerminal"}})).await.unwrap();
    let mut request = json!({"CaptureVisibleRegion":{"capture_id":"room-region","surface":{"kind":"room","session_id":room,"attachment_id":attached["SessionAttached"]["attachment"]["id"],"runtime_generation":state.runtime_generation,"viewport_revision":state.viewport.revision},"region":{"x":5,"y":15,"width":20,"height":20,"viewport_width":640,"viewport_height":400,"frame_width":1280,"frame_height":800}}});
    let response = human_capture(&fixture.home, request.clone()).await.unwrap();
    let capture = &response["VisibleRegionCaptured"]["capture"];
    assert_eq!(capture["width"], 40);
    assert_eq!(capture["height"], 40);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(capture["data_base64"].as_str().unwrap())
        .unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(&pixels[(10 * 40 + 10) * 3..(10 * 40 + 10) * 3 + 3], &[0; 3]);
    assert_eq!(&pixels[..3], &[200; 3]);
    request["CaptureVisibleRegion"]["surface"]["viewport_revision"] =
        (state.viewport.revision + 1).into();
    assert!(human_capture(&fixture.home, request).await.is_err());
    fixture.stop().await;
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_ROOM_SCREENSHOT_FIXTURE");
}

async fn human_capture(router: &CommandRouter, value: Value) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
    let mut command = KernelCommand::from_local_request("human-capture", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}
