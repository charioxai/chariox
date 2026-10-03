use super::*;

#[test]
fn capability_response_cannot_overwrite_later_room_binding() {
    run_test(|| capability_response_ordering(false));
}

#[test]
fn capability_response_cannot_restore_deleted_room_binding() {
    run_test(|| capability_response_ordering(true));
}

async fn capability_response_ordering(initially_bound: bool) {
    let mut fixture = LiveWorker::start().await;
    let (_worker_state, worker) = start_agent_worker(&mut fixture).await;
    let mut capability = None;
    let mut release = None;
    let check = std::panic::AssertUnwindSafe(async {
        wait_for_agent_worker(&fixture).await;
        fixture.create_slice().await;
        let room = fixture.rooms[0].clone();
        if initially_bound {
            dispatch_json(
                &fixture.home,
                json!({"BindRoomEnvironmentSlice": {
                    "session_id": room, "slice_ref": "desktop"
                }}),
            )
            .await
            .expect("initial Room binding");
        }
        let leased = launch_leased_room_provider(
            &fixture,
            &worker,
            "keep the leased provider live while capability responses race with binding updates",
        )
        .await;
        assert_eq!(
            leased.remote_extension_manifest.room_browser_available,
            initially_bound,
        );
        let token = worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&leased.worker_provider_run_id)
            .expect("leased provider tool token");
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        release = Some(release_tx);
        worker
            .runtime_state
            .pause_capability_response_before_apply_for_test(entered_tx, release_rx);
        let calling_worker = Arc::clone(&worker);
        capability = Some(tokio::spawn(async move {
            calling_worker
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(
                    &token,
                    crate::transport::runtime_tools::LIST_SESSION_AGENTS_TOOL,
                    json!({}),
                )
                .await
        }));
        timeout(Duration::from_secs(10), entered_rx)
            .await
            .expect("capability response reaches its application barrier")
            .expect("capability response barrier remains installed");

        let (push_observed_tx, push_observed_rx) = tokio::sync::oneshot::channel();
        worker
            .runtime_state
            .observe_capability_push_lock_for_test(push_observed_tx);

        if initially_bound {
            fixture
                .home
                .runtime_state
                .delete_slice("desktop")
                .expect("delete the Room binding through shared kernel state");
        } else {
            dispatch_json(
                &fixture.home,
                json!({"BindRoomEnvironmentSlice": {
                    "session_id": room, "slice_ref": "desktop"
                }}),
            )
            .await
            .expect("bind the Room after the capability snapshot");
        }
        let manifest = {
            let app = fixture.home.app.lock().await;
            let agent = app.agents().get_agent(&leased.home_agent_id).unwrap();
            app.remote_extension_manifest_for_agent(&agent).unwrap()
        };
        assert_eq!(manifest.room_browser_available, !initially_bound);
        let acquired_before_release = timeout(Duration::from_secs(10), push_observed_rx)
            .await
            .expect("automatic manifest push reaches worker app lock")
            .expect("worker reports actual lock readiness");
        if acquired_before_release {
            wait_for_manifest_sync(&fixture, &leased.home_agent_id, &manifest.manifest_hash())
                .await;
        }
        release
            .take()
            .unwrap()
            .send(())
            .expect("release capability response application");
        let joined = timeout(Duration::from_secs(10), capability.as_mut().unwrap())
            .await
            .expect("capability call finishes");
        capability.take();
        let result = joined
            .expect("capability task joins")
            .expect("capability call succeeds");
        assert!(result.ok, "{:?}", result.payload);
        wait_for_manifest_sync(&fixture, &leased.home_agent_id, &manifest.manifest_hash()).await;
        let app = worker.app.lock().await;
        let run = app
            .providers()
            .get_run(&leased.worker_provider_run_id)
            .expect("same leased provider run survives the capability update");
        assert_eq!(
            run.remote_extension_manifest().room_browser_available,
            !initially_bound
        );
        assert!(
            acquired_before_release,
            "the outbound capability request must leave the worker available for home callbacks"
        );
    })
    .catch_unwind()
    .await;
    if let Some(release) = release.take() {
        let _ = release.send(());
    }
    let capability_cleanup = if let Some(mut task) = capability.take() {
        match timeout(Duration::from_secs(10), &mut task).await {
            Ok(result) => Some(result),
            Err(_) => {
                task.abort();
                Some(task.await)
            }
        }
    } else {
        None
    };
    worker
        .runtime_state
        .clear_capability_response_pause_for_test();
    let cleanup = worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    cleanup.expect("stop leased provider after capability response ordering test");
    if let Some(result) = capability_cleanup {
        let _ = result.expect("released capability task must finish before teardown");
    }
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

async fn wait_for_manifest_sync(fixture: &LiveWorker, agent_id: &str, hash: &str) {
    timeout(Duration::from_secs(10), async {
        loop {
            let synced = {
                let app = fixture.home.app.lock().await;
                let agent = app.agents().get_agent(agent_id).unwrap();
                agent
                    .remote_extension_manifest_sync()
                    .is_some_and(|status| {
                        status.manifest_hash.as_deref() == Some(hash)
                            && status.state
                                == crate::extension::RemoteExtensionManifestSyncState::Synced
                    })
            };
            if synced {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("automatic manifest push is acknowledged at home");
}
