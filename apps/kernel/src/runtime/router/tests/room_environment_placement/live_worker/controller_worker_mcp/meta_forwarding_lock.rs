use super::*;

#[test]
fn forwarded_meta_request_releases_worker_app_lock_during_home_round_trip() {
    run_test(forwarded_meta_request_lock_scenario);
}

async fn forwarded_meta_request_lock_scenario() {
    let mut fixture = LiveWorker::start().await;
    let (_worker_state, worker) = start_agent_worker(&mut fixture).await;
    wait_for_agent_worker(&fixture).await;
    let leased = launch_leased_room_provider(
        &fixture,
        &worker,
        "keep the leased provider live during the forwarded meta request",
    )
    .await;
    let provider_run = worker
        .app
        .lock()
        .await
        .providers()
        .get_run(&leased.worker_provider_run_id)
        .expect("leased provider run")
        .clone();

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    fixture
        .home
        .runtime_state
        .pause_forwarded_meta_request_for_test(entered_tx, release_rx);
    let calling_worker = Arc::clone(&worker);
    let mut forwarded = tokio::spawn(async move {
        calling_worker
            .runtime_state
            .try_dispatch_remote_meta_runtime_tool_call(
                &provider_run,
                crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
                json!({}),
            )
            .await
    });
    timeout(Duration::from_secs(10), entered_rx)
        .await
        .expect("home receives the forwarded meta request")
        .expect("home request barrier remains installed");
    // Other worker activity can briefly acquire the app lock. The forwarding
    // path must leave a window to acquire it while home is deliberately paused.
    let worker_app_unlocked = timeout(Duration::from_secs(1), async {
        loop {
            if worker.app.try_lock().is_ok() {
                break true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or(false);
    release_tx
        .send(())
        .expect("release the home request barrier");
    let _ = timeout(Duration::from_secs(10), &mut forwarded)
        .await
        .expect("forwarded request returns after home responds")
        .expect("forwarded request task joins");
    worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("stop the leased provider");
    fixture.stop().await;
    assert!(
        worker_app_unlocked,
        "the worker app lock must be available while home handles a forwarded meta tool"
    );
}
