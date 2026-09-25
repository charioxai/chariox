use super::*;
use futures_util::FutureExt;

#[test]
fn worker_browser_and_user_display_share_the_bound_room_environment() {
    run_test(worker_browser_and_user_display_share_the_bound_room_environment);
}

async fn worker_browser_and_user_display_share_the_bound_room_environment() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let check = std::panic::AssertUnwindSafe(async {
        let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
        let started = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id": &room,
                "viewport": {
                    "css_width": 1280,
                    "css_height": 800,
                    "device_scale_factor": 1,
                    "desktop_pixel_width": 1280,
                    "desktop_pixel_height": 800
                }
            }}),
        )
        .await
        .expect("start the Room Environment before sharing it with agents and Web View");
        let started_environment = &started["RoomEnvironmentUpdated"]["environment"];
        assert_eq!(started_environment["session_id"], room);
        assert_eq!(started_environment["lifecycle"], "ready");
        let environment_id = started_environment["environment_id"]
            .as_str()
            .expect("Room Environment identity")
            .to_string();
        let initial_tab_id = started_environment["focused_tab_id"]
            .as_str()
            .expect("initial focused Room Tab")
            .to_string();
        let initial_tab_url = started_environment["tabs"]
            .as_array()
            .and_then(|tabs| tabs.iter().find(|tab| tab["tab_id"] == initial_tab_id))
            .and_then(|tab| tab["url"].as_str())
            .expect("initial Room Tab URL")
            .to_string();

        // This is the existing public worker-agent Browser tool flow: it
        // launches a slice-placed agent and forwards authenticated Browser
        // runtime calls to the home Room's controller.
        let placement = fixture.placement();
        super::controller_worker_mcp::check(&fixture, placement).await;

        let environment = fixture
            .home
            .runtime_state
            .room_environment_snapshot(&room)
            .expect("read the shared home Room Environment after worker Browser calls");
        assert_eq!(environment.session_id, room);
        assert_eq!(environment.environment_id, environment_id);
        assert_eq!(
            environment.focused_tab_id.as_deref(),
            Some(initial_tab_id.as_str())
        );
        let tab = environment
            .tabs
            .iter()
            .find(|tab| tab.tab_id == initial_tab_id)
            .expect("worker Browser call must retain the Room's stable Tab");
        assert_eq!(tab.url, initial_tab_url);

        let binding = dispatch_json(
            &fixture.home,
            json!({"GetRoomEnvironmentSlice":{"session_id":&room}}),
        )
        .await
        .expect("read the Room's authoritative Environment slice binding");
        assert_eq!(
            binding["RoomEnvironmentSlice"]["binding"]["session_id"],
            room
        );
        assert_eq!(
            binding["RoomEnvironmentSlice"]["binding"]["slice_id"],
            "slice-1"
        );

        let opened = dispatch_json(
            &fixture.home,
            json!({"GetSliceDisplayEndpoint": {
                "slice_ref": "desktop",
                "session_id": &room,
                "attachment_id": attachment_id,
                "viewer_public_key": viewer_public.clone()
            }}),
        )
        .await
        .expect("open the user-facing Room display through its bound worker");
        let endpoint = &opened["SliceDisplayEndpoint"]["endpoint"];
        assert_eq!(
            endpoint["slice_id"],
            binding["RoomEnvironmentSlice"]["binding"]["slice_id"],
            "Web View must resolve the slice bound to the same Room Environment used by worker Browser"
        );
        assert_eq!(endpoint["kind"], "selkies");
        assert_eq!(endpoint["access"], "tunnel");
        assert_eq!(endpoint["stream_protocol"], "chariox-display-v1");
        assert_eq!(
            endpoint["peer_public_key"],
            fixture._worker_state.config.relay_public_key
        );
        let stream_id = endpoint["stream_id"].as_str().expect("stream ID");
        assert_eq!(
            endpoint["url"],
            format!("ws://{}/display/{stream_id}/stream", fixture.address)
        );
        assert!(endpoint["capabilities"]
            .as_array()
            .is_some_and(|capabilities| capabilities.contains(&json!("encrypted"))
                && capabilities.contains(&json!("single_use"))));
        let reopened = dispatch_json(
            &fixture.home,
            json!({"GetSliceDisplayEndpoint": {
                "slice_ref": "desktop",
                "session_id": &room,
                "attachment_id": attachment_id,
                "viewer_public_key": viewer_public
            }}),
        )
        .await
        .expect("reconnect should receive a fresh one-use display grant");
        assert_ne!(
            reopened["SliceDisplayEndpoint"]["endpoint"]["stream_id"],
            endpoint["stream_id"]
        );
    })
    .catch_unwind()
    .await;
    let controller_cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    let provider_cleanup = fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    controller_cleanup.expect("stop the Room controller after Browser and Web View calls");
    provider_cleanup.expect("stop worker provider processes after Browser and Web View calls");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn hosted_service_cannot_claim_a_room_display_grant() {
    run_test(rejects_hosted_service);
}

