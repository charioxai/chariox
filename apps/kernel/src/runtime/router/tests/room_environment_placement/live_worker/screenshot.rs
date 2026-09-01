use super::*;
use base64::Engine as _;

const SCREENSHOT_BYTES: usize = 131_079;
const SCREENSHOT_SHA256: &str = "3f96168e429ba6431f32b0f76efe5c49720be00e677026e038f8d6508f52bc42";

#[test]
fn room_screenshot_relay_drill_crosses_the_bound_worker_in_bounded_chunks() {
    run_test(captures_and_reads_bound_worker_screenshot);
}

#[test]
fn room_screenshot_rejects_an_attachment_from_another_room() {
    run_test(rejects_cross_room_attachment);
}

#[test]
fn room_screenshot_rejects_oversized_chunk_requests() {
    run_test(rejects_oversized_chunk_request);
}

async fn captures_and_reads_bound_worker_screenshot() {
    let _guard = crate::env_lock::lock();
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let source = fixture._worker_state.root.join("expected-screenshot.png");
    let helper = fixture
        ._worker_state
        .root
        .join("slice-screen-screenshot.sh");
    let mut expected = vec![0_u8; SCREENSHOT_BYTES];
    for (index, byte) in expected.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    expected[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    std::fs::write(&source, &expected).expect("screenshot fixture should write");
    std::fs::write(
        &helper,
        "#!/bin/sh\n[ \"$1\" = screenshot ] || exit 2\ncp \"$CHARIOX_ROOM_SCREENSHOT_FIXTURE\" \"$2\"\n",
    )
    .expect("screenshot helper should write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700))
            .expect("screenshot helper should be executable");
    }
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &helper);
    std::env::set_var("CHARIOX_ROOM_SCREENSHOT_FIXTURE", &source);

    create_running_slice(&fixture).await;
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id": &room,
            "slice_ref": "desktop"
        }}),
    )
    .await
    .expect("Room should bind to the worker slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id": &room,
            "client_id": "screenshot-client",
            "capability_level": "FullTerminal"
        }}),
    )
    .await
    .expect("client should attach to the Room");
    let attachment_id = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("attachment ID");

    let captured = dispatch_json(
        &fixture.home,
        json!({"CaptureRoomEnvironmentScreenshot": {
            "session_id": &room,
            "attachment_id": attachment_id
        }}),
    )
    .await
    .expect("public screenshot capture should cross the bound worker");
    let artifact = &captured["RoomEnvironmentScreenshotCaptured"]["artifact"];
    assert_eq!(artifact["sha256"], SCREENSHOT_SHA256);
    assert_eq!(artifact["size_bytes"], SCREENSHOT_BYTES);
    assert_eq!(artifact["media_type"], "image/png");
    assert!(artifact.get("operational_path").is_none());
    let artifact_id = artifact["artifact_id"].as_str().expect("artifact ID");

    let first = dispatch_json(
        &fixture.home,
        json!({"ReadRoomEnvironmentScreenshotChunk": {
            "session_id": &room,
            "attachment_id": attachment_id,
            "artifact_id": artifact_id,
            "offset": 0,
            "max_bytes": 131072
        }}),
    )
    .await
    .expect("first screenshot chunk should cross the bound worker");
    let first = &first["RoomEnvironmentScreenshotChunk"]["chunk"];
    let first_bytes = base64::engine::general_purpose::STANDARD
        .decode(first["data_base64"].as_str().expect("first chunk base64"))
        .expect("first chunk should decode");
    assert_eq!(first_bytes, expected[..131_072]);
    assert_eq!(first["eof"], false);

    let second = dispatch_json(
        &fixture.home,
        json!({"ReadRoomEnvironmentScreenshotChunk": {
            "session_id": &room,
            "attachment_id": attachment_id,
            "artifact_id": artifact_id,
            "offset": 131072,
            "max_bytes": 131072
        }}),
    )
    .await
    .expect("final screenshot chunk should cross the bound worker");
    let second = &second["RoomEnvironmentScreenshotChunk"]["chunk"];
    let second_bytes = base64::engine::general_purpose::STANDARD
        .decode(second["data_base64"].as_str().expect("second chunk base64"))
        .expect("second chunk should decode");
    assert_eq!(second_bytes, expected[131_072..]);
    assert_eq!(second["eof"], true);

    fixture.stop().await;
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    std::env::remove_var("CHARIOX_ROOM_SCREENSHOT_FIXTURE");
}

async fn rejects_cross_room_attachment() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    create_running_slice(&fixture).await;
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id": &room,
            "slice_ref": "desktop"
        }}),
    )
    .await
    .expect("Room should bind to the worker slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id": &fixture.rooms[1],
            "client_id": "other-room-client",
            "capability_level": "FullTerminal"
        }}),
    )
    .await
    .expect("client should attach to the other Room");
    let attachment_id = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("attachment ID");

    let result = dispatch_json(
        &fixture.home,
        json!({"CaptureRoomEnvironmentScreenshot": {
            "session_id": room,
            "attachment_id": attachment_id
        }}),
    )
    .await;
    fixture.stop().await;
    assert!(result
        .expect_err("another Room's attachment must not capture the Environment")
        .to_string()
        .contains("attachment"));
}

async fn rejects_oversized_chunk_request() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    create_running_slice(&fixture).await;
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id": &room,
            "slice_ref": "desktop"
        }}),
    )
    .await
    .expect("Room should bind to the worker slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id": &room,
            "client_id": "screenshot-client",
            "capability_level": "FullTerminal"
        }}),
    )
    .await
    .expect("client should attach to the Room");
    let attachment_id = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("attachment ID");

    let result = dispatch_json(
        &fixture.home,
        json!({"ReadRoomEnvironmentScreenshotChunk": {
            "session_id": room,
            "attachment_id": attachment_id,
            "artifact_id": "artifact-1",
            "offset": 0,
            "max_bytes": 131073
        }}),
    )
    .await;
    fixture.stop().await;
    assert!(result
        .expect_err("chunk requests above the transfer bound must fail")
        .to_string()
        .contains("between 1 and 131072 bytes"));
}

async fn create_running_slice(fixture: &LiveWorker) {
    dispatch_json(
        &fixture.home,
        json!({"CreateSlice": {
            "name": "desktop",
            "base": "clean",
            "display_mode": "headed",
            "display_backend": "selkies",
            "worker_kernel_ref": "desktop-worker"
        }}),
    )
    .await
    .expect("slice should be created");
    let slices = fixture.home.app.lock().await.slices().clone();
    slices
        .set_relay_endpoint(
            "desktop",
            Some(crate::slice::SliceRelayEndpoint {
                url: format!("ws://{}", fixture.address),
                private: false,
            }),
            1,
        )
        .expect("fixture relay endpoint should be set");
    slices
        .set_worker_presence(
            "desktop",
            Some("environment-worker".to_string()),
            Some("slice:slice-1".to_string()),
            vec!["managed-dev-stub".to_string()],
            crate::session::unix_epoch_ms(),
        )
        .expect("fixture worker should be present");
    slices
        .set_status(
            "desktop",
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .expect("fixture slice should be running");
}
