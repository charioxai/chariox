//! MP-08/MP-10/MP-11: two existing single-frame credits overlap the peer RTT.
//! The worker still serializes capture. Reply order is not execution order.
use crate::error::DaemonError;
use futures_util::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use std::{collections::BTreeMap, future::Future};

const CAPACITY: usize = 2;
// The producer's 1 MiB egress limit plus encryption/base64 overhead. Two
// pending replies therefore retain at most 4 MiB of ciphertext per viewer.
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct Frame<T> {
    pub sequence: u64,
    pub bytes: usize,
    pub value: T,
}

pub(super) struct CaptureWindow<T> {
    pending: FuturesUnordered<BoxFuture<'static, Result<Option<Frame<T>>, DaemonError>>>,
    ready: BTreeMap<u64, Frame<T>>,
    next_sequence: u64,
    reset: bool,
}

impl<T> Default for CaptureWindow<T> {
    fn default() -> Self {
        Self {
            pending: FuturesUnordered::new(),
            ready: BTreeMap::new(),
            next_sequence: 0,
            reset: true,
        }
    }
}

fn gap() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Room display capture window",
        message: "Room display frame chain requires an independent frame".into(),
    }
}

impl<T: Send + 'static> CaptureWindow<T> {
    pub(super) fn invalidate(&mut self) {
        self.reset = true;
    }

    pub(super) async fn next<F, Fut>(
        &mut self,
        after: u64,
        reset: bool,
        mut capture: F,
    ) -> Result<Option<Frame<T>>, DaemonError>
    where
        F: FnMut(u64, bool) -> Fut,
        Fut: Future<Output = Result<Option<Frame<T>>, DaemonError>> + Send + 'static,
    {
        self.reset |= reset;
        let result = self.advance(after, &mut capture).await;
        if result.is_err() {
            self.reset = true;
        }
        result
    }

    async fn advance<F, Fut>(
        &mut self,
        after: u64,
        capture: &mut F,
    ) -> Result<Option<Frame<T>>, DaemonError>
    where
        F: FnMut(u64, bool) -> Fut,
        Fut: Future<Output = Result<Option<Frame<T>>, DaemonError>> + Send + 'static,
    {
        if self.reset {
            self.ready.clear();
            // Dropping a waiter does not cancel its worker operation. Settle
            // old credits before issuing the single new independent leader.
            while self.pending.next().await.is_some() {}
            let Some(frame) = capture(after, true).await? else {
                return Ok(None);
            };
            if frame.bytes > MAX_FRAME_BYTES {
                return Err(gap());
            }
            self.next_sequence = frame.sequence.checked_add(1).ok_or_else(gap)?;
            self.reset = false;
            return Ok(Some(frame));
        }
        while self.pending.len() + self.ready.len() < CAPACITY {
            self.pending.push(capture(after, false).boxed());
        }
        loop {
            if let Some(frame) = self.ready.remove(&self.next_sequence) {
                self.next_sequence = self.next_sequence.checked_add(1).ok_or_else(gap)?;
                return Ok(Some(frame));
            }
            match self.pending.next().await {
                Some(Ok(Some(frame))) => {
                    if frame.bytes > MAX_FRAME_BYTES
                        || frame.sequence < self.next_sequence
                        || self.ready.contains_key(&frame.sequence)
                    {
                        return Err(gap());
                    }
                    self.ready.insert(frame.sequence, frame);
                }
                Some(Ok(None)) => return Ok(None),
                Some(Err(error)) => return Err(error),
                None if self.ready.is_empty() => return Ok(None),
                None => return Err(gap()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::time::{sleep, Duration};

    fn frame(sequence: u64) -> Frame<()> {
        Frame {
            sequence,
            bytes: 100,
            value: (),
        }
    }

    #[tokio::test]
    async fn room_window_overlaps_two_credits_and_orders_worker_sequences() {
        let mut window = CaptureWindow::default();
        window
            .next(0, false, |_, reset| async move {
                assert!(reset);
                Ok(Some(frame(1)))
            })
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let first = window
            .next(1, false, {
                let calls = calls.clone();
                move |after, reset| {
                    assert_eq!(after, 1);
                    assert!(!reset);
                    let call = calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        // The second request executes first at the worker, but
                        // its later sequence reaches home first over the network.
                        sleep(Duration::from_millis(if call == 0 { 1 } else { 10 })).await;
                        Ok(Some(frame(if call == 0 { 3 } else { 2 })))
                    }
                }
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(first.sequence, 2);
        let next = window
            .next(2, false, |_, _| async { Ok(None) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.sequence, 3);
        assert!(window.pending.len() + window.ready.len() <= CAPACITY);
    }

    #[tokio::test]
    async fn room_window_null_leader_keeps_reset_pending() {
        let mut window = CaptureWindow::<()>::default();
        assert!(window
            .next(0, false, |_, reset| async move {
                assert!(reset);
                Ok(None)
            })
            .await
            .unwrap()
            .is_none());
        window
            .next(0, false, |_, reset| async move {
                assert!(reset);
                Ok(Some(frame(4)))
            })
            .await
            .unwrap();
        assert_eq!(window.next_sequence, 5);
    }

    #[tokio::test]
    async fn room_window_reset_drains_old_worker_operations_before_leader() {
        let mut window = CaptureWindow::default();
        window
            .next(0, false, |_, _| async { Ok(Some(frame(1))) })
            .await
            .unwrap();
        let settled = Arc::new(AtomicUsize::new(0));
        let old = settled.clone();
        window.pending.push(
            async move {
                sleep(Duration::from_millis(10)).await;
                old.fetch_add(1, Ordering::SeqCst);
                Ok(Some(frame(2)))
            }
            .boxed(),
        );
        window.ready.insert(3, frame(3));
        let leader = window
            .next(1, true, move |_, reset| {
                assert!(reset);
                assert_eq!(settled.load(Ordering::SeqCst), 1);
                async { Ok(Some(frame(5))) }
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(leader.sequence, 5);
        assert!(window.pending.is_empty() && window.ready.is_empty());
    }

    #[tokio::test]
    async fn room_window_gap_error_and_oversize_require_independent_recovery() {
        let mut window = CaptureWindow::default();
        window
            .next(0, false, |_, _| async { Ok(Some(frame(1))) })
            .await
            .unwrap();
        assert!(window
            .next(1, false, |_, _| async { Err(gap()) })
            .await
            .is_err());
        assert!(window.reset);
        assert!(window
            .next(1, false, |_, reset| async move {
                assert!(reset);
                Ok(Some(Frame {
                    bytes: MAX_FRAME_BYTES + 1,
                    ..frame(3)
                }))
            })
            .await
            .is_err());
        assert!(window.reset);
    }
}
