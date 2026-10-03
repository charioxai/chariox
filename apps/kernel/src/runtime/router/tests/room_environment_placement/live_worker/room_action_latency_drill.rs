use super::*;
use futures_util::FutureExt;

// Run only in an owned headed fixture. Home/worker kernels and encrypted relay
// transport are real; no provider accounts or running kernel services are used.
#[test]
#[ignore = "requires an owned headed display; see room action latency evidence"]
fn room_action_latency_headed_drill() {
    run_test(headed_drill);
}

async fn headed_drill() {
    let script =
        std::env::var("CHARIOX_ROOM_LATENCY_CONTROLLER").expect("owned production controller path");
    let output = std::env::var("CHARIOX_ROOM_LATENCY_OUTPUT").expect("external evidence path");
    let coordinates: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("CHARIOX_ROOM_LATENCY_COORDINATES").unwrap()).unwrap(),
    )
    .unwrap();
    let mut fixture = LiveWorker::start_configured(true, true).await;
    fixture
        .worker
        .runtime_state
        .set_browser_controller_process_store_for_test(
            crate::runtime::browser_controller_process::BrowserControllerProcessStore::from_script(
                script,
                Duration::from_secs(30),
            ),
        );
    let measurements = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop")).await.unwrap();
        dispatch_json(&fixture.home, json!({"StartRoomEnvironment": {
            "session_id":room,"viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800
            }
        }})).await.unwrap();
        dispatch_json(&fixture.home, json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":room,"target":{"kind":"desktop"}
        }})).await.unwrap();
        let control = std::path::PathBuf::from(std::env::var("CHARIOX_ROOM_LATENCY_CONTROL").unwrap());
        std::fs::write(control.with_extension("ready"), fixture.address.port().to_string()).unwrap();
        timeout(Duration::from_secs(240), async {
            while !control.with_extension("go").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("external network/display setup");
        let runtime = fixture.home.runtime_state.clone();
        let (stop, mut stopped) = watch::channel(false);
        let maintenance = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(100)) => runtime.pump_transport_runtime().await,
                    _ = stopped.changed() => break,
                }
            }
        });
        let mut samples = Vec::new();
        for index in 0..20 {
            let read = dispatch_json(&fixture.home, json!({"GetRoomEnvironmentState":{"session_id":room}})).await.unwrap();
            let environment = &read["RoomEnvironmentState"]["environment"];
            let key = format!("latency-{index}");
            let submitted = dispatch_json(&fixture.home, json!({"SubmitRoomEnvironmentAction":{
                "session_id":room,"runtime_generation":environment["runtime_generation"],
                "viewport_revision":environment["viewport"]["revision"],
                "idempotency_key":key,
                "action":{"kind":"pointer_click","x":coordinates["x"],"y":coordinates["y"],"button":"left","click_count":1}
            }})).await.unwrap();
            let snapshot = fixture.home.runtime_state.room_environment_snapshot(room).unwrap();
            let action = snapshot.actions.iter().find(|action| action.idempotency_key.as_deref() == Some(&key)).unwrap();
            assert_eq!(action.state, crate::session::EnvironmentActionState::Completed);
            samples.push(serde_json::to_value(action).unwrap());
            assert_eq!(submitted["RoomEnvironmentActionSubmitted"]["action_id"], action.action_id);
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
        let _ = stop.send(true);
        maintenance.await.unwrap();
        samples
    }).catch_unwind().await;
    fixture.stop().await;
    match measurements {
        Ok(samples) => std::fs::write(output, serde_json::to_vec_pretty(&json!({"scope":"owned headed production controller/X11 helpers, in-process kernels, private encrypted relay", "actions":samples})).unwrap()).unwrap(),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
