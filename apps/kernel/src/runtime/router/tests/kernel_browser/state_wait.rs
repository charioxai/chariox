//! MP-08/MP-10 (protocol 556): a viewer's parked state read answers when the state differs.
use super::*;

async fn wait(router: &CommandRouter, after: &str, wait_ms: u32) -> Value {
    human(
        router,
        KernelBrowserCommand::StateWait {
            after: after.into(),
            wait_ms,
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

            // Nothing changed: the read parks for its wait and then answers with the same revision.
            let started = std::time::Instant::now();
            let idle = wait(&router, &revision, 400).await;
            assert!(
                started.elapsed() >= std::time::Duration::from_millis(350),
                "idle waits last their deadline"
            );
            assert_eq!(idle["revision"], revision.as_str());

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
                tokio::spawn(async move { wait(&router, &revision, 2000).await })
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
                    wait_ms: 2001,
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
