//! MP-08 / MP-10 A12-S02: exercise the local subscription writer and baseline.
use super::*;

#[tokio::test]
async fn review971c_local_subscription_keeps_mixed_task_changes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let session = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap()
        .0;
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "task-subscriber",
            crate::attachment::ClientCapabilityLevel::MessageTransport,
        ))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 4));
    let runtime = Arc::new(KernelTransportRuntime::new(router.transport_health_store()));
    let (priority_tx, _priority_rx) = mpsc::channel(64);
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (close_tx, _close_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(super::super::subscriptions::run_subscription_loop(
        router.clone(),
        runtime,
        super::super::outgoing::KernelOutgoingSender::new(priority_tx, event_tx),
        close_tx,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        KernelSubscription {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            subscription_scope: KernelSubscriptionScope::Session,
            connection_class: crate::local::KernelConnectionClass::Terminal,
            heartbeat_interval: std::time::Duration::from_secs(5),
        },
    ));
    let result = timeout(Duration::from_secs(5), async {
        async fn next_snapshot(rx: &mut mpsc::Receiver<KernelOutgoingFrame>) -> KernelEvent {
            loop {
                if let KernelOutgoingFrame::Event { event, .. } =
                    rx.recv().await.expect("subscription closed")
                {
                    if matches!(*event, KernelEvent::SessionSnapshot { .. }) {
                        return *event;
                    }
                }
            }
        }
        next_snapshot(&mut event_rx).await;
        for case in 0..4 {
            let updated = crate::test_support::mixed_task_subscription_updates(
                &mut *app.lock().await,
                session.id(),
                attachment.id(),
                case,
            );
            let KernelEvent::SessionSnapshot {
                session: delivered,
                provider_run,
                agent_activity,
                ..
            } = next_snapshot(&mut event_rx).await
            else {
                unreachable!()
            };
            assert_eq!(delivered.agent_tasks(), updated.agent_tasks());
            assert_eq!(delivered.alias(), updated.alias());
            assert_eq!(delivered.agents(), updated.agents());
            assert_eq!(
                serde_json::to_value(&delivered).unwrap()["prompt_runtime"],
                serde_json::to_value(&updated).unwrap()["prompt_runtime"]
            );
            assert_eq!(
                provider_run.as_ref().as_ref().map(|run| run.id()),
                updated.active_provider_run_id()
            );
            if case == 2 {
                assert_eq!(
                    agent_activity[updated.agents()[0].id()].queued_prompt_count,
                    1
                );
            }
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    router.runtime_state().shutdown_cleanup().await.unwrap();
    result.expect("MP-08 / MP-10 local subscriber lost a mixed task update");
}
