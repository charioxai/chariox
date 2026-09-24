//! Host-only response ownership through actual frame publication. A canceled
//! body response cannot silently turn into permission to consume another chunk.
use super::ResponsePublication;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::{sync::watch, time::Instant};
pub(super) struct Guarded {
    pub id: String,
    pub deadline: Instant,
    pub cancellation: watch::Receiver<bool>,
    pub finished: Arc<AtomicBool>,
    pub physically_published: Arc<AtomicBool>,
    pub guard: Box<dyn ResponsePublication>,
    pub completion: Arc<Mutex<()>>,
}
pub(super) struct Pending {
    pub cancel: watch::Sender<bool>,
    pub deadline: Instant,
    pub failure: Option<super::RemoteError>,
    pub completion: Arc<Mutex<()>>,
    pub finished: Arc<AtomicBool>,
    pub physically_published: Arc<AtomicBool>,
}
pub(super) struct Ack {
    pub id: String,
    pub finished: Arc<AtomicBool>,
    pub published: bool,
}
impl Guarded {
    pub fn stopped(&self) -> bool {
        *self.cancellation.borrow()
            || self.cancellation.has_changed().is_err()
            || Instant::now() >= self.deadline
    }
    pub fn complete(mut self, published: bool) -> Ack {
        let _completion = self
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A fully written reply cannot be replaced with another response. But
        // cancellation that wins before this boundary still poisons the body,
        // even if its success frame reached the peer immediately beforehand.
        if published && !self.stopped() {
            self.guard.published();
        }
        // The callback releases the body-operation gate before the actor ACK;
        // an already received success never waits on actor bookkeeping.
        self.physically_published
            .store(published, Ordering::Relaxed);
        self.finished.store(true, Ordering::Release);
        Ack {
            id: self.id,
            finished: self.finished,
            published,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Observed(Arc<AtomicBool>, Arc<AtomicBool>);
    impl ResponsePublication for Observed {
        fn published(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    impl Drop for Observed {
        fn drop(&mut self) {
            self.1.store(true, Ordering::Release);
        }
    }
    #[test]
    fn completed_frame_cancellation_preserves_physical_ack_and_discards_body() {
        for physical in [true, false] {
            let (cancel, cancellation) = watch::channel(false);
            let published = Arc::new(AtomicBool::new(false));
            let dropped = Arc::new(AtomicBool::new(false));
            let finished = Arc::new(AtomicBool::new(false));
            let physically_published = Arc::new(AtomicBool::new(false));
            let guarded = Guarded {
                id: "body".into(),
                deadline: Instant::now() + std::time::Duration::from_secs(1),
                cancellation,
                finished: finished.clone(),
                physically_published: physically_published.clone(),
                guard: Box::new(Observed(published.clone(), dropped.clone())),
                completion: Arc::new(Mutex::new(())),
            };
            // Precisely model cancellation winning after a completed write
            // and before the callback: cleanup cannot rewrite physical fact.
            cancel.send_replace(true);
            let ack = guarded.complete(physical);
            assert_eq!(ack.published, physical);
            assert_eq!(physically_published.load(Ordering::Acquire), physical);
            assert!(finished.load(Ordering::Acquire));
            assert!(!published.load(Ordering::Acquire));
            assert!(dropped.load(Ordering::Acquire));
        }
    }
}
