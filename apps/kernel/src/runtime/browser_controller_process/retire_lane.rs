// A parked viewer input outlives the request that parked it, so its authority has to end even while another
// operation holds the backend (its request/response lock). The retirement is written straight to the controller's
// stdin, which has its own lock; the controller handles it without waiting behind any browser operation.
use std::process::ChildStdin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::request_wire;

// Ids no ordinary request reaches; the reply is ignored by whichever request is waiting.
const LANE_ID_BASE: u64 = 1 << 52;
static NEXT: AtomicU64 = AtomicU64::new(LANE_ID_BASE);

#[derive(Clone, Default)]
pub(crate) struct RetireLane(Arc<Mutex<Weak<Mutex<ChildStdin>>>>);

impl RetireLane {
    pub(super) fn attach(&self, stdin: &Arc<Mutex<ChildStdin>>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(stdin);
    }

    pub(crate) fn retire_observer(&self, observer: &str) -> bool {
        let stdin = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade();
        let Some(stdin) = stdin else { return false };
        let Ok(mut stdin) = stdin.lock() else {
            return false;
        };
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        request_wire::write_line(
            &mut *stdin,
            &serde_json::json!({"id":id,"method":"host.browser.retire_observer","params":{"observer":observer}}),
        )
        .is_ok()
    }
}
