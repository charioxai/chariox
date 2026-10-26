//! MP-08/MP-10/MP-11: supplementary admission tests, not live acceptance.
use super::*;
use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
use crate::slice::{CreateSliceInput, SliceBackendKind, SliceDisplayMode, SliceStatus};

pub(super) async fn fixture() -> (TestRoom, String, tokio::net::TcpListener) {
    let room = TestRoom::new("display-resume-admission");
    let store = &room.runtime.owned.slice_store;
    let slice = store
        .create(
            "home",
            "machine",
            CreateSliceInput {
                source_slice_ref: None,
                name: "display-resume".into(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: SliceDisplayMode::Headed,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("admission-test-worker".into()),
                display_url: None,
                provider_auth: vec![],
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap();
    store
        .bind_environment(&room.session_id, &slice.id, 2, |_| Ok(()))
        .unwrap();
    store
        .set_status(&slice.id, SliceStatus::Running, 3)
        .unwrap();
    // Reaching this owned socket proves admission ended before the held reads
    // were released. It does not simulate a successful worker/provider run.
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    store
        .set_relay_endpoint(
            &slice.id,
            Some(crate::slice::SliceRelayEndpoint {
                url: format!("ws://{}", sink.local_addr().unwrap()),
                private: true,
            }),
            4,
        )
        .unwrap();
    (room, slice.id, sink)
}

#[tokio::test]
async fn mp10_resume_reaches_peer_while_three_other_viewers_hold_reads() {
    for command in [
        RoomDisplayCommand::Resume {},
        RoomDisplayCommand::ResumeAttach {},
    ] {
        let (room, slice, sink) = fixture().await;
        let runtime = &room.runtime;
        let attachment = crate::app::KernelSessionService::new(&mut *runtime.app.lock().await)
            .attach(crate::attachment::AttachRequest::new(
                &room.session_id,
                "resume-viewer",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap()
            .id()
            .to_owned();
        let viewer = runtime.owned.config_projection.snapshot().relay_public_key;
        let caller = KernelCaller::default();
        // A fresh App pump has a startup wake. Settle that unrelated work so
        // this regression actually exercises the idle transport interval.
        runtime
            .app_control()
            .event_pump()
            .try_begin()
            .unwrap()
            .finish(None, None, None, false);
        assert_eq!(
            runtime.transport_runtime_pump_interval_ms(500, 5000, crate::session::unix_epoch_ms()),
            5000,
            "MP-08/MP-09: fixture must be idle before a display opens"
        );
        let binding = RoomDisplayBinding {
            generation: 1,
            subscription_id: "resume-admission".into(),
            codec: "h264".into(),
            bitrate: 8_000_000,
            device_scale_factor: 1,
            source: crate::slice::RoomDisplaySource {
                kind: "desktop".into(),
                surface_id: "surface".into(),
                generation: "source-1".into(),
                width: 1280,
                height: 800,
            },
            resumed: false,
        };
        runtime
            .owned
            .room_display_leases
            .home
            .lock()
            .unwrap()
            .insert(
                binding.subscription_id.clone(),
                HomeLease::new(RoomDisplayLease {
                    caller: caller.clone(),
                    session_id: room.session_id.clone(),
                    attachment_id: attachment,
                    slice_id: slice.clone(),
                    viewer_public_key: viewer.clone(),
                    binding,
                    worker_public_key: viewer,
                    used: Instant::now(),
                }),
            );
        let lease = runtime.home_display_lease("resume-admission").unwrap();
        let held_frame = lease.lock().await;
        assert_eq!(
            runtime.room_display_scopes(),
            BTreeMap::from([(room.session_id.clone(), slice.clone())]),
            "audience refresh uses live immutable scopes without waiting for frames or reviving stale rooms"
        );
        assert!(runtime.transport_runtime_pump_interval_ms(500, 5000, crate::session::unix_epoch_ms()) <= 500,
            "MP-08/MP-09: live displays renew the three-second owner grant before the idle pump sleeps");
        drop(held_frame);
        let mut readers = Vec::new();
        for _ in 0..3 {
            readers.push(
                runtime
                    .owned
                    .slice_store
                    .queue_environment_use(
                        &slice,
                        Some(&room.session_id),
                        crate::slice::SHARED_DISPLAY_READ_OPERATION,
                    )
                    .await
                    .unwrap(),
            );
        }
        let resume = runtime.room_display_control(&caller, "resume-admission", 1, command);
        tokio::pin!(resume);
        tokio::select! {
            result = sink.accept() => {drop(result.unwrap());},
            result = &mut resume => panic!("MP-11: fixture must reach peer I/O after full display admission, got {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(200)) => panic!("MP-10: resume waited behind another viewer's held frame RPC"),
        }
        assert!(
            runtime
                .owned
                .slice_store
                .try_begin_operation(&slice, "slice.stop")
                .is_err(),
            "MP-11: peer I/O retains lifecycle exclusion"
        );
        drop(readers);
    }
}

#[tokio::test]
async fn mp10_resource_inventory_reaches_peer_without_waiting_for_active_input() {
    let (room, slice, sink) = fixture().await;
    let route = room
        .runtime
        .owned
        .slice_store
        .guard_environment_use(&slice, Some(&room.session_id), "browser_controller.route")
        .unwrap();
    let inventory = room
        .runtime
        .room_environment_resource_inventory(&room.session_id, &slice);
    tokio::pin!(inventory);
    tokio::select! {
        result = sink.accept() => {drop(result.unwrap());},
        result = &mut inventory => panic!("MP-11: fixture must reach inventory peer I/O, got {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(200)) => panic!("MP-10: reconnect inventory queued an exclusive Reconcile behind active input"),
    }
    drop(route);
}
