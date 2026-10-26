//! MP-08/MP-10/MP-11: delivery cancellation owns the actual in-flight frame pump.
use super::subscriptions::RelaySubscriptionTasks;
use std::future::Future;
mod task;
pub(super) use task::DisplayTask;

pub(super) async fn start(
    tasks: &RelaySubscriptionTasks,
    key: &str,
    relay_id: &str,
    public_key: &str,
    delivery: impl Future<Output = ()> + Send + 'static,
) {
    let mut tasks = tasks.lock().await;
    let Some(task) = tasks.get_mut(key).filter(|task| {
        task.relay_subscription_id == relay_id && task.client_public_key == public_key
    }) else {
        // Unsubscribe or a newer admission already retired this delivery.
        return;
    };
    if let Some(pump) = &task.display_pump {
        pump.start_delivery(delivery);
    }
}

#[cfg(test)]
mod tests {
    use super::super::subscriptions::{
        remove_relay_subscription_task_by_relay_id, RelaySubscriptionTask,
    };
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};
    use tokio::{
        sync::{oneshot, Mutex},
        time::{timeout, Duration},
    };

    struct Capturing(Option<oneshot::Sender<()>>);
    impl Drop for Capturing {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    fn registry() -> RelaySubscriptionTasks {
        Arc::new(Mutex::new(BTreeMap::new()))
    }
    async fn register(tasks: &RelaySubscriptionTasks, relay: &str) {
        tasks.lock().await.insert(
            "display:lease:viewer".into(),
            RelaySubscriptionTask {
                relay_subscription_id: relay.into(),
                client_public_key: "viewer".into(),
                display_id: Some("lease".into()),
                display_pump: Some(super::super::display_pump::DisplayPump::new(1, "test")),
                handle: tokio::spawn(std::future::pending()),
            },
        );
    }

    #[tokio::test]
    async fn unsubscribe_drops_inflight_display_capture() {
        let tasks = registry();
        register(&tasks, "relay").await;
        let (began, started) = oneshot::channel();
        let (released, dropped) = oneshot::channel();
        let capture = Capturing(Some(released));
        start(
            &tasks,
            "display:lease:viewer",
            "relay",
            "viewer",
            async move {
                let _capture = capture;
                let _ = began.send(());
                std::future::pending::<()>().await;
            },
        )
        .await;
        started.await.unwrap();
        let task = remove_relay_subscription_task_by_relay_id(&tasks, "relay")
            .await
            .unwrap();
        task.handle.abort();
        task.display_pump.as_ref().unwrap().stop();
        timeout(Duration::from_millis(100), dropped)
            .await
            .expect("unsubscribe must cancel the frame read holding the home lease")
            .unwrap();
    }

    #[tokio::test]
    async fn retired_admission_cannot_replace_new_delivery() {
        let tasks = registry();
        register(&tasks, "new-relay").await;
        let (released, dropped) = oneshot::channel();
        let capture = Capturing(Some(released));
        start(
            &tasks,
            "display:lease:viewer",
            "old-relay",
            "viewer",
            async move {
                let _capture = capture;
                std::future::pending::<()>().await;
            },
        )
        .await;
        timeout(Duration::from_millis(100), dropped)
            .await
            .expect("a retired registration must discard its delivery")
            .unwrap();
        let task = remove_relay_subscription_task_by_relay_id(&tasks, "new-relay")
            .await
            .unwrap();
        task.handle.abort();
    }

    #[tokio::test]
    async fn acknowledgements_renew_ttl_without_aborting_capture() {
        let tasks = registry();
        register(&tasks, "relay").await;
        let (began, started) = oneshot::channel();
        let (released, mut dropped) = oneshot::channel();
        let capture = Capturing(Some(released));
        start(
            &tasks,
            "display:lease:viewer",
            "relay",
            "viewer",
            async move {
                let _capture = capture;
                let _ = began.send(());
                std::future::pending::<()>().await;
            },
        )
        .await;
        started.await.unwrap();
        for _ in 0..3 {
            assert_eq!(
                super::super::browser_display::browser_display_delivery_id(
                    &tasks, "lease", "viewer"
                )
                .await
                .as_deref(),
                Some("relay")
            );
            tokio::task::yield_now().await;
            assert!(matches!(
                dropped.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
        }
        let task = remove_relay_subscription_task_by_relay_id(&tasks, "relay")
            .await
            .unwrap();
        task.handle.abort();
        task.display_pump.as_ref().unwrap().stop();
        timeout(Duration::from_secs(1), dropped)
            .await
            .unwrap()
            .unwrap();
    }
}