async fn rejects_hosted_service() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let result = dispatch_with_caller(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": attachment_id,
            "viewer_public_key": viewer_public.clone()
        }}),
        KernelCaller {
            caller_id: "cloud-control-plane".to_string(),
            caller_kind: KernelCallerKind::HostedService,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: None,
            machine_id: None,
            realm_id: Some("default".to_string()),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &viewer_public,
            )),
            metaagent_id: None,
        },
    )
    .await;
    fixture.stop().await;
    let error = result.expect_err("Cloud control plane must not receive runtime display grants");
    assert!(error
        .to_string()
        .contains("hosted service identity is not authorized for this request"));
}

#[test]
fn remote_kernel_cannot_claim_a_room_display_grant() {
    run_test(rejects_remote_kernel);
}

async fn rejects_remote_kernel() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let result = dispatch_with_caller(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": attachment_id,
            "viewer_public_key": viewer_public
        }}),
        KernelCaller {
            caller_id: "another-kernel".to_string(),
            caller_kind: KernelCallerKind::RemoteKernel,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: None,
            machine_id: Some("another-machine".to_string()),
            realm_id: Some("default".to_string()),
            public_key_thumbprint: None,
            metaagent_id: None,
        },
    )
    .await;
    fixture.stop().await;
    let error = result.expect_err("peer kernels must use the bound worker relay request");
    assert!(error
        .to_string()
        .contains("caller cannot open a Room display"));
}

#[test]
fn remote_client_display_grant_requires_its_authenticated_viewer_key() {
    run_test(rejects_remote_client_key_mismatch);
}

async fn rejects_remote_client_key_mismatch() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let other_private = crate::transport::relay_crypto::generate_private_key_base64();
    let other_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&other_private)
            .expect("other viewer public key");
    let result = dispatch_with_caller(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": attachment_id,
            "viewer_public_key": viewer_public
        }}),
        KernelCaller {
            caller_id: "remote-viewer".to_string(),
            caller_kind: KernelCallerKind::RemoteClient,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: Some("remote-viewer".to_string()),
            machine_id: None,
            realm_id: Some("default".to_string()),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &other_public,
            )),
            metaagent_id: None,
        },
    )
    .await;
    fixture.stop().await;
    let error = result.expect_err("the relay identity and viewer cipher key must match");
    assert!(error
        .to_string()
        .contains("viewer key does not match the authenticated relay client"));
}

#[test]
fn remote_client_with_its_authenticated_viewer_key_can_open_the_room_display() {
    run_test(admits_remote_client_with_matching_key);
}

async fn admits_remote_client_with_matching_key() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let opened = dispatch_with_caller(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": attachment_id,
            "viewer_public_key": viewer_public.clone()
        }}),
        KernelCaller {
            caller_id: "remote-viewer".to_string(),
            caller_kind: KernelCallerKind::RemoteClient,
            user_id: Some(DEFAULT_LOCAL_USER_ID.to_string()),
            client_id: Some("remote-viewer".to_string()),
            machine_id: None,
            realm_id: Some("default".to_string()),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &viewer_public,
            )),
            metaagent_id: None,
        },
    )
    .await
    .expect("the authenticated remote viewer should receive its own key-bound grant");
    fixture.stop().await;
    assert_eq!(
        opened["SliceDisplayEndpoint"]["endpoint"]["stream_protocol"],
        "chariox-display-v1"
    );
}

#[test]
fn room_display_grant_rejects_a_different_attachment_owner() {
    run_test(rejects_different_attachment_owner);
}

