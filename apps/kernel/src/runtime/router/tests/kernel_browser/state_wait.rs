//! MP-08/MP-10 (protocol 556): a viewer's parked state read answers when the state differs.
use super::*;

async fn wait(router: &CommandRouter, after: &str, wait_ms: u32) -> Value {
    human(
        router,
        KernelBrowserCommand::StateWait {
            after: after.into(),
            wait_ms,
            known_favicons: vec![],
        },
    )
    .await
}

#[test]
fn mp08_state_wait_parks_until_the_state_differs_and_never_polls_the_host() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("mp08-state-wait");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let root = std::env::temp_dir().join(format!(
                "chariox-state-wait-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            focus(&router, session.id(), agent.id()).await;

            // No revision yet: answered at once, with the revision to send next.
            let first = wait(&router, "", 0).await;
            let revision = first["revision"].as_str().unwrap().to_string();
            assert!(
                first.get("_state_signal").is_none(),
                "the host's signal stays private"
            );
            assert_eq!(first["tabs"][0]["title"], "first");

            // Nothing changed: the read parks for its wait and then answers with the same revision
            // only (transport3: the whole state with its Tab icons was 3.7 KB every two seconds).
            let started = std::time::Instant::now();
            let idle = wait(&router, &revision, 400).await;
            assert!(
                started.elapsed() >= std::time::Duration::from_millis(350),
                "idle waits last their deadline"
            );
            assert_eq!(
                idle,
                serde_json::json!({"revision": revision, "unchanged": true})
            );

            // A signal without a content change (a wake that read the same state) keeps it parked.
            std::fs::write(root.join("signal"), "1").unwrap();
            let started = std::time::Instant::now();
            let spurious = wait(&router, &revision, 400).await;
            assert!(started.elapsed() >= std::time::Duration::from_millis(350));
            let revision = spurious["revision"].as_str().unwrap().to_string();

            // A signal and a changed Tab: answered promptly with a new revision.
            let parked = {
                let router = router.clone();
                let revision = revision.clone();
                tokio::spawn(async move { wait(&router, &revision, 10_000).await })
            };
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            assert!(!parked.is_finished(), "parked while nothing changes");
            std::fs::write(root.join("title"), "second").unwrap();
            std::fs::write(root.join("signal"), "2").unwrap();
            let changed = tokio::time::timeout(std::time::Duration::from_millis(1500), parked)
                .await
                .expect("a change answers the parked read")
                .unwrap();
            assert_eq!(changed["tabs"][0]["title"], "second");
            assert_ne!(changed["revision"], revision.as_str());

            // Waits are bounded.
            let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                command: KernelBrowserCommand::StateWait {
                    after: String::new(),
                    wait_ms: 10_001,
                    known_favicons: vec![],
                },
            });
            assert!(router
                .dispatch(terminal_command("MD-4-browser", &request), request)
                .await
                .is_err());
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}

