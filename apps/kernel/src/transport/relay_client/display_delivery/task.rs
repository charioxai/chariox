//! MP-08/MP-10/MP-11: frame cancellation is independent of the delivery TTL timer.
use std::{future::Future, sync::Mutex};
use tokio::task::AbortHandle;

#[derive(Default)]
pub(crate) struct DisplayTask(Mutex<Option<AbortHandle>>);

impl DisplayTask {
    pub(crate) fn start(&self, delivery: impl Future<Output = ()> + Send + 'static) {
        let mut task = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(old) = task.take() {
            old.abort();
        }
        *task = Some(tokio::spawn(delivery).abort_handle());
    }

    pub(crate) fn stop(&self) {
        if let Some(task) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        sync::oneshot,
        time::{timeout, Duration},
    };

    struct Capture(Option<oneshot::Sender<()>>);
    impl Drop for Capture {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    #[tokio::test]
    async fn ttl_renewal_does_not_cancel_capture_but_stop_does() {
        let delivery = DisplayTask::default();
        let (began, started) = oneshot::channel();
        let (released, mut dropped) = oneshot::channel();
        let capture = Capture(Some(released));
        delivery.start(async move {
            let _capture = capture;
            let _ = began.send(());
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        let mut timer = tokio::spawn(std::future::pending::<()>());
        for _ in 0..3 {
            timer.abort();
            timer = tokio::spawn(std::future::pending::<()>());
            tokio::task::yield_now().await;
            assert!(matches!(
                dropped.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
        }
        delivery.stop();
        timeout(Duration::from_secs(1), dropped)
            .await
            .unwrap()
            .unwrap();
        timer.abort();
    }
}