async fn rejects_different_attachment_owner() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let result = dispatch_with_caller(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": attachment_id,
            "viewer_public_key": viewer_public
        }}),
        KernelCaller {
            caller_id: "different-local-user".to_string(),
            caller_kind: KernelCallerKind::LocalClient,
            user_id: Some("user-2".to_string()),
            client_id: Some("different-local-user".to_string()),
            machine_id: None,
            realm_id: None,
            public_key_thumbprint: None,
            metaagent_id: None,
        },
    )
    .await;
    fixture.stop().await;
    assert!(matches!(
        result,
        Err(DaemonError::SessionAccessDenied { user_id, .. }) if user_id == "user-2"
    ));
}

#[test]
fn worker_rejects_room_display_open_from_the_wrong_home_binding() {
    run_test(rejects_wrong_worker_binding);
}

async fn rejects_wrong_worker_binding() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
    let viewer_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
            .expect("viewer public key");
    let result = fixture
        .worker
        .runtime_state
        .execute_bound_room_display_open(
            "wrong-home-kernel",
            &fixture.home_state.config.relay_public_key,
            &fixture.rooms[0],
            "slice-1",
            viewer_public,
        )
        .await;
    fixture.stop().await;
    assert!(result
        .expect_err("the worker must independently reject a mismatched home binding")
        .to_string()
        .contains("peer or binding scope was denied"));
}

#[test]
fn selkies_display_does_not_accept_the_legacy_unscoped_endpoint_request() {
    run_test(rejects_unscoped_selkies_request);
}

async fn rejects_unscoped_selkies_request() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    create_running_selkies_slice(&fixture).await;
    let result = dispatch_json(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {"slice_ref": "desktop"}}),
    )
    .await;
    fixture.stop().await;
    assert!(result
        .expect_err("Selkies must not inherit the unscoped noVNC request")
        .to_string()
        .contains("requires session_id"));
}

#[test]
fn room_display_grant_rejects_an_attachment_from_another_room() {
    run_test(rejects_cross_room_attachment);
}

async fn rejects_cross_room_attachment() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let (room, _attachment_id, viewer_public) = prepare_room_display(&fixture).await;
    let other_attachment = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id": fixture.rooms[1],
            "client_id": "other-room-viewer",
            "capability_level": "FullTerminal"
        }}),
    )
    .await
    .expect("attach a viewer to the other Room");
    let other_attachment_id = other_attachment["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("other attachment ID");
    let result = dispatch_json(
        &fixture.home,
        json!({"GetSliceDisplayEndpoint": {
            "slice_ref": "desktop",
            "session_id": room,
            "attachment_id": other_attachment_id,
            "viewer_public_key": viewer_public
        }}),
    )
    .await;
    fixture.stop().await;
    let error = result.expect_err("an attachment from another Room must not open the display");
    assert!(error.to_string().contains("attachment"));
}

async fn prepare_room_display(fixture: &LiveWorker) -> (String, String, String) {
    create_running_selkies_slice(fixture).await;
    let room = fixture.rooms[0].clone();
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id": &room, "slice_ref": "desktop"
        }}),
    )
    .await
    .expect("bind the Room to its Selkies slice");
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession": {
            "session_id": &room,
            "client_id": "local-viewer",
            "capability_level": "FullTerminal"
        }}),
    )
    .await
    .expect("attach the local viewer to the Room");
    let attachment_id = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .expect("attachment ID")
        .to_string();
    let viewer_private = crate::transport::relay_crypto::generate_private_key_base64();
    let viewer_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&viewer_private)
            .expect("viewer public key");
    (room, attachment_id, viewer_public)
}

async fn dispatch_with_caller(
    router: &CommandRouter,
    request: Value,
    caller: KernelCaller,
) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(request).expect("public request");
    let command = KernelCommand::from_local_request_with_caller(
        "room-display",
        KernelCommandSource::RelayClient,
        caller,
        None,
        None,
        &request,
    );
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).expect("public response"))
}

async fn create_running_selkies_slice(fixture: &LiveWorker) {
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
    .expect("create the Selkies slice record");
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
        .expect("set fixture relay endpoint");
    slices
        .set_worker_presence(
            "desktop",
            Some("environment-worker".to_string()),
            Some("slice:slice-1".to_string()),
            vec!["managed-dev-stub".to_string()],
            crate::session::unix_epoch_ms(),
        )
        .expect("mark fixture worker present");
    slices
        .set_status(
            "desktop",
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .expect("mark fixture slice running");
}
