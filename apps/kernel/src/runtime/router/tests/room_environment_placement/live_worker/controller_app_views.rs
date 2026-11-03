//! MP-08/MP-10/MP-11: the home outwaits a valid remote App-agent action.
use super::*;
use crate::runtime::browser_controller_app_view::{
    BrowserAppViewAsset, BrowserAppViewRequest, ViewAgentCallKind,
};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand as Command, RoomBrowserControllerResult as Response,
};
use futures_util::FutureExt;

#[test]
fn a_remote_app_view_action_taking_sixteen_seconds_keeps_its_result() {
    crate::test_support::isolated_env_test!();
    run_test(remote_app_view_action);
}

async fn remote_app_view_action() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    std::fs::write(fixture._worker_state.root.join("app-view-mode"), "enabled").unwrap();
    let result = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        fixture.home.app.lock().await.slices().set_status("desktop", SliceStatus::Running, 1).unwrap();
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop")).await.unwrap();
        dispatch_json(&fixture.home, json!({"StartRoomEnvironment": {"session_id":room,"viewport":{
            "css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800
        }}})).await.unwrap();
        let opened = fixture.home.runtime_state.room_browser_controller_command(room, Command::AppView { request: BrowserAppViewRequest::Open {
            projection: crate::runtime::browser_controller_app_view::AppViewProjection::Human,
            origin_label: "remote-action".into(), installation_id: "installed".into(), instance_id: None, entry: "index.html".into(),
            assets: vec![BrowserAppViewAsset { path: "index.html".into(), content_type: "text/html".into(), body_base64: "PG1haW4+PC9tYWluPg==".into() }], page: None,
        }}).await.unwrap();
        let Response::AppView { result: Some(opened) } = opened else { panic!("App must open: {opened:?}") };
        let started = std::time::Instant::now();
        let answer = fixture.home.runtime_state.room_browser_controller_command(room, Command::AppView { request: BrowserAppViewRequest::Agent {
            target_id: opened["target_id"].as_str().unwrap().into(), agent: "agent-7".into(), call: ViewAgentCallKind::Invoke,
            name: Some("slow".into()), input: Some(json!({})), since: None, wait_ms: None, deadline_ms: 20_000,
        }}).await;
        assert!(started.elapsed() >= Duration::from_secs(15));
        assert!(matches!(answer, Ok(Response::AppView { result: Some(ref value) }) if value == &json!({"ok":true,"value":{"completed":true}})), "remote result must survive the home transport: {answer:?}");
    }).catch_unwind().await;
    fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await
        .unwrap();
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
