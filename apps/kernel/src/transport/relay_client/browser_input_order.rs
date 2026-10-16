//! MP-08/MP-10/MP-11: reserve browser input order before concurrent task polling.
//! This is scheduling only; the normal kernel caller/document admission remains.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

const LIMIT: usize = 128;
#[derive(Default)]
pub(super) struct BrowserInputOrder(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    tails: HashMap<String, (u64, oneshot::Receiver<()>)>,
    next: u64,
    pending: usize,
}
impl std::fmt::Debug for BrowserInputOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserInputOrder").finish_non_exhaustive()
    }
}
pub(super) struct Turn {
    order: Arc<Mutex<State>>,
    key: String,
    id: u64,
    previous: Option<oneshot::Receiver<()>>,
    done: Option<oneshot::Sender<()>>,
}
impl BrowserInputOrder {
    pub(super) fn reserve(&self, key: String) -> Option<Turn> {
        let mut state = self.0.lock().expect("browser input order poisoned");
        if state.pending >= LIMIT {
            return None;
        }
        state.next = state.next.wrapping_add(1);
        let id = state.next;
        let (done, ready) = oneshot::channel();
        let previous = state
            .tails
            .insert(key.clone(), (id, ready))
            .map(|(_, ready)| ready);
        state.pending += 1;
        Some(Turn {
            order: self.0.clone(),
            key,
            id,
            previous,
            done: Some(done),
        })
    }
}
impl Turn {
    pub(super) async fn ready(&mut self) {
        if let Some(previous) = self.previous.as_mut() {
            let _ = previous.await;
        }
        self.previous = None;
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        let order = self.order.clone();
        let key = std::mem::take(&mut self.key);
        let id = self.id;
        let done = self.done.take();
        let finish = move || {
            let mut state = order.lock().expect("browser input order poisoned");
            state.pending -= 1;
            if state.tails.get(&key).is_some_and(|(tail, _)| *tail == id) {
                state.tails.remove(&key);
            }
            drop(done);
        };
        // Cancelling a queued turn cannot release its successor ahead of the
        // still-active predecessor. Keep that chain and its capacity accounted.
        if let Some(previous) = self.previous.take() {
            tokio::spawn(async move {
                let _ = previous.await;
                finish();
            });
        } else {
            finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};
    #[tokio::test]
    async fn mp08_reservation_order_survives_reverse_task_polling() {
        let order = BrowserInputOrder::default();
        let mut first = order.reserve("tab".into()).unwrap();
        let mut second = order.reserve("tab".into()).unwrap();
        let mut third = order.reserve("tab".into()).unwrap();
        assert!(timeout(Duration::from_millis(5), third.ready())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(5), second.ready())
            .await
            .is_err());
        first.ready().await;
        drop(first);
        second.ready().await;
        assert!(timeout(Duration::from_millis(5), third.ready())
            .await
            .is_err());
        drop(second);
        third.ready().await;
        drop(third);
        assert_eq!(order.0.lock().unwrap().pending, 0);
        assert!(order.0.lock().unwrap().tails.is_empty());
    }
    #[tokio::test]
    async fn mp11_cancelled_queued_turn_cannot_skip_an_active_predecessor() {
        let order = BrowserInputOrder::default();
        let mut first = order.reserve("tab".into()).unwrap();
        first.ready().await;
        let second = order.reserve("tab".into()).unwrap();
        let mut third = order.reserve("tab".into()).unwrap();
        drop(second);
        assert!(timeout(Duration::from_millis(5), third.ready())
            .await
            .is_err());
        assert_eq!(order.0.lock().unwrap().pending, 3);
        drop(first);
        third.ready().await;
        drop(third);
        assert_eq!(order.0.lock().unwrap().pending, 0);
    }
    #[tokio::test]
    async fn mp08_other_tabs_and_noninput_work_stay_independent() {
        let order = BrowserInputOrder::default();
        let held = order.reserve("first".into()).unwrap();
        let mut other = order.reserve("second".into()).unwrap();
        timeout(Duration::from_millis(5), other.ready())
            .await
            .unwrap();
        drop(held);
        drop(other);
    }
    #[tokio::test]
    async fn mp11_bounded_queue_releases_capacity_after_settlement() {
        let order = BrowserInputOrder::default();
        let mut turns = (0..LIMIT)
            .map(|i| order.reserve(i.to_string()).unwrap())
            .collect::<Vec<_>>();
        assert!(order.reserve("overload".into()).is_none());
        for turn in &mut turns {
            turn.ready().await;
        }
        drop(turns);
        assert!(order.reserve("fresh".into()).is_some());
    }
}
