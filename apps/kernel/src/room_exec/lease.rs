//! A reader must keep polling output; listing or writing cannot keep an orphan alive.
use std::sync::Weak;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use super::{Inner, RoomExecService};

// Longer than the maximum four-minute read wait, including its transport margin.
pub(super) const READER_LEASE: Duration = Duration::from_secs(300);

pub(super) fn start(exec_id: String, inner: Weak<Inner>, mut deadline: watch::Receiver<Instant>) {
    tokio::spawn(async move {
        loop {
            let expires = *deadline.borrow_and_update();
            tokio::select! {
                changed = deadline.changed() => {
                    if changed.is_err() { return; }
                }
                _ = tokio::time::sleep_until(expires) => {
                    let Some(inner) = inner.upgrade() else { return; };
                    let mut records = inner.records.lock().expect("exec records poisoned");
                    let expired = records.get(&exec_id).is_some_and(|record| {
                        *record.reader_deadline.borrow() <= Instant::now()
                    });
                    if expired {
                        let record = records.remove(&exec_id).expect("expired record exists");
                        record.cancel.send_replace(true);
                        drop(records);
                        // The process task releases managed work only after group
                        // cleanup, reaping and pipe settlement have completed.
                        RoomExecService { inner }.report_work();
                        return;
                    }
                    if !records.contains_key(&exec_id) { return; }
                }
            }
        }
    });
}