// A person's other window learns of a takeover when it happens, not when its parked read runs out: its typing
// meanwhile would be refused as input to a watched Tab.
#[test]
fn a_takeover_by_another_window_answers_the_parked_state_read_at_once() {
    use crate::session::{EnvironmentActor, EnvironmentActorKind};
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("takeover-wakes-state-wait");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let root = std::env::temp_dir().join(format!(
                "chariox-state-wait-takeover-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            focus(&router, session.id(), agent.id()).await;

            let first = wait(&router, "", 0).await;
            assert_eq!(first["tabs"][0]["taken_over"], false);
            let revision = first["revision"].as_str().unwrap().to_string();
            let parked = {
                let router = router.clone();
                tokio::spawn(async move { wait(&router, &revision, 10_000).await })
            };
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            assert!(!parked.is_finished(), "parked while nothing changes");
            router
                .runtime_state()
                .kernel_browser_host_for_test()
                .request_takeover(
                    agent.owner_user_id(),
                    EnvironmentActor::new(
                        "terminal:other-window",
                        EnvironmentActorKind::Human,
                        "Human",
                    ),
                    "host-tab-fixture",
                    1,
                )
                .unwrap();
            let changed = tokio::time::timeout(std::time::Duration::from_millis(1500), parked)
                .await
                .expect("the takeover answers the parked read")
                .unwrap();
            assert_eq!(changed["tabs"][0]["taken_over"], true);
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}

/// How a person's control of the shown Tab ends between a window's read and its next wait.
#[derive(Clone, Copy)]
enum EndsHold {
    Return,
    IdleReclaim,
}

// A takeover, its return or its idle reclaim that happens after a window got its revision but before that
// window's next wait is already news at the wait: the wait ends at once instead of parking its ten seconds.
async fn held_change_between_a_windows_read_and_its_next_wait(
    name: &'static str,
    holds_first: bool,
    ends: Option<EndsHold>,
) {
    use crate::session::{EnvironmentActor, EnvironmentActorKind};
    let workspace = crate::test_support::TestWorktree::new(name);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let root = std::env::temp_dir().join(format!(
        "chariox-state-wait-{name}-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    router
        .runtime_state()
        .install_kernel_browser_fixture(agent.owner_user_id(), &root);
    focus(&router, session.id(), agent.id()).await;
    let state = router.runtime_state();
    let host = state.kernel_browser_host_for_test();
    let other_window = || {
        EnvironmentActor::new(
            "terminal:other-window",
            EnvironmentActorKind::Human,
            "Human",
        )
    };
    let take_over = || {
        host.request_takeover(agent.owner_user_id(), other_window(), "host-tab-fixture", 1)
            .unwrap();
    };
    if holds_first {
        take_over();
    }
    // This window's last read: its revision names who holds the Tab at that moment.
    let read = wait(&router, "", 0).await;
    assert_eq!(read["tabs"][0]["taken_over"], holds_first);
    let revision = read["revision"].as_str().unwrap().to_string();
    match ends {
        None => take_over(),
        Some(EndsHold::Return) => host
            .release_input(
                agent.owner_user_id(),
                "terminal:other-window",
                "host-tab-fixture",
                1,
            )
            .unwrap(),
        Some(EndsHold::IdleReclaim) => assert!(host.reclaim_idle_takeovers_after_for_test(
            crate::runtime::kernel_browser_actors::TAKEOVER_IDLE_TIMEOUT
        )),
    }
    let started = std::time::Instant::now();
    let changed = wait(&router, &revision, 10_000).await;
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1500),
        "the wait answers at once, not after its ten seconds"
    );
    assert_eq!(changed["tabs"][0]["taken_over"], ends.is_none());
    assert_ne!(changed["revision"], revision.as_str());
    router.runtime_state().shutdown_cleanup().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_takeover_between_a_windows_read_and_its_next_wait_ends_that_wait_at_once() {
    run_test(|| {
        Box::pin(held_change_between_a_windows_read_and_its_next_wait(
            "takeover-before-wait",
            false,
            None,
        ))
    });
}

#[test]
fn a_return_of_control_between_a_windows_read_and_its_next_wait_ends_that_wait_at_once() {
    run_test(|| {
        Box::pin(held_change_between_a_windows_read_and_its_next_wait(
            "return-before-wait",
            true,
            Some(EndsHold::Return),
        ))
    });
}

#[test]
fn an_idle_reclaim_between_a_windows_read_and_its_next_wait_ends_that_wait_at_once() {
    run_test(|| {
        Box::pin(held_change_between_a_windows_read_and_its_next_wait(
            "reclaim-before-wait",
            true,
            Some(EndsHold::IdleReclaim),
        ))
    });
}

// The revision's control digest and the `taken_over` marks of the state it names come from one snapshot: a
// takeover that lands while a read is in flight is both in the state and in the digest, so its return afterwards
// differs from the revision and the next wait answers at once.
#[test]
fn a_takeover_landing_during_a_read_and_returned_after_it_is_news_to_the_next_wait() {
    use crate::session::{EnvironmentActor, EnvironmentActorKind};
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("takeover-during-read");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let root = std::env::temp_dir().join(format!(
                "chariox-state-wait-during-read-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            focus(&router, session.id(), agent.id()).await;
            let state = router.runtime_state();
            let host = state.kernel_browser_host_for_test();

            // A read is in flight in the browser host while another window takes the Tab over.
            std::fs::write(root.join("hold"), "1").unwrap();
            let reading = {
                let router = router.clone();
                tokio::spawn(async move { wait(&router, "", 0).await })
            };
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            assert!(!reading.is_finished(), "the read is held in the host");
            let owner = agent.owner_user_id().to_string();
            let takeover = {
                let host = host.clone();
                std::thread::spawn(move || {
                    host.request_takeover(
                        &owner,
                        EnvironmentActor::new(
                            "terminal:other-window",
                            EnvironmentActorKind::Human,
                            "Human",
                        ),
                        "host-tab-fixture",
                        1,
                    )
                    .unwrap();
                })
            };
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            std::fs::remove_file(root.join("hold")).unwrap();
            takeover.join().unwrap();
            let read = reading.await.unwrap();
            assert_eq!(read["tabs"][0]["taken_over"], true);
            let revision = read["revision"].as_str().unwrap().to_string();

            // The person returns control before this window waits again.
            host.release_input(
                agent.owner_user_id(),
                "terminal:other-window",
                "host-tab-fixture",
                1,
            )
            .unwrap();
            let started = std::time::Instant::now();
            let next = wait(&router, &revision, 10_000).await;
            assert!(
                started.elapsed() < std::time::Duration::from_millis(1500),
                "the wait answers at once, not after its ten seconds"
            );
            assert_eq!(next["tabs"][0]["taken_over"], false);
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}
