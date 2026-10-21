//! MP-08/MP-10/MP-11: peer-backed display admission must leave the relay reader free.
use super::subscriptions::{RelaySubscriptionTask, RelaySubscriptionTasks};
use std::future::Future;

// Registered `display:` delivery sorts first while its admission sends the ACK.
const PREFIX: &str = "pending-display-admission:";
const MAX_PENDING: usize = 32;

pub(super) async fn enqueue(
    tasks: &RelaySubscriptionTasks,
    relay_id: String,
    public_key: String,
    admission: impl Future<Output = ()> + Send + 'static,
) -> bool {
    let key = format!("{PREFIX}{relay_id}:{public_key}");
    let mut guard = tasks.lock().await;
    guard.retain(|key, task| !key.starts_with(PREFIX) || !task.handle.is_finished());
    if guard.contains_key(&key)
        || guard.keys().filter(|key| key.starts_with(PREFIX)).count() >= MAX_PENDING
    {
        return false;
    }
    let (start, ready) = tokio::sync::oneshot::channel();
    let owned_tasks = tasks.clone();
    let owned_key = key.clone();
    let handle = tokio::spawn(async move {
        if ready.await.is_err() {
            return;
        }
        admission.await;
        let id = tokio::task::id();
        let mut guard = owned_tasks.lock().await;
        if guard
            .get(&owned_key)
            .is_some_and(|task| task.handle.id() == id)
        {
            // Dropping our own handle detaches it; aborting it would cancel cleanup.
            guard.remove(&owned_key);
        }
    });
    guard.insert(
        key,
        RelaySubscriptionTask {
            relay_subscription_id: relay_id,
            client_public_key: public_key,
            // A pending admission is cancellable, but cannot receive display ACKs.
            display_id: None,
            display_pump: None,
            handle,
        },
    );
    let _ = start.send(());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tokio::sync::{oneshot, Mutex};
    use tokio::time::{timeout, Duration};

    fn tasks() -> RelaySubscriptionTasks {
        Arc::new(Mutex::new(BTreeMap::new()))
    }

    #[tokio::test]
    async fn peer_reply_can_arrive_before_display_admission_finishes() {
        let tasks = tasks();
        let (reply, received) = oneshot::channel();
        let (finished, done) = oneshot::channel();
        assert!(timeout(
            Duration::from_millis(100),
            enqueue(&tasks, "relay".into(), "viewer".into(), async move {
                received.await.unwrap();
                finished.send(()).unwrap();
            })
        )
        .await
        .expect("the relay reader must not wait for its peer response"));
        assert_eq!(tasks.lock().await.len(), 1);
        reply.send(()).unwrap();
        timeout(Duration::from_secs(1), done)
            .await
            .unwrap()
            .unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_cancels_pending_admission_before_it_can_publish() {
        let tasks = tasks();
        let (reply, received) = oneshot::channel::<()>();
        let (published, observed) = oneshot::channel();
        assert!(
            enqueue(&tasks, "relay".into(), "viewer".into(), async move {
                let _ = received.await;
                let _ = published.send(());
            })
            .await
        );
        let task = super::super::subscriptions::remove_relay_subscription_task_by_relay_id(
            &tasks, "relay",
        )
        .await
        .unwrap();
        assert!(task.display_id.is_none());
        task.handle.abort();
        let _ = task.handle.await;
        assert!(reply.send(()).is_err());
        assert!(observed.await.is_err());
        assert!(tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn duplicate_admission_does_not_replace_the_pending_owner() {
        let tasks = tasks();
        assert!(
            enqueue(
                &tasks,
                "relay".into(),
                "viewer".into(),
                std::future::pending()
            )
            .await
        );
        assert!(!enqueue(&tasks, "relay".into(), "viewer".into(), async {}).await);
        assert_eq!(tasks.lock().await.len(), 1);
        for (_, task) in std::mem::take(&mut *tasks.lock().await) {
            task.handle.abort();
        }
    }

    #[tokio::test]
    async fn unsubscribe_retires_delivery_during_the_admission_ack() {
        let tasks = tasks();
        let owned = tasks.clone();
        let (registered, ready) = oneshot::channel();
        let (finish, ack) = oneshot::channel::<()>();
        assert!(
            enqueue(&tasks, "relay".into(), "viewer".into(), async move {
                owned.lock().await.insert(
                    "display:id:viewer".into(),
                    RelaySubscriptionTask {
                        relay_subscription_id: "relay".into(),
                        client_public_key: "viewer".into(),
                        display_id: Some("id".into()),
                        display_pump: None,
                        handle: tokio::spawn(tokio::time::sleep(Duration::from_secs(60))),
                    },
                );
                registered.send(()).unwrap();
                ack.await.unwrap();
            })
            .await
        );
        ready.await.unwrap();
        let delivery = super::super::subscriptions::remove_relay_subscription_task_by_relay_id(
            &tasks, "relay",
        )
        .await
        .unwrap();
        assert_eq!(delivery.display_id.as_deref(), Some("id"));
        delivery.handle.abort();
        finish.send(()).unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn pending_admissions_are_bounded_without_polling_refused_work() {
        let tasks = tasks();
        for n in 0..MAX_PENDING {
            assert!(
                enqueue(
                    &tasks,
                    n.to_string(),
                    "viewer".into(),
                    std::future::pending()
                )
                .await
            );
        }
        assert!(
            !enqueue(&tasks, "overflow".into(), "viewer".into(), async {
                panic!("refused work must never execute")
            })
            .await
        );
        assert_eq!(tasks.lock().await.len(), MAX_PENDING);
        for (_, task) in std::mem::take(&mut *tasks.lock().await) {
            task.handle.abort();
        }
    }
}
